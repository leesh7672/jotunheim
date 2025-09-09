#![allow(clippy::missing_safety_doc)]

use core::mem::size_of;
use core::sync::atomic::{AtomicU8, Ordering};
use spin::{Mutex, Once};

use crate::arch::x86_64::context;
use crate::arch::x86_64::context::CpuContext;
use crate::println;
use x86_64::instructions::{hlt, interrupts};
pub type TaskId = u64;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Dead,
}

unsafe extern "C" {
    fn kthread_trampoline();
}

/* ------------------------- Preemption handshake exports ------------------------- */

/// Set by Rust when a timer tick decides to preempt. Read by the ISR stub.
/// Nonzero means “preempt now”.
#[unsafe(no_mangle)]
pub static __SCHED_PREEMPT_REQUESTED: AtomicU8 = AtomicU8::new(0);

/// Published by Rust before setting the flag; consumed by ISR stub.
/// Raw pointer to current task context (to be saved into).
#[unsafe(no_mangle)]
pub static mut __SCHED_PREEMPT_PREV: *mut CpuContext = core::ptr::null_mut();

/// Raw pointer to next task context (to be loaded from).
#[unsafe(no_mangle)]
pub static mut __SCHED_PREEMPT_NEXT: *const CpuContext = core::ptr::null();

/* --------------------------------- Task model ---------------------------------- */

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub kstack_top: u64,
    pub time_slice: u32, // ticks remaining
}

const MAX_TASKS: usize = 128;
const DEFAULT_SLICE: u32 = 5; // 5ms @ 1kHz

struct RunQueue {
    tasks: [Option<Task>; MAX_TASKS],
    current: Option<usize>, // index into tasks
    next_id: TaskId,
    rr_cursor: usize, // where round-robin search resumes
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_exit_current_trampoline() -> ! {
    exit_current()
}

static RQ_ONCE: Once<Mutex<RunQueue>> = Once::new();

#[inline]
fn rq() -> &'static Mutex<RunQueue> {
    RQ_ONCE.call_once(|| {
        let tasks: [Option<Task>; MAX_TASKS] = core::array::from_fn(|_| None);
        Mutex::new(RunQueue {
            tasks,
            current: None,
            next_id: 1,
            rr_cursor: 0,
        })
    })
}

/* ---------------------------------- Public API --------------------------------- */

pub fn init() {
    static ONCE: Once<()> = Once::new();
    ONCE.call_once(|| {
        // Slot 0 = idle, already "Running"
        let mut g = rq().lock();
        g.tasks[0] = Some(Task {
            id: g.next_id,
            state: TaskState::Running,
            ctx: CpuContext::default(),
            kstack_top: 0,
            time_slice: u32::MAX, // never preempt idle
        });
        g.next_id += 1;
        g.current = Some(0);
        g.rr_cursor = 1;
    });
}

/// Create a kernel thread that starts at `entry(arg)` using the provided stack region.
/// `stack_ptr..stack_ptr+stack_len` must be valid & writable.
pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    let top = ((stack_ptr as usize + stack_len) & !0xF) as u64;

    // Prepare the initial stack so the trampoline pops arg then entry RIP.
    let init_rsp = (top - 16) as *mut u64;
    unsafe {
        core::ptr::write(init_rsp.add(0), arg as u64);
        core::ptr::write(init_rsp.add(1), entry as u64);
    }

    let ctx = CpuContext {
        rip: kthread_trampoline as u64,
        rsp: init_rsp as u64,
        ..CpuContext::default()
    };

    interrupts::without_interrupts(|| {
        let mut g = rq().lock();
        let id = g.next_id;
        g.next_id += 1;

        let idx = g
            .tasks
            .iter()
            .position(|t| t.is_none())
            .expect("runqueue full");
        g.tasks[idx] = Some(Task {
            id,
            state: TaskState::Ready,
            ctx,
            kstack_top: top,
            time_slice: DEFAULT_SLICE,
        });
        id
    })
}
pub fn tick() {
    interrupts::without_interrupts(|| {
        let mut g = rq().lock();
        let Some(cur) = g.current else { return };

        let mut want_preempt = false;
        let mut prev_ptr: *mut CpuContext = core::ptr::null_mut();
        let mut next_ptr: *const CpuContext = core::ptr::null();

        {
            let t = g.tasks[cur].as_mut().unwrap();
            if t.time_slice != u32::MAX {
                if t.time_slice > 0 {
                    t.time_slice -= 1;
                }
                if t.time_slice == 0 {
                    prev_ptr = core::ptr::addr_of_mut!(t.ctx);
                    t.time_slice = DEFAULT_SLICE;
                    want_preempt = true; // defer the rest
                }
            }
        } // <-- mutable borrow ends here

        if want_preempt {
            if let Some(next_idx) = pick_next_locked(&g, cur) {
                next_ptr = core::ptr::addr_of!(g.tasks[next_idx].as_ref().unwrap().ctx);
                unsafe {
                    __SCHED_PREEMPT_PREV = prev_ptr;
                    __SCHED_PREEMPT_NEXT = next_ptr;
                }
                __SCHED_PREEMPT_REQUESTED.store(1, Ordering::Release);
            }
        }
    });
}

