#![allow(clippy::missing_safety_doc)]

pub mod sched_simd;

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

use spin::{Mutex, Once};
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;
use alloc::boxed::Box;

use crate::arch::x86_64::context;
use crate::arch::x86_64::context::CpuContext;
use crate::arch::x86_64::simd;

/* ------------------------------- Types & consts ------------------------------- */

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Dead,
}

unsafe extern "C" {
    // Must not return: pops (arg, entry) prepared by init/spawn,
    // calls entry(arg), and on return jumps to sched_exit_current_trampoline.
    fn kthread_trampoline();
}

pub type TaskId = u64;

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub simd: Option<sched_simd::SimdArea>,
    pub kstack_top: u64,
    pub time_slice: u32,
}

const MAX_TASKS: usize = 192;
const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz
const IDLE_STACK_SIZE: usize = 16 * 1024;

/* ----------------------------- Runqueue container ----------------------------- */

pub struct RunQueue {
    // Static-backed slice to avoid early-boot heap allocations.
    tasks: &'static mut [Option<Task>],
    current: Option<usize>,
    next_id: TaskId,
    need_resched: bool,
}

/* ---------------------- Static storage (no allocator) ------------------------- */

// Backing storage for the tasks array. We initialize it exactly once under RQ_CELL.call_once().
struct TaskBuf(UnsafeCell<MaybeUninit<[Option<Task>; MAX_TASKS]>>);
// SAFETY: We only write to the buffer inside Once::call_once() and then expose a single
// &'static mut [Option<Task>] slice. No aliasing mutable references are created afterwards.
unsafe impl Sync for TaskBuf {}

static RQ_TASKS_BUF: TaskBuf = TaskBuf(UnsafeCell::new(MaybeUninit::uninit()));

// One-time global runqueue (guarded by a Mutex).
static RQ_CELL: Once<Mutex<RunQueue>> = Once::new();

static mut IDLE_STACK: [u8; IDLE_STACK_SIZE] = [0; IDLE_STACK_SIZE];

/* --------------------------------- Utilities --------------------------------- */

#[inline]
fn tasks_slice_init() -> &'static mut [Option<Task>] {
    // SAFETY:
    // - Called exactly once within RQ_CELL.call_once().
    // - We write a valid `None` to each element before exposing any reference.
    // - We then yield a single &'static mut slice to RunQueue.
    unsafe {
        let arr_mu: &mut MaybeUninit<[Option<Task>; MAX_TASKS]> = &mut *RQ_TASKS_BUF.0.get();

        // Initialize each element to None in place without needing Option<Task>:Copy.
        let base: *mut Option<Task> = (*arr_mu).as_mut_ptr() as *mut Option<Task>;
        for i in 0..MAX_TASKS {
            base.add(i).write(None);
        }

        let arr_ref: &mut [Option<Task>; MAX_TASKS] = (*arr_mu).assume_init_mut();
        &mut arr_ref[..]
    }
}

#[inline]
fn rq() -> &'static Mutex<RunQueue> {
    RQ_CELL.call_once(|| {
        let tasks = tasks_slice_init();
        Mutex::new(RunQueue {
            tasks,
            current: None,
            next_id: 1,
            need_resched: false,
        })
    })
}

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
        let mut g = rq().lock();
        let rq: &mut RunQueue = &mut *g;

        // Build initial idle thread stack frame for kthread_trampoline
        let base = unsafe { core::ptr::addr_of_mut!(IDLE_STACK) as *mut u8 };
        let top = ((base as usize + IDLE_STACK_SIZE) & !0xF) as u64;

        // kthread_trampoline expects: [arg][entry] on the stack top (popped in that order)
        let init_rsp = (top - 16) as *mut u64;
        unsafe {
            core::ptr::write(init_rsp.add(0), 0u64); // arg
            core::ptr::write(init_rsp.add(1), idle_main as u64); // entry
        }

        rq.tasks[0] = Some(Task {
            id: rq.next_id,
            state: TaskState::Running,
            ctx: CpuContext {
                rip: kthread_trampoline as u64,
                rsp: init_rsp as u64,
                ..CpuContext::default()
            },
            kstack_top: top,
            simd: sched_simd::SimdArea::alloc(),
            time_slice: u32::MAX, // never timeslice the idle task
        });
        rq.next_id += 1;
        rq.current = Some(0);
    });
}

/* ------------------------------- Public API ---------------------------------- */

pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
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
        ..CpuContext::default()
    };

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;

        let idx = rq.tasks.iter().position(|t| t.is_none()).expect("no slots");
        rq.tasks[idx] = Some(Task {
            id,
            state: TaskState::Ready,
            ctx,
            simd: sched_simd::SimdArea::alloc(),
            kstack_top: top,
            time_slice: DEFAULT_SLICE,
        });
        id
    })
}

