pub mod sched_simd;

use core::sync::atomic::{AtomicBool, Ordering};
use core::u32;

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::hlt;
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;

use crate::arch::x86_64::context::{CpuContext, switch};
use crate::arch::x86_64::simd::{restore, save};
use crate::sched::sched_simd::SimdArea;

/* ------------------------------- Types & consts ------------------------------- */

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Dead,
}

pub type TaskId = u64;

#[derive(Clone, Debug)]
pub struct Task {
    pub _id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub simd: SimdArea,
    pub _kstack_top: u64,
    pub time_slice: u32,
    _stack: Box<ThreadStack>,
}

pub const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz

/* ----------------------------- Runqueue container ----------------------------- */

struct RunQueue {
    tasks: Vec<Box<Task>>,
    current: usize,
    next_id: TaskId,
    need_resched: bool,
}

static RQ: Mutex<Option<Box<RunQueue>>> = Mutex::new(None);

impl RunQueue {
    fn pick_next(&self) -> Option<usize> {
        let n = self.tasks.len();
        if n == 0 {
            return None;
        }
        let start = (self.current + 1) % n;
        let mut i = start;
        loop {
            if i != self.current && matches!(self.tasks[i].state, TaskState::Ready) {
                return Some(i);
            }
            i = (i + 1) % n;
            if i == start {
                break;
            }
        }
        let t0 = &self.tasks[0];
        if matches!(t0.state, TaskState::Ready) && self.current != 0 {
            return Some(0);
        }
        None
    }
}

/* Thread Stack */

const STACK_SIZE: usize = 0x8000;

#[derive(Clone, Debug)]
struct ThreadStack {
    dump: Box<[u8; STACK_SIZE]>,
}

impl ThreadStack {
    fn new() -> Self {
        ThreadStack {
            dump: Box::new([0; STACK_SIZE]),
        }
    }
}

/* --------------------------------- Utilities --------------------------------- */

extern "C" fn idle_main(_arg: usize) -> ! {
    loop {
        hlt();
    }
}

/* --------------------------------- Init path --------------------------------- */

unsafe extern "C" {
    fn kthread_trampoline() -> !;
}

pub fn init() {
    let mut stack = Box::new(ThreadStack::new());
    let stack_ptr: *mut u8 = stack.as_mut().dump.as_mut_ptr();
    let top_aligned = ((stack_ptr as usize + STACK_SIZE) & !0xF) as u64; // 16-align
    let frame = (top_aligned - 16) as *mut u64; // space for [arg][entry]
    unsafe {
        core::ptr::write(frame.add(0), 0 as u64);
        core::ptr::write(frame.add(1), idle_main as u64);
    }

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        rq.tasks.insert(
            0,
            Box::new(Task {
                _id: id,
                state: TaskState::Ready,
                ctx: CpuContext {
                    // zero GPRs you don’t care about; set the essentials:
                    rip: kthread_trampoline as u64, // <- trampoline first
                    rsp: frame as u64,
                    rflags: 0x202,
                    ..CpuContext::default()
                },
                simd: SimdArea {
                    dump: [0; sched_simd::SIZE],
                },
                _kstack_top: top_aligned,
                time_slice: u32::MAX,
                _stack: stack,
            }),
        );
    });
    spawn(|| {
        loop {
            with_rq_locked(|rq| {
                let tasks: &mut Vec<Box<Task>> = rq.tasks.as_mut();
                let mut deads = Vec::<u64>::new();
                for task in tasks.iter_mut() {
                    if task.state == TaskState::Dead {
                        if task.time_slice == 0 {
                            deads.insert(0, task._id);
                        } else {
                            task.time_slice -= 1;
                        }
                    }
                }
                for id in deads {
                    let mut i = 0;
                    while id == tasks[i]._id {
                        i += 1;
                    }
                    tasks.remove(i);
                }
            })
        }
    });
}

struct ThreadFn<F>
where
    F: FnOnce() -> (),
{
    func: F,
}

extern "C" fn thread_main<F>(arg: usize) -> !
where
    F: FnOnce() -> (),
{
    let main = arg as *mut ThreadFn<F>;
    let f = unsafe { main.read().func };
    f();
    exit_current()
}

/* ------------------------------- Public API ---------------------------------- */

