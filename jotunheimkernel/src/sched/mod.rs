#![allow(clippy::missing_safety_doc)]

pub mod sched_simd;

use core::cell::UnsafeCell;
use core::fmt::DebugMap;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::{Mutex, Once};
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;

use crate::arch::x86_64::context;
use crate::arch::x86_64::context::CpuContext;
use crate::arch::x86_64::simd;
use crate::kprintln;
use crate::sched::sched_simd::SimdArea;

/* ------------------------------- Types & consts ------------------------------- */

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Dead,
    Void,
}

unsafe extern "C" {
    // Must not return: pops (arg, entry) prepared by init/spawn,
    // calls entry(arg), and on return jumps to sched_exit_current_trampoline.
    fn kthread_trampoline();
}

pub type TaskId = u64;

#[derive(Clone)]
pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub simd: SimdArea,
    pub kstack_top: u64,
    pub time_slice: u32,
}

impl Copy for Task {}

const MAX_TASKS: usize = 16;
pub const DEFAULT_SLICE: u32 = 2; // 5ms at 1 kHz
const IDLE_STACK_SIZE: usize = 16 * 1024;

/* ----------------------------- Runqueue container ----------------------------- */

pub struct RunQueue {
    tasks: [Task; MAX_TASKS],
    current: usize,
    next_id: TaskId,
    need_resched: bool,
}

/* ---------------------- Static storage (no allocator) ------------------------- */

// Backing storage for the tasks array. We initialize it exactly once under RQ_CELL.call_once().
struct TaskBuf(UnsafeCell<MaybeUninit<[Option<Task>; MAX_TASKS]>>);
// SAFETY: We only write to the buffer inside Once::call_once() and then expose a single
// &'static mut [Option<Task>] slice. No aliasing mutable references are created afterwards.
unsafe impl Sync for TaskBuf {}
static RQ: Mutex<RunQueue> = Mutex::new(RunQueue {
    tasks: [Task {
        id: 0,
        ctx: CpuContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rbx: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,
            rsp: 0,
            rip: 0,
            rflags: 0,
        },
        simd: SimdArea {
            dump: [0; sched_simd::SIZE],
        },
        state: TaskState::Void,
        kstack_top: 0,
        time_slice: 0,
    }; MAX_TASKS],
    current: 0,
    next_id: 0,
    need_resched: false,
});
static mut IDLE_STACK: [u8; IDLE_STACK_SIZE] = [0; IDLE_STACK_SIZE];

impl RunQueue {
    fn pick_next(&self) -> Option<usize> {
        let current = self.current;
        // Simple round-robin over READY tasks; skip slot 0 unless no other READY
        for off in 1..self.tasks.len() {
            let i = (current + off) % self.tasks.len();
            if i == 0 {
                continue;
            }
            match &self.tasks[i] {
                t => {
                    if matches!(t.state, TaskState::Ready) {
                        return Some(i);
                    }
                }
                _ => (),
            }
        }
        if let Some(t0) = self.tasks.get(0) {
            if matches!(t0.state, TaskState::Ready | TaskState::Running) {
                return Some(0);
            }
        }
        None
    }
}

/* --------------------------------- Utilities --------------------------------- */

#[unsafe(no_mangle)]
pub extern "C" fn sched_exit_current_trampoline() -> ! {
    exit_current()
}

extern "C" fn idle_main(_arg: usize) -> ! {
    loop {
        yield_now();
    }
}

/* --------------------------------- Init path --------------------------------- */

pub fn init() {
    static ONCE: spin::Once<()> = spin::Once::new();
    ONCE.call_once(|| {
        with_rq(|rq| {
            let base = core::ptr::addr_of_mut!(IDLE_STACK) as *mut u8;
            let top = ((base as usize + IDLE_STACK_SIZE) & !0xF) as u64;
            let init_rsp = (top - 16) as *mut u64;

            unsafe {
                core::ptr::write(init_rsp.add(0), 0u64);
                core::ptr::write(init_rsp.add(1), idle_main as u64);
            }

            let context = CpuContext {
                rip: kthread_trampoline as u64,
                rsp: init_rsp as u64,
                ..CpuContext::default()
            };

            kprintln!("X");
            kprintln!("Y");
            rq.tasks[0] = Task {
                id: rq.next_id,
                state: TaskState::Running,
                ctx: context,
                kstack_top: top,
                simd: SimdArea {
                    dump: [0; sched_simd::SIZE],
                },
                time_slice: u32::MAX,
            };
            kprintln!("Z");
            kprintln!("W");

            rq.next_id += 1;
            rq.current = 0;
        })
    });
}

/* ------------------------------- Public API ---------------------------------- */

pub fn spawn_kthread(
    entry: extern "C" fn(usize),
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    // Prepare trampoline stack frame [arg][entry]
    let top = ((stack_ptr as usize + stack_len) & !0xF) as u64;
    let init_rsp = (top - 16) as *mut u64;
    unsafe {
        core::ptr::write(init_rsp.add(0), arg as u64);
        core::ptr::write(init_rsp.add(1), entry as u64);
    }

    let ctx = CpuContext {
        rip: kthread_trampoline as u64,
        rsp: init_rsp as u64,
        rflags: 0x202,
        ..CpuContext::default()
    };

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;

        let idx = rq
            .tasks
            .iter()
            .position(|t| t.state == TaskState::Void)
            .expect("no slots");
        rq.tasks[idx] = Task {
            id,
            state: TaskState::Ready,
            ctx,
            simd: sched_simd::SimdArea::default(),
            kstack_top: top,
            time_slice: DEFAULT_SLICE,
        };
        id
    })
}