#[repr(C)]
pub struct SwitchPtrs {
    pub prev: *mut CpuContext,
    pub next: *const CpuContext,
}
#[unsafe(no_mangle)]
pub extern "C" fn sched_preempt_handle_from_isr() {
    // Take & clear the flag (paired with Release in tick()).
    if __SCHED_PREEMPT_REQUESTED.swap(0, Ordering::AcqRel) == 0 {
        return; // nothing to do -> let the stub iret
    }

    let (prev_ptr, next_ptr) = unsafe { (__SCHED_PREEMPT_PREV, __SCHED_PREEMPT_NEXT) };
    if prev_ptr.is_null() || next_ptr.is_null() {
        return; // pointers not ready -> bail
    }

    // Update runqueue states atomically wrt other cores/paths.
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut g = rq().lock();

        // Resolve indices by pointer equality (OK for small MAX_TASKS).
        let mut cur_idx = None;
        let mut nxt_idx = None;
        for (i, slot) in g.tasks.iter().enumerate() {
            if let Some(t) = slot {
                let p = core::ptr::addr_of!(t.ctx);
                if p as *const _ == next_ptr {
                    nxt_idx = Some(i);
                }
                if p as *const _ == prev_ptr.cast_const() {
                    cur_idx = Some(i);
                }
            }
        }

        if let (Some(c), Some(n)) = (cur_idx, nxt_idx) {
            if c != 0 {
                if let Some(tc) = g.tasks[c].as_mut() {
                    tc.state = TaskState::Ready;
                    tc.time_slice = DEFAULT_SLICE;
                }
            }
            if let Some(tn) = g.tasks[n].as_mut() {
                tn.state = TaskState::Running;
            }
            g.current = Some(n);
            g.rr_cursor = (n + 1) % g.tasks.len();
        }
    });

    // Do not return: jump into next context.
    context::switch(prev_ptr, next_ptr);
    // unreachable
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_preempt_take(out: *mut SwitchPtrs) -> u8 {
    // Atomically take & clear the flag; Release in tick(), Acquire here.
    if __SCHED_PREEMPT_REQUESTED.swap(0, core::sync::atomic::Ordering::Acquire) == 0 {
        return 0;
    }

    // Publish the pair. (These were set before the flag with Release.)
    unsafe {
        (*out).prev = __SCHED_PREEMPT_PREV;
        (*out).next = __SCHED_PREEMPT_NEXT;
    }

    1
}

pub fn yield_now() {
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        // Compute everything we need while holding the lock,
        // but only *store pointers/indices*. Then drop the lock.
        let (prev_ptr, next_ptr, cur_idx, next_idx): (*mut CpuContext, *const CpuContext, _, _) = {
            let mut g = rq().lock();
            let cur = g.current.expect("no current");

            let Some(next_idx) = pick_next_locked(&g, cur) else {
                return;
            };

            // state bookkeeping
            if cur != 0 {
                let cur_task = g.tasks[cur].as_mut().unwrap();
                cur_task.state = TaskState::Ready;
                cur_task.time_slice = DEFAULT_SLICE;
            }
            g.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;

            // round-robin cursor + current
            g.current = Some(next_idx);
            g.rr_cursor = (next_idx + 1) % g.tasks.len();

            // raw context pointers (don’t deref while holding the lock)
            let prev_ctx = &mut g.tasks[cur].as_mut().unwrap().ctx as *mut _;
            let next_ctx: *const _ = &g.tasks[next_idx].as_ref().unwrap().ctx as *const _;
            (prev_ctx, next_ctx, cur, next_idx)
        };

        // Snapshot next context registers for logging (unsafe deref of raw ptr)
        let (next_rsp, next_rip) = unsafe { ((*next_ptr).rsp, (*next_ptr).rip) };

        crate::println!(
            "[SCHED] switch {} -> {} rsp=0x{:016x} rip=0x{:016x}",
            cur_idx,
            next_idx,
            next_rsp,
            next_rip
        );

        // Do the actual switch
        context::switch(prev_ptr, next_ptr)
    });
}