pub fn spawn<F>(func: F)
where
    F: FnOnce() -> (),
{
    let mut arg = Box::leak(Box::new(ThreadFn { func }));
    spawn_kthread(thread_main::<F>, &raw mut arg as usize);
}

fn spawn_kthread(entry: extern "C" fn(usize) -> !, arg: usize) -> TaskId {
    let mut stack = Box::new(ThreadStack::new());
    let stack_ptr: *mut u8 = stack.as_mut().dump.as_mut_ptr();
    let top_aligned = ((stack_ptr as usize + STACK_SIZE) & !0xF) as u64; // 16-align
    let frame = (top_aligned - 16) as *mut u64; // space for [arg][entry]
    unsafe {
        // [0] = arg, [1] = entry
        core::ptr::write(frame.add(0), arg as u64);
        core::ptr::write(frame.add(1), entry as u64);
    }

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;

        rq.tasks.insert(
            1,
            Box::new(Task {
                _id: id,
                state: TaskState::Ready,
                ctx: CpuContext {
                    rip: kthread_trampoline as u64,
                    rsp: frame as u64,
                    rflags: 0x202,
                    ..CpuContext::default()
                },
                simd: SimdArea {
                    dump: [0; sched_simd::SIZE],
                },
                _kstack_top: top_aligned,
                time_slice: DEFAULT_SLICE,
                _stack: stack,
            }),
        );
        id
    })
}

pub fn tick() {
    let Some((prev_ctx, next_ctx)) = with_rq_locked(|rq| {
        let current = rq.current;
        {
            let t = rq.tasks[current].as_mut();
            if t.time_slice != u32::MAX && t.time_slice > 0 {
                t.time_slice -= 1;
                if t.time_slice == 0 {
                    t.time_slice = DEFAULT_SLICE;
                    rq.need_resched = true;
                }
            }
        }

        let cur_is_idle;
        {
            let t = rq.tasks[current].as_mut();
            cur_is_idle = t.time_slice == u32::MAX;
        }

        let some_ready;
        {
            some_ready = rq
                .tasks
                .iter()
                .enumerate()
                .any(|(i, t)| i != current && t.state == TaskState::Ready)
        }

        if !(rq.need_resched || (cur_is_idle && some_ready)) {
            return None;
        } else {
            let next;
            {
                let picked = rq.pick_next();
                if picked.is_none() {
                    return None;
                } else {
                    next = picked.unwrap();
                }
            }
            {
                if next == current {
                    rq.need_resched = false;
                    return None;
                }
                {
                    let t = rq.tasks[current].as_mut();
                    if t.time_slice != u32::MAX {
                        t.state = TaskState::Ready;
                        t.time_slice = DEFAULT_SLICE;
                    }
                }
            }
            rq.tasks[next].as_mut().state = TaskState::Running;

            let (prev_ctx, prev_simd) = {
                let prev = rq.tasks[current].as_mut();
                (&mut prev.ctx as *mut CpuContext, prev.simd.as_mut_ptr())
            };
            let (next_ctx, next_simd) = {
                let next = rq.tasks[next].as_mut();
                (&next.ctx as *const CpuContext, next.simd.as_mut_ptr())
            };

            rq.current = next;
            rq.need_resched = false;

            save(prev_simd);
            restore(next_simd);
            Some((prev_ctx, next_ctx))
        }
    }) else {
        return;
    };
    switch(prev_ctx, next_ctx);
}
/* ------------------------------ Core switching ------------------------------- */

pub fn exit_current() -> ! {
    with_rq_locked(|rq| {
        let task = rq.tasks[rq.current].as_mut();
        task.state = TaskState::Dead;
        task.time_slice = DEFAULT_SLICE * 2;
    });
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_exit_current_trampoline() -> ! {
    exit_current()
}

/* ------------------------------- Helper wrapper ------------------------------ */

fn with_rq_locked<F, R>(f: F) -> R
where
    F: FnOnce(&mut RunQueue) -> R,
{
    without_interrupts(|| {
        let mut guard = RQ.lock();
        let op = guard.as_mut();
        if let Some(rq) = op {
            f(rq.as_mut())
        } else {
            *guard = Some(Box::new(RunQueue {
                tasks: Vec::new(),
                current: 0,
                next_id: 0,
                need_resched: false,
            }));
            f(guard.as_mut().unwrap().as_mut())
        }
    })
}
