use spin::{Mutex, Once};

use crate::arch::x86_64::context::CpuContext;

use x86_64::instructions::{hlt, interrupts};

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

const MAX_TASKS: usize = 128;
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
        hlt();
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
    use spin::Once;
    static ONCE: Once<()> = Once::new();

    ONCE.call_once(|| {
        let mut q = rq().lock();

        // Build idle stack using raw ptrs (Rust 2024 forbids &mut to static mut)
        let base = core::ptr::addr_of_mut!(IDLE_STACK) as *mut u8;
        let top_u = ((base as usize) + IDLE_STACK_SIZE) & !0xF; // 16B align
        let top = top_u as u64;

        // Trampoline expects: [arg][entry] at RSP
        let init_rsp = (top - 16) as *mut u64;
        unsafe {
            core::ptr::write(init_rsp.add(0), 0u64); // arg
            core::ptr::write(init_rsp.add(1), idle_main as u64); // entry
        }

        q.tasks[0] = Some(Task {
            id: q.next_id,
            state: TaskState::Running,
            ctx: CpuContext {
                rip: kthread_trampoline as u64,
                rsp: init_rsp as u64,
                ..CpuContext::default()
            },
            kstack_top: top,
            time_slice: u32::MAX, // never preempt idle
        });
        q.next_id += 1;
        q.current = Some(0);

        crate::println!(
            "[SCHED] idle: top={:#018x} init_rsp={:#018x} ctx.rsp={:#018x} ctx.rip={:#018x}",
            top,
            init_rsp as u64,
            q.tasks[0].as_ref().unwrap().ctx.rsp,
            q.tasks[0].as_ref().unwrap().ctx.rip
        );
    });

    crate::println!("[SCHED] init done current={:?}", rq().lock().current);
}

pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    let top = ((stack_ptr as usize + stack_len) & !0xF) as u64; // 16B align

    // Lay out [arg][entry] at top-16 / top-8
    let init_rsp = (top - 16) as *mut u64;
    unsafe {
        core::ptr::write(init_rsp.add(0), arg as u64);
        core::ptr::write(init_rsp.add(1), entry as u64);
    }

    let ctx = CpuContext {
        rip: kthread_trampoline as u64,
        rsp: top - 16, // <-- critical: start below the two words
        ..CpuContext::default()
    };

    let mut rq = rq().lock();
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
}

// Called from timer ISR at 1 kHz.
pub fn tick() {
    let mut rq = rq().lock();
    let cur = match rq.current {
        Some(i) => i,
        None => return,
    };
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
}
fn pick_next(rq: &RunQueue, cur: usize) -> Option<usize> {
    let n = rq.tasks.len();

    // First pass: skip idle (index 0)
    for off in 1..n {
        let i = (cur + off) % n;
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
pub fn yield_now() {
    // prevent racing the timer ISR/IST
    interrupts::disable();

    let mut q = rq().lock();
    let cur = q.current.expect("no current");

    // only pick next when we’re not the only runner
    if let Some(next_idx) = pick_next(&q, cur) {
        q.tasks[cur].as_mut().unwrap().state = TaskState::Ready;
        q.tasks[cur].as_mut().unwrap().time_slice = DEFAULT_SLICE;

        q.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;

        // take raw ctx ptrs without aliasing two &mut at once
        let (prev_ctx, next_ctx) = {
            let prev_ptr = core::ptr::from_mut(&mut q.tasks[cur].as_mut().unwrap().ctx);
            let next_ptr = core::ptr::from_ref(&q.tasks[next_idx].as_ref().unwrap().ctx);
            (prev_ptr, next_ptr)
        };
        q.current = Some(next_idx);
        drop(q); // release lock before switching (still with IF=0)

        unsafe { crate::arch::x86_64::context::switch(prev_ctx, next_ctx) };
    }
}

pub fn exit_current() -> ! {
    // We'll capture these, then release the lock, then switch.
    let (prev_ctx, next_ctx);

    {
        let mut q = rq().lock();
        let cur = q.current.expect("no current");
        q.tasks[cur].as_mut().unwrap().state = TaskState::Dead;

        let Some(next_idx) = pick_next(&q, cur) else {
            // No runnable task; park this CPU forever.
            drop(q); // explicit or just fall out of scope
            loop {
                x86_64::instructions::hlt();
            }
        };

        q.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;
        prev_ctx = &mut q.tasks[cur].as_mut().unwrap().ctx as *mut _;
        next_ctx = &q.tasks[next_idx].as_ref().unwrap().ctx as *const _;
        q.current = Some(next_idx);
        // `q` is dropped here at end of this block (exactly once).
    }

    unsafe {
        crate::arch::x86_64::context::switch(prev_ctx, next_ctx);
    }
    // We should never return here, but if we do, park:
    loop {
        x86_64::instructions::hlt();
    }
}
use core::mem::{size_of, transmute};

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
