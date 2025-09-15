pub mod sched_simd;

use core::sync::atomic::{AtomicBool, Ordering};
use core::u32;

use spin::Mutex;
use x86_64::instructions::hlt;

extern crate alloc;

use crate::arch::x86_64::context::{CpuContext, switch};
use crate::arch::x86_64::simd::{restore, save};
use crate::kprintln;
use crate::sched::sched_simd::SimdArea;

/* ------------------------------- Types & consts ------------------------------- */

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Dead,
}

pub type TaskId = u64;

#[derive(Clone, Copy)]
pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub simd: SimdArea,
    pub kstack_top: u64,
    pub time_slice: u32,
}

const MAX_TASKS: usize = 128;
pub const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz
const IDLE_STACK_SIZE: usize = 16 * 1024;

/* ----------------------------- Runqueue container ----------------------------- */

struct RunQueue {
    tasks: [Task; MAX_TASKS],
    current: usize,
    next_id: TaskId,
    need_resched: bool,
}

static RQ: Mutex<RunQueue> = Mutex::new(RunQueue {
    tasks: [Task {
        id: 0,
        ctx: CpuContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            rsp: 0,
            rip: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rsi: 0,
            rdi: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,
            rflags: 0,
        },
        simd: SimdArea {
            dump: [0; sched_simd::SIZE],
        },
        state: TaskState::Dead,
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

/* --------------------------------- Utilities --------------------------------- */

extern "C" fn idle_main(_arg: usize) -> ! {
    loop {
        kprintln!("IDLE");
        hlt();
    }
}

/* --------------------------------- Init path --------------------------------- */

static INIT_DONE: AtomicBool = AtomicBool::new(false);

fn ensure_init() {
    if !INIT_DONE.load(Ordering::SeqCst) {
        init();
    }
}

unsafe extern "C" {
    fn kthread_trampoline() -> !;
}

pub fn init() {
    let top_aligned = ((&raw const IDLE_STACK as usize + IDLE_STACK_SIZE) & !0xF) as u64; // 16-align
    let frame = (top_aligned - 16) as *mut u64; // space for [arg][entry]
    unsafe {
        core::ptr::write(frame.add(0), 0 as u64);
        core::ptr::write(frame.add(1), idle_main as u64);
    }

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        let idx = rq
            .tasks
            .iter()
            .position(|t| matches!(t.state, TaskState::Dead))
            .expect("no slots");

        let t = &mut rq.tasks[idx];
        *t = Task {
            id,
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
            kstack_top: top_aligned,
            time_slice: u32::MAX,
        };
    })
}

/* ------------------------------- Public API ---------------------------------- */
pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    let top_aligned = ((stack_ptr as usize + stack_len) & !0xF) as u64; // 16-align
    let frame = (top_aligned - 16) as *mut u64; // space for [arg][entry]
    unsafe {
        // [0] = arg, [1] = entry
        core::ptr::write(frame.add(0), arg as u64);
        core::ptr::write(frame.add(1), entry as u64);
    }

    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        let idx = rq
            .tasks
            .iter()
            .position(|t| matches!(t.state, TaskState::Dead /* or Void */))
            .expect("no slots");

        let t = &mut rq.tasks[idx];
        *t = Task {
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
            kstack_top: top_aligned,
            time_slice: DEFAULT_SLICE,
        };
        id
    })
}

pub fn tick() {
    let Some((prev_ctx, next_ctx)) = with_rq_locked(|rq| {
        let current = rq.current;
        {
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
        let cur_is_idle;
        {
            let t = &rq.tasks[current];
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
                    let t = &mut rq.tasks[current];
                    if t.time_slice != u32::MAX {
                        t.state = TaskState::Ready;
                        t.time_slice = DEFAULT_SLICE;
                    }
                }
            }
            rq.tasks[next].state = TaskState::Running;

            let (prev_ctx, prev_simd) = {
                let prev = &mut rq.tasks[current];
                (&mut prev.ctx as *mut CpuContext, prev.simd.as_mut_ptr())
            };
            let (next_ctx, next_simd) = {
                let next = &mut rq.tasks[next];
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
    ensure_init();
    with_rq_locked(|rq| {
        let cur = rq.current;
        rq.tasks[cur].state = TaskState::Dead;
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
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut guard = RQ.lock();
        f(&mut guard)
    })
}
