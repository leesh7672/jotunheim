#![allow(clippy::missing_safety_doc)]

pub mod sched_simd;

use core::u32;

use spin::Mutex;
use x86_64::instructions::hlt;
use x86_64::instructions::interrupts::without_interrupts;

extern crate alloc;

use crate::arch::x86_64::context;
use crate::arch::x86_64::context::CpuContext;
use crate::arch::x86_64::simd;
use crate::kprint;
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

const MAX_TASKS: usize = 128;
pub const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz
const IDLE_STACK_SIZE: usize = 16 * 1024;

/* ----------------------------- Runqueue container ----------------------------- */

pub struct RunQueue {
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

extern "C" fn idle_main(_arg: usize) -> ! {
    loop {
        hlt();
    }
}

/* --------------------------------- Init path --------------------------------- */

pub fn init() {
    spawn_kthread(idle_main, 0, core::ptr::addr_of_mut!(IDLE_STACK) as *mut u8, IDLE_STACK_SIZE);
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
    let init_rsp = 16 as *mut u64;
    with_rq_locked(|rq| {
        let id = rq.next_id;
        rq.next_id += 1;
        let idx = rq
            .tasks
            .iter()
            .position(|t| t.state == TaskState::Void)
            .expect("no slots");

        let task = &mut rq.tasks[idx];
        task.id = id;
        task.state = TaskState::Ready;

        task.ctx.rip = entry as u64;
        task.ctx.rdi = arg as u64;
        task.ctx.rsp = init_rsp as u64;
        task.ctx.rflags = 0x202;
        task.kstack_top = top;
        task.time_slice = DEFAULT_SLICE;

        for i in 0..sched_simd::SIZE {
            task.simd.dump[i] = 0;
        }
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

pub fn tick() -> *const PreemptPack {
    with_rq_locked(|rq| {
        let current = rq.current;
        let t = &mut rq.tasks[current];
        if t.time_slice != u32::MAX && t.time_slice > 0 {
            t.time_slice -= 1;
            if t.time_slice == 0 {
                t.state = TaskState::Ready;
                t.time_slice = DEFAULT_SLICE;
                rq.need_resched = true;
            }
        }

        let cur_is_idle = rq.tasks[current].time_slice == u32::MAX;

        let some_ready = {
            rq.tasks
                .iter()
                .enumerate()
                .any(|(i, t)| i != current && t.state == TaskState::Ready)
        };

        if !(rq.need_resched || (cur_is_idle && some_ready)) {
            return core::ptr::null();
        } else {
            let Some(next) = rq.pick_next() else {
                rq.need_resched = false;
                return core::ptr::null();
            };

            if next == current {
                rq.need_resched = false;
                return core::ptr::null();
            }
            let t = &mut rq.tasks[current];
            if t.time_slice != u32::MAX {
                t.state = TaskState::Ready;
                t.time_slice = DEFAULT_SLICE;
            }
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
    without_interrupts(|| {
        let mut guard = RQ.lock();
        let rq: &mut RunQueue = &mut *guard;
        f(rq)
    })
}