#[repr(C)]
pub struct PreemptPack {
    pub prev_ctx: *mut CpuContext,
    pub next_ctx: *const CpuContext,
    pub prev_simd: *mut u8, // null if none
    pub next_simd: *mut u8, // null if none
}

static mut PREEMPT_PACK: PreemptPack = PreemptPack {
    prev_ctx: core::ptr::null_mut(),
    next_ctx: core::ptr::null(),
    prev_simd: core::ptr::null_mut(),
    next_simd: core::ptr::null_mut(),
};

static DEFER_RESCHED: AtomicBool = AtomicBool::new(false);

pub fn tick() -> *const PreemptPack {
    with_rq_locked(|rq| {
        if let current = rq.current {
            let t = &mut rq.tasks[current];
            if t.time_slice != u32::MAX && t.time_slice > 0 {
                t.time_slice -= 1;
                if t.time_slice == 0 {
                    t.state = TaskState::Ready;
                    t.time_slice = DEFAULT_SLICE;
                    rq.need_resched = true;
                }
            }
        }
    });
    let current = with_rq_locked(|rq| rq.current);

    let cur_is_idle = with_rq_locked(|rq| rq.tasks[current].time_slice == u32::MAX);

    let some_ready = with_rq_locked(|rq| {
        rq.tasks
            .iter()
            .enumerate()
            .any(|(i, t)| i != current && t.state == TaskState::Ready)
    });

    if !(with_rq_locked(|rq| rq.need_resched) || (cur_is_idle && some_ready)) {
        return core::ptr::null();
    }

    let Some(next) = with_rq_locked(|rq| rq.pick_next()) else {
        with_rq_locked(|rq| rq.need_resched = false);
        return core::ptr::null();
    };

    if next == current {
        with_rq_locked(|rq| rq.need_resched = false);
        return core::ptr::null();
    }
    with_rq_locked(|rq| {
        let t = &mut rq.tasks[current];
        if t.time_slice != u32::MAX {
            t.state = TaskState::Ready;
            t.time_slice = DEFAULT_SLICE;
        }
    });
    with_rq_locked(|rq| {
        rq.tasks[next].state = TaskState::Running;
        let (prev_ctx, prev_simd) = {
            let prev = &mut rq.tasks[current];
            (&mut prev.ctx as *mut CpuContext, prev.simd.as_mut_ptr())
        };
        let (next_ctx, next_simd) = {
            let next = rq.tasks[next];
            (&next.ctx as *const CpuContext, next.simd.as_mut_ptr())
        };

        rq.current = next;
        rq.need_resched = false;

        unsafe {
            PREEMPT_PACK.prev_ctx = prev_ctx;
            PREEMPT_PACK.next_ctx = next_ctx;
            PREEMPT_PACK.prev_simd = prev_simd;
            PREEMPT_PACK.next_simd = next_simd;
            &raw const PREEMPT_PACK
        }
    })
}
/* ------------------------------ Core switching ------------------------------- */

pub fn yield_now() {
    let Some((prev_ctx, next_ctx, prev_simd_ptr, next_simd_ptr)) = with_rq_locked(|rq| {
        let Some(next) = rq.pick_next() else {
            return None;
        };
        if next == rq.current {
            rq.need_resched = false;
            return None;
        }
        let t = &mut rq.tasks[rq.current];
        t.state = TaskState::Ready;
        t.time_slice = DEFAULT_SLICE;
        t.state = TaskState::Running;

        let (prev_ctx, prev_simd_ptr) = {
            let prev = &mut rq.tasks[rq.current];
            (&mut prev.ctx as *mut CpuContext, prev.simd.as_mut_ptr())
        };
        let (next_ctx, next_simd_ptr) = {
            let next = rq.tasks[next];
            (&next.ctx as *const CpuContext, next.simd.as_mut_ptr())
        };

        rq.current = next;
        rq.need_resched = false;

        Some((prev_ctx, next_ctx, prev_simd_ptr, next_simd_ptr))
    }) else {
        return;
    };
    simd::save(prev_simd_ptr);
    context::switch(prev_ctx, next_ctx);
    simd::restore(next_simd_ptr);
}

pub fn exit_current() -> ! {
    let (prev_ctx, next_ctx) = with_rq_locked(|rq| {
        let cur = rq.current;
        rq.tasks[cur].state = TaskState::Dead;

        let Some(next_idx) = rq.pick_next() else {
            loop {
                x86_64::instructions::hlt();
            }
        };
        if next_idx == cur {
            loop {
                x86_64::instructions::hlt();
            }
        }

        rq.tasks[next_idx].state = TaskState::Running;

        let prev_ctx = &mut rq.tasks[cur].ctx as *mut _;
        let next_ctx = &rq.tasks[next_idx].ctx as *const _;
        rq.current = next_idx;

        (prev_ctx, next_ctx)
    });
    context::switch(prev_ctx, next_ctx);
    loop {
        x86_64::instructions::hlt();
    }
}

/* ------------------------------- Helper wrapper ------------------------------ */

fn with_rq_locked<F, R>(f: F) -> R
where
    F: FnOnce(&mut RunQueue) -> R,
{
    without_interrupts(|| with_rq(f))
}

fn with_rq<F, R>(f: F) -> R
where
    F: FnOnce(&mut RunQueue) -> R,
{
    let mut guard = RQ.lock();
    let rq: &mut RunQueue = &mut *guard;
    f(rq)
}
