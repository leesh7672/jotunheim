// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub mod exec;
pub mod sched_simd;

use core::u32;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::hlt;
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;

use crate::arch::native::context::{switch, CpuContext};
use crate::arch::native::simd::{restore, save};
use crate::kprintln;
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
    id: TaskId,
    state: TaskState,
    ctx: CpuContext,
    simd: SimdArea,
    time_slice: u32,
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
        if matches!(self.tasks[self.current].state, TaskState::Ready) {
            return Some(i);
        }
        let t0 = &self.tasks[0];
        if matches!(t0.state, TaskState::Ready) {
            return Some(0);
        }
        None
    }
}

/* Thread Stack */
#[derive(Clone, Debug)]
struct ThreadStack {
    dump: Box<[u8]>,
}

impl ThreadStack {
    fn new() -> Self {
        const STACK_SIZE: usize = 0x4_0000;
        let dump = vec![0u8; STACK_SIZE].into_boxed_slice();
        ThreadStack { dump }
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
    unsafe fn kthread_trampoline() -> !;
}

pub fn init() {
    let mut stack = Box::new(ThreadStack::new());
    let dump = stack.as_mut().dump.as_mut();
    let stack_ptr: *mut u8 = &raw mut dump[dump.len() - 1];
    let top_aligned = ((stack_ptr as usize) & !0xF) as u64; // 16-align
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
                id,
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
                time_slice: u32::MAX,
                _stack: stack,
            }),
        );
    });
    spawn(|| loop {
        for _ in 0..1000 {
            yield_now();
        }
        with_rq_locked(|rq| {
            let tasks: &mut Vec<Box<Task>> = rq.tasks.as_mut();
            let mut deads = Vec::<u64>::new();
            for task in tasks.iter_mut() {
                if task.state == TaskState::Dead {
                    if task.time_slice == 0 {
                        deads.insert(0, task.id);
                    } else {
                        task.time_slice -= 1;
                    }
                }
            }
            for id in deads {
                let mut i = 0;
                while id == tasks[i].id {
                    i += 1;
                }
                tasks.remove(i);
            }
        });
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
    let main = unsafe { Box::from_raw(arg as *mut ThreadFn<F>) };
    (main.func)();
    exit_current()
}

/* ------------------------------- Public API ---------------------------------- */

pub fn spawn<F>(func: F)
where
    F: FnOnce() -> (),
{
    let arg = Box::new(ThreadFn { func });
    spawn_kthread(thread_main::<F>, Box::into_raw(arg) as usize);
}

fn spawn_kthread(entry: extern "C" fn(usize) -> !, arg: usize) -> TaskId {
    let mut stack = Box::new(ThreadStack::new());
    let dump = stack.as_mut().dump.as_mut();
    let stack_ptr: *mut u8 = &raw mut dump[dump.len() - 1];
    let top_aligned = ((stack_ptr as usize) & !0xF) as u64;
    let frame = (top_aligned - 16) as *mut u64;
    unsafe {
        core::ptr::write(frame.add(0), arg as u64);
        core::ptr::write(frame.add(1), entry as u64);
    }

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        rq.tasks.insert(
            0,
            Box::new(Task {
                id,
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
                time_slice: DEFAULT_SLICE,
                _stack: stack,
            }),
        );
        rq.current += 1;
        rq.need_resched = true;
        id
    })
}

pub fn yield_now() {
    let Some((mut prev, mut next)) = with_rq_locked(|rq| {
        let current = rq.current;
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
            let t = rq.tasks[current].as_mut();
            t.state = TaskState::Ready;
            if t.time_slice != u32::MAX {
                t.time_slice = DEFAULT_SLICE;
            }
        }
        rq.tasks[next].as_mut().state = TaskState::Running;
        rq.need_resched = false;
        Some((rq.tasks[current].clone(), rq.tasks[next].clone()))
    }) else {
        return;
    };
    save(prev.simd.as_mut_ptr());
    restore(next.simd.as_mut_ptr());
    switch(&mut prev.ctx, &mut next.ctx);
}

pub fn tick() {
    let Some((prev_ptr, prev_simd, next_ptr, next_simd)) = with_rq_locked(|rq| {
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
            let next_idx;
            {
                let picked = rq.pick_next();
                if picked.is_none() {
                    return None;
                } else {
                    next_idx = picked.unwrap();
                }
            }
            {
                let t = rq.tasks[current].as_mut();
                t.state = TaskState::Ready;
                if t.time_slice != u32::MAX {
                    t.time_slice = DEFAULT_SLICE;
                }
            }
            rq.tasks[next_idx].as_mut().state = TaskState::Running;
            let prev_idx = current;
            let (prev, next) = if prev_idx < next_idx {
                let (left, right) = rq.tasks.split_at_mut(next_idx);
                let prev = &mut left[prev_idx];
                let next = &mut right[0]; // element at next_idx
                (prev, next)
            } else {
                let (left, right) = rq.tasks.split_at_mut(prev_idx);
                let next = &mut left[next_idx]; // element at next_idx
                let prev = &mut right[0]; // element at prev_idx
                (prev, next)
            };

            // Capture stable raw pointers (Boxes don’t move)
            let prev_ctx = &raw mut prev.ctx;
            let next_ctx = &raw mut next.ctx;
            let prev_simd = prev.simd.as_mut_ptr();
            let next_simd = next.simd.as_mut_ptr();

            Some((prev_ctx, prev_simd, next_ctx, next_simd))
        }
    }) else {
        return;
    };
    unsafe {
        save(prev_simd);
        restore(next_simd);
        switch(&mut *prev_ptr, &mut *next_ptr);
    }
}
/* ------------------------------ Core switching ------------------------------- */

pub fn exit_current() -> ! {
    kill_current();
    loop {
        hlt();
    }
}

fn kill_current() {
    with_rq_locked(|rq| {
        let task = rq.tasks[rq.current].as_mut();
        task.state = TaskState::Dead;
        task.time_slice = DEFAULT_SLICE * 2;
    });
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
        let ret: R;
        if let Some(rq) = op {
            ret = f(rq.as_mut())
        } else {
            *guard = Some(Box::new(RunQueue {
                tasks: Vec::new(),
                current: 0,
                next_id: 0,
                need_resched: true,
            }));
            ret = f(guard.as_mut().unwrap().as_mut())
        }
        ret
    })
}
