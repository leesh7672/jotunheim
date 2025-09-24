// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub mod exec;
pub mod sched_simd;

use core::array::from_mut;
use core::{ptr, u32};

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::hlt;
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;

use crate::arch::native::simd::{restore, save};
use crate::arch::x86_64::tables::gdt::{kernel_cs, kernel_ds};
use crate::debug::TrapFrame;
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

#[derive(Clone)]
pub struct Task {
    id: TaskId,
    state: TaskState,
    simd: SimdArea,
    time_slice: u32,
    tf: TrapFrame,
    _stack: Box<ThreadStack>,
}

pub const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz

/* ----------------------------- Runqueue container ----------------------------- */

struct RunQueue {
    tasks: Vec<Box<Task>>,
    current: Option<usize>,
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
        if let Some(current) = self.current {
            let start = (current + 1) % n;
            let mut i = start;
            loop {
                if i != current && matches!(self.tasks[i].state, TaskState::Ready) {
                    return Some(i);
                }
                i = (i + 1) % n;
                if i == start {
                    break;
                }
            }
        } else {
            for i in 0..n {
                if matches!(self.tasks[i].state, TaskState::Ready) {
                    return Some(i);
                }
            }
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
    let top = ((stack_ptr as usize) & !0xF) as u64;
    let frame = top - 0x30;
    unsafe {
        let frame_ptr = frame as *mut u64;
        ptr::write(frame_ptr.add(0), kthread_trampoline as u64);
        ptr::write(frame_ptr.add(1), kernel_cs() as u64);
        ptr::write(frame_ptr.add(2), 0x202);
        ptr::write(frame_ptr.add(3), 0u64);
        ptr::write(frame_ptr.add(4), idle_main as u64);
    };
    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        rq.tasks.insert(
            0,
            Box::new(Task {
                id,
                state: TaskState::Ready,
                simd: SimdArea {
                    dump: [0; sched_simd::SIZE],
                },
                tf: TrapFrame {
                    rip: kthread_trampoline as u64,
                    rsp: frame,
                    cs: kernel_cs() as u64 & !3,
                    rflags: 0x202,
                    ss: kernel_ds() as u64,
                    ..TrapFrame::default()
                },
                time_slice: u32::MAX,
                _stack: stack,
            }),
        );
    });
    spawn(|| {
        loop {
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
    let top = ((stack_ptr as usize) & !0xF) as u64;
    let frame = top - 0x30;
    unsafe {
        let frame_ptr = frame as *mut u64;
        ptr::write(frame_ptr.add(0), kthread_trampoline as u64);
        ptr::write(frame_ptr.add(1), kernel_cs() as u64);
        ptr::write(frame_ptr.add(2), 0x202);
        ptr::write(frame_ptr.add(3), arg as u64);
        ptr::write(frame_ptr.add(4), entry as u64);
    };
    let mut element = Box::new(Task {
        state: TaskState::Ready,
        simd: SimdArea {
            dump: [0; sched_simd::SIZE],
        },
        tf: TrapFrame {
            rip: kthread_trampoline as u64,
            rsp: frame,
            cs: kernel_cs() as u64,
            rflags: 0x202,
            ss: kernel_ds() as u64,
            ..TrapFrame::default()
        },
        time_slice: DEFAULT_SLICE,
        _stack: stack,
        id: 0,
    });

    with_rq_locked(move |rq| {
        let id = rq.next_id;
        element.id = id;
        rq.next_id += 1;
        rq.tasks.insert(0, element);
        if let Some(current) = rq.current {
            *rq.current.as_mut().unwrap() = current + 1;
        }
        id
    })
}

pub fn yield_now() {}

pub fn tick(tf: TrapFrame) -> TrapFrame {
    let Some(ntf) = with_rq_locked(|rq| {
        let extra: bool;
        if let Some(current) = rq.current {
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
            extra = cur_is_idle && some_ready;
        } else {
            rq.need_resched = true;
            extra = true;
        }
        if !(rq.need_resched || extra) {
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
            if let Some(current) = rq.current {
                let t = rq.tasks[current].as_mut();
                t.state = TaskState::Ready;
                if t.time_slice != u32::MAX {
                    t.time_slice = DEFAULT_SLICE;
                }
                save(rq.tasks[current].simd.as_mut_ptr());
                rq.tasks[current].tf = tf;
            }
            rq.need_resched = false;
            rq.tasks[next_idx].as_mut().state = TaskState::Running;
            rq.current = Some(next_idx);

            restore(rq.tasks[next_idx].simd.as_mut_ptr());
            Some(rq.tasks[next_idx].tf)
        }
    }) else {
        return tf;
    };
    ntf
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
        if let Some(current) = rq.current {
            let task = rq.tasks[current].as_mut();
            task.state = TaskState::Dead;
            task.time_slice = DEFAULT_SLICE * 2;
        }
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
        let ret;
        if let Some(rq) = op {
            ret = f(rq.as_mut());
        } else {
            *guard = Some(Box::new(RunQueue {
                tasks: Vec::new(),
                current: None,
                next_id: 0,
                need_resched: true,
            }));
            ret = f(guard.as_mut().unwrap().as_mut());
        }
        drop(guard);
        ret
    })
}
