use spin::{Mutex, Once};

use crate::arch::x86_64::context::CpuContext;

use x86_64::instructions::interrupts;

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

pub type TaskId = u64;

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub kstack_top: u64,
    pub time_slice: u32, // ticks remaining
}

const MAX_TASKS: usize = 192;
const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz
const IDLE_STACK_SIZE: usize = 16 * 1024;

struct RunQueue {
    tasks: [Option<Task>; MAX_TASKS],
    current: Option<usize>,
    next_id: TaskId,
    need_resched: bool,
}

static RQ_ONCE: Once<Mutex<RunQueue>> = Once::new();
static mut IDLE_STACK: [u8; IDLE_STACK_SIZE] = [0; IDLE_STACK_SIZE];

#[unsafe(no_mangle)]
pub extern "C" fn sched_exit_current_trampoline() -> ! {
    exit_current()
}

extern "C" fn idle_main(_arg: usize) -> ! {
    loop {
        yield_now();
    }
}

#[inline]
fn rq() -> &'static Mutex<RunQueue> {
    RQ_ONCE.call_once(|| {
        let tasks: [Option<Task>; MAX_TASKS] = core::array::from_fn(|_| None);
        Mutex::new(RunQueue {
            tasks,
            current: None,
            next_id: 1,
            need_resched: false,
        })
    })
}
pub fn init() {
    static ONCE: spin::Once<()> = spin::Once::new();
    ONCE.call_once(|| {
        let mut rq = rq().lock();

        // --- build a proper stack for idle (slot 0) ---
        let base = core::ptr::addr_of_mut!(IDLE_STACK) as *mut u8;
        let top = ((base as usize + IDLE_STACK_SIZE) & !0xF) as u64;
        let init_rsp = (top - 16) as *mut u64;
        unsafe {
            core::ptr::write(init_rsp.add(0), 0u64); // arg = 0
            core::ptr::write(init_rsp.add(1), idle_main as u64); // entry = idle_main
        }

        rq.tasks[0] = Some(Task {
            id: rq.next_id,
            state: TaskState::Running,
            ctx: CpuContext {
                rip: kthread_trampoline as u64,
                rsp: top - 16, // <- important
                ..CpuContext::default()
            },
            kstack_top: top,
            time_slice: u32::MAX, // never preempt
        });
        rq.next_id += 1;
        rq.current = Some(0);
    });
}

pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    let top = ((stack_ptr as usize + stack_len) & !0xF) as u64;
    let init_rsp = (top - 16) as *mut u64;
    unsafe {
        core::ptr::write(init_rsp.add(0), arg as u64); // will be popped into rdi
        core::ptr::write(init_rsp.add(1), entry as u64); // will be popped into rax
    }

    let ctx = CpuContext {
        rip: kthread_trampoline as u64,
        rsp: init_rsp as u64, // <-- was `top`; must be `top - 16`
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
            return;
        } // idle
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

pub fn yield_now() {
    let mut rq = rq().lock();
    let cur = rq.current.expect("no current");

    let Some(next_idx) = pick_next(&rq, cur) else {
        // no runnable task (shouldn’t happen because idle exists)
        return;
    };

    if next_idx == cur {
        // nothing to do; keep running current
        return;
    }

    // switch states/slices
    rq.tasks[cur].as_mut().unwrap().state = TaskState::Ready;
    rq.tasks[cur].as_mut().unwrap().time_slice = DEFAULT_SLICE;
    rq.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;

    // take context pointers while lock is held
    let (prev_ctx, next_ctx) = {
        let prev = &mut rq.tasks[cur].as_mut().unwrap().ctx as *mut _;
        let next = &rq.tasks[next_idx].as_ref().unwrap().ctx as *const _;
        (prev, next)
    };
    rq.current = Some(next_idx);
    rq.need_resched = false;
    drop(rq);
    unsafe {
        crate::arch::x86_64::context::switch(prev_ctx, next_ctx);
    }
}

fn pick_next(rq: &RunQueue, cur: usize) -> Option<usize> {
    let n = rq.tasks.len();

    // Prefer non-idle tasks, skipping current first
    for off in 1..n {
        let i = (cur + off) % n;
        if i == 0 {
            continue;
        } // skip idle
        if let Some(t) = &rq.tasks[i] {
            if matches!(t.state, TaskState::Ready) {
                return Some(i);
            }
        }
    }

    // Fall back to idle if nothing else is runnable
    if let Some(Some(t0)) = rq.tasks.get(0) {
        if matches!(t0.state, TaskState::Ready | TaskState::Running) {
            return Some(0);
        }
    }

    None
}

pub fn exit_current() -> ! {
    let (prev_ctx, next_ctx);

    {
        let mut q = rq().lock();
        let cur = q.current.expect("no current");
        q.tasks[cur].as_mut().unwrap().state = TaskState::Dead;

        let Some(next_idx) = pick_next(&q, cur) else {
            drop(q);
            loop {
                x86_64::instructions::hlt();
            }
        };

        if next_idx == cur {
            // if we somehow picked ourselves and we’re Dead, park
            drop(q);
            loop {
                x86_64::instructions::hlt();
            }
        }

        q.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;
        prev_ctx = &mut q.tasks[cur].as_mut().unwrap().ctx as *mut _;
        next_ctx = &q.tasks[next_idx].as_ref().unwrap().ctx as *const _;
        q.current = Some(next_idx);
        // drop lock here
    }

    unsafe {
        crate::arch::x86_64::context::switch(prev_ctx, next_ctx);
    }
    loop {
        x86_64::instructions::hlt();
    }
}

use core::mem::size_of;

pub fn ctx_layout_sanity() {
    // size
    crate::println!("[CTX] size = {:#x}", size_of::<CpuContext>());

    // offsets
    let base = 0usize;
    let sample = CpuContext::default();
    let p: *const CpuContext = &sample;

    unsafe {
        crate::println!(
            "[CTX] off r15={:#x}",
            (&(*p).r15 as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off r14={:#x}",
            (&(*p).r14 as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off r13={:#x}",
            (&(*p).r13 as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off r12={:#x}",
            (&(*p).r12 as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off rbx={:#x}",
            (&(*p).rbx as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off rbp={:#x}",
            (&(*p).rbp as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off rsp={:#x}",
            (&(*p).rsp as *const _ as usize) - (p as usize)
        );
        crate::println!(
            "[CTX] off rip={:#x}",
            (&(*p).rip as *const _ as usize) - (p as usize)
        );
    }
}

fn with_rq_locked<F, R>(f: F) -> R
where
    F: FnOnce(&mut RunQueue) -> R,
{
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut guard = rq().lock();
        f(&mut guard)
    })
}
