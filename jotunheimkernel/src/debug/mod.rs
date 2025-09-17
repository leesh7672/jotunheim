#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::identity_op)]

use spin::Mutex;

pub mod breakpoint;

pub use crate::arch::native::context::TrapFrame;
use crate::kprintln;

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    Continue,
    SingleStep,
    KillTask,
}

static ACTIVE: Mutex<bool> = Mutex::new(false);
pub(crate) static BKPT: Mutex<Option<(u64, u8)>> = Mutex::new(None);


pub fn clear_tf(tf: &mut TrapFrame) {
    tf.rflags &= !(1 << 8);
}

pub fn set_tf(tf: &mut TrapFrame) {
    tf.rflags |= 1 << 8;
}

pub fn setup() {
    if cfg!(debug_assertions) {
        kprintln!("[JOTUNHEIM] Waiting a debugger.");
        unsafe {
            core::arch::asm!("int3");
        }
        kprintln!("[JOTUNHEIM] Connected the debugger.");
    }
}

pub mod rsp {
    pub mod arch_x86_64;
    pub mod core;
    pub mod memory;
    pub mod transport;

    pub use super::Outcome;
    use super::{ACTIVE, TrapFrame};
    use crate::debug::rsp::arch_x86_64::X86_64Core;
    use crate::debug::rsp::core::RspServer;
    use crate::debug::rsp::memory::SectionMemory;
    use crate::debug::rsp::transport::Com2Transport;

    pub fn serve(tf: *mut TrapFrame) -> Outcome {
        {
            let mut active = ACTIVE.lock();
            if *active {
                return Outcome::Continue;
            }
            *active = true;
        }

        let t = Com2Transport;
        let a = X86_64Core;
        let m = SectionMemory;

        let out = RspServer::run(t, a, m, tf);

        *ACTIVE.lock() = false;
        out
    }
}
