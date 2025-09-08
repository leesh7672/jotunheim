use spin::{Mutex, Once};

use crate::arch::x86_64::context::CpuContext;

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
    pub _id: TaskId,
    pub state: TaskState,
    pub ctx: CpuContext,
    pub _kstack_top: u64,
    pub time_slice: u32, // ticks remaining
}

const MAX_TASKS: usize = u16::MAX as usize;
const DEFAULT_SLICE: u32 = 5; // 5ms at 1 kHz

struct RunQueue {
    tasks: [Option<Task>; MAX_TASKS],
    current: Option<usize>,
    next_id: TaskId,
    need_resched: bool,
}

static RQ_ONCE: Once<Mutex<RunQueue>> = Once::new();

#[unsafe(no_mangle)]
pub extern "C" fn sched_exit_current_trampoline() -> ! {
    exit_current()
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
static INIT_ONCE: Once<()> = Once::new();

pub fn init() {
    INIT_ONCE.call_once(|| {
        // Create idle task; we "become" idle by installing a context record for the current CPU
        let mut rq = rq().lock();
        let idle_idx = 0;
        let idle = Task {
            _id: rq.next_id,
            state: TaskState::Running,
            ctx: CpuContext::default(),
            _kstack_top: 0,
            time_slice: u32::MAX,
        };
        rq.next_id += 1;
        rq.tasks[idle_idx] = Some(idle);
        rq.current = Some(idle_idx);
    });
}

pub fn spawn_kthread(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_ptr: *mut u8,
    stack_len: usize,
) -> TaskId {
    let top = ((stack_ptr as usize + stack_len) & !0xF) as u64; // 16B align

    let init_rsp = (top - 16) as *mut u64;
    unsafe {
        // write arg then entry; order matches the trampoline's two POPs
        core::ptr::write(init_rsp.add(0), arg as u64);
        core::ptr::write(init_rsp.add(1), entry as u64);
    }

    let ctx = CpuContext {
        rip: kthread_trampoline as u64,
        rsp: top,
        ..CpuContext::default()
    };

    let mut rq = rq().lock(); // (or RQ.lock() if you kept the static)
    let id = rq.next_id;
    rq.next_id += 1;

    let idx = rq.tasks.iter().position(|t| t.is_none()).expect("no slots");
    rq.tasks[idx] = Some(Task {
        _id: id,
        state: TaskState::Ready,
        ctx,
        _kstack_top: top,
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
    for off in 1..n {
        let i = (cur + off) % n;
        if let Some(t) = &rq.tasks[i] {
            if matches!(t.state, TaskState::Ready) {
                return Some(i);
            }
        }
    }
    None
}

// Voluntary yield from thread context.
pub fn yield_now() {
    let mut rq = rq().lock();
    let cur = rq.current.expect("no current");
    let next = if rq.need_resched {
        rq.need_resched = false;
        pick_next(&rq, cur)
    } else {
        pick_next(&rq, cur)
    };
    if let Some(next_idx) = next {
        rq.tasks[cur].as_mut().unwrap().state = TaskState::Ready;
        rq.tasks[cur].as_mut().unwrap().time_slice = DEFAULT_SLICE;
        rq.tasks[next_idx].as_mut().unwrap().state = TaskState::Running;
        let (prev_ctx, next_ctx) = {
            let prev = &mut rq.tasks[cur].as_mut().unwrap().ctx as *mut _;
            let next = &rq.tasks[next_idx].as_ref().unwrap().ctx as *const _;
            (prev, next)
        };
        rq.current = Some(next_idx);
        drop(rq);
        unsafe {
            crate::arch::x86_64::context::switch(prev_ctx, next_ctx);
        }
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