/// Exit the current thread (never returns).
pub fn exit_current() -> ! {
    interrupts::without_interrupts(|| {
        let (prev_ptr, next_ptr): (*mut CpuContext, *const CpuContext) = {
            let mut g = rq().lock();
            let cur = g.current.expect("no current");

            // kill current
            g.tasks[cur].as_mut().unwrap().state = TaskState::Dead;

            let Some(next_idx) = pick_next_locked(&g, cur) else {
                drop(g);
                loop {
                    hlt();
                }
            };

            g.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;
            g.current = Some(next_idx);
            g.rr_cursor = (next_idx + 1) % g.tasks.len();

            let prev_ctx = &mut g.tasks[cur].as_mut().unwrap().ctx as *mut _;
            let next_ctx = &g.tasks[next_idx].as_ref().unwrap().ctx as *const _;
            (prev_ctx, next_ctx)
        };

        context::switch(prev_ptr, next_ptr);
        unreachable!();
    })
}
pub fn preempt_point() {
    use core::sync::atomic::Ordering;

    // Quick path: anything to do?
    if __SCHED_PREEMPT_REQUESTED.load(Ordering::Acquire) == 0 {
        return;
    }

    // Take the request exactly once and snapshot the two pointers.
    let (prev_ptr, next_ptr) = x86_64::instructions::interrupts::without_interrupts(|| {
        if __SCHED_PREEMPT_REQUESTED.swap(0, Ordering::AcqRel) == 0 {
            return (core::ptr::null_mut(), core::ptr::null());
        }
        unsafe { (__SCHED_PREEMPT_PREV, __SCHED_PREEMPT_NEXT) }
    });

    if prev_ptr.is_null() || next_ptr.is_null() {
        return;
    }

    // Minimal runqueue bookkeeping with the lock held.
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut g = rq().lock();

        // Find indices by pointer equality.
        let mut cur_idx: Option<usize> = None;
        let mut nxt_idx: Option<usize> = None;
        for (i, opt) in g.tasks.iter().enumerate() {
            if let Some(t) = opt {
                let p = core::ptr::addr_of!(t.ctx) as *const CpuContext;
                if p == next_ptr {
                    nxt_idx = Some(i);
                }
                if p == prev_ptr.cast_const() {
                    cur_idx = Some(i);
                }
            }
        }

        if let (Some(c), Some(n)) = (cur_idx, nxt_idx) {
            if c != 0 {
                if let Some(tc) = g.tasks[c].as_mut() {
                    tc.state = TaskState::Ready;
                    tc.time_slice = DEFAULT_SLICE;
                }
            }
            if let Some(tn) = g.tasks[n].as_mut() {
                tn.state = TaskState::Running;
            }
            g.current = Some(n);
            g.rr_cursor = (n + 1) % g.tasks.len();
        }
        // lock drops here
    });

    // Do the actual context switch (no locks held, interrupts disabled by caller).
    context::switch(prev_ptr, next_ptr);
}

/* ------------------------------ Scheduling utils ------------------------------ */

/// Round-robin: find next runnable after `cur`, prefer non-idle; fall back to idle if needed.
/// Requires the caller to hold the runqueue lock.
fn pick_next_locked(g: &RunQueue, cur: usize) -> Option<usize> {
    let n = g.tasks.len();
    let start = if g.rr_cursor == 0 { 1 } else { g.rr_cursor }; // skip idle as start

    // First pass: non-idle only
    for off in 0..n {
        let i = (start + off) % n;
        if i == 0 || i == cur {
            continue;
        }
        if let Some(t) = &g.tasks[i] {
            if t.state == TaskState::Ready {
                return Some(i);
            }
        }
    }
    // Fallback: allow idle
    if let Some(Some(t0)) = g.tasks.get(0) {
        if matches!(t0.state, TaskState::Ready | TaskState::Running) {
            return Some(0);
        }
    }
    None
}

/* ------------------------------ Debug / sanity -------------------------------- */

pub fn ctx_layout_sanity() {
    // Make sure the asm offsets match.
    crate::println!("[CTX] size = {:#x}", size_of::<CpuContext>());
    let s = CpuContext::default();
    let p = &s as *const CpuContext;
    unsafe {
        crate::println!(
            "[CTX] off r15={:#x}",
            (&(*p).r15 as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off r14={:#x}",
            (&(*p).r14 as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off r13={:#x}",
            (&(*p).r13 as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off r12={:#x}",
            (&(*p).r12 as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off rbx={:#x}",
            (&(*p).rbx as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off rbp={:#x}",
            (&(*p).rbp as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off rsp={:#x}",
            (&(*p).rsp as *const _ as usize) - p as usize
        );
        crate::println!(
            "[CTX] off rip={:#x}",
            (&(*p).rip as *const _ as usize) - p as usize
        );
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn sched_preempt_bookkeeping() {
    interrupts::without_interrupts(|| {
        let mut g = rq().lock();
        // find indices by pointer equality
        let mut cur = None;
        let mut nxt = None;
        for (i, t) in g.tasks.iter().enumerate() {
            if let Some(t) = t {
                let p = core::ptr::addr_of!(t.ctx);
                if p as *const CpuContext == unsafe { __SCHED_PREEMPT_NEXT } {
                    nxt = Some(i);
                }
                if p as *const CpuContext == unsafe { __SCHED_PREEMPT_PREV } {
                    cur = Some(i);
                }
            }
        }
        if let (Some(c), Some(n)) = (cur, nxt) {
            if c != 0 {
                if let Some(tc) = g.tasks[c].as_mut() {
                    tc.state = TaskState::Ready;
                    tc.time_slice = DEFAULT_SLICE;
                }
            }
            if let Some(tn) = g.tasks[n].as_mut() {
                tn.state = TaskState::Running;
            }
            g.current = Some(n);
            g.rr_cursor = (n + 1) % g.tasks.len();
        }
    });
}