pub fn tick() {
    with_rq_locked(|rq| {
        let Some(cur) = rq.current else { return };
        let t = rq.tasks[cur].as_mut().unwrap();
        if t.time_slice == u32::MAX {
            return; // idle task
        }
        if t.time_slice > 0 {
            t.time_slice -= 1;
        }
        if t.time_slice == 0 {
            t.state = TaskState::Ready;
            t.time_slice = DEFAULT_SLICE;
            rq.need_resched = true;
        }
    });
}

pub fn should_preempt_now() -> bool {
    with_rq_locked(|rq| rq.need_resched)
}

#[unsafe(no_mangle)]
pub fn preempt_trampoline() {
    yield_now();
}

/* ------------------------------ Core switching ------------------------------- */

pub fn yield_now() {
    let (prev_ctx, next_ctx, prev_simd_ptr, next_simd_ptr) = {
        let mut g = rq().lock();
        let rq: &mut RunQueue = &mut *g;

        let cur = rq.current.expect("no current");
        let Some(nxt) = pick_next(rq, cur) else {
            return;
        };
        if nxt == cur {
            rq.need_resched = false;
            return;
        }

        {
            let t = rq.tasks[cur].as_mut().unwrap();
            t.state = TaskState::Ready;
            t.time_slice = DEFAULT_SLICE;
        }
        {
            let t = rq.tasks[nxt].as_mut().unwrap();
            t.state = TaskState::Running;
        }

        let (prev_ctx, prev_simd_ptr) = {
            let prev = rq.tasks[cur].as_mut().unwrap();
            (
                &mut prev.ctx as *mut CpuContext,
                prev.simd.as_ref().map(|s| s.as_mut_ptr()),
            )
        };
        let (next_ctx, next_simd_ptr) = {
            let next = rq.tasks[nxt].as_ref().unwrap();
            (
                &next.ctx as *const CpuContext,
                next.simd.as_ref().map(|s| s.as_mut_ptr()),
            )
        };

        rq.current = Some(nxt);
        rq.need_resched = false;

        (prev_ctx, next_ctx, prev_simd_ptr, next_simd_ptr)
    };

    // Save/restore SIMD around the context switch. Order: save prev -> switch -> restore next.
    if let Some(area) = prev_simd_ptr {
        unsafe {
            simd::save(area);
        } // or fxsave path inside impl when OSXSAVE=0
    }
    unsafe {
        context::switch(prev_ctx, next_ctx);
    }
    if let Some(area) = next_simd_ptr {
        unsafe {
            simd::restore(area);
        } // or fxrstor path inside impl
    }
}

fn pick_next(rq: &RunQueue, cur: usize) -> Option<usize> {
    // Simple round-robin over READY tasks; skip slot 0 unless no other READY
    for off in 1..rq.tasks.len() {
        let i = (cur + off) % rq.tasks.len();
        if i == 0 {
            continue;
        }
        if let Some(t) = &rq.tasks[i] {
            if matches!(t.state, TaskState::Ready) {
                return Some(i);
            }
        }
    }
    if let Some(Some(t0)) = rq.tasks.get(0) {
        if matches!(t0.state, TaskState::Ready | TaskState::Running) {
            return Some(0);
        }
    }
    None
}

pub fn exit_current() -> ! {
    let (prev_ctx, next_ctx) = {
        let mut g = rq().lock();
        let rq: &mut RunQueue = &mut *g;

        let cur = rq.current.expect("no current");
        rq.tasks[cur].as_mut().unwrap().state = TaskState::Dead;

        let Some(next_idx) = pick_next(rq, cur) else {
            drop(g);
            loop {
                x86_64::instructions::hlt();
            }
        };
        if next_idx == cur {
            drop(g);
            loop {
                x86_64::instructions::hlt();
            }
        }

        rq.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;

        let prev_ctx = &mut rq.tasks[cur].as_mut().unwrap().ctx as *mut _;
        let next_ctx = &rq.tasks[next_idx].as_ref().unwrap().ctx as *const _;
        rq.current = Some(next_idx);

        (prev_ctx, next_ctx)
    };

    unsafe {
        context::switch(prev_ctx, next_ctx);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

/* ------------------------------- Helper wrapper ------------------------------ */

fn with_rq_locked<F, R>(f: F) -> R
where
    F: FnOnce(&mut RunQueue) -> R,
{
    without_interrupts(|| {
        let mut guard = rq().lock();
        let rq: &mut RunQueue = &mut *guard;
        f(rq)
    })
}
