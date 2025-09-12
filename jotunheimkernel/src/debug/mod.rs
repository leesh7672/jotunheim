#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::identity_op)]

use spin::Mutex;

pub mod breakpoint;

pub use crate::arch::x86_64::context::TrapFrame;

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    Continue,
    SingleStep,
    KillTask,
}

static ACTIVE: Mutex<bool> = Mutex::new(false);
pub(crate) static BKPT: Mutex<Option<(u64, u8)>> = Mutex::new(None);

#[inline(always)]
pub fn clear_tf(tf: &mut TrapFrame) {
    tf.rflags &= !(1 << 8);
}
#[inline(always)]
pub fn set_tf(tf: &mut TrapFrame) {
    tf.rflags |= 1 << 8;
}

pub fn setup() {
    unsafe {
        core::arch::asm!("int3");
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

    /// Keep the same signature your ISRs call.
    #[inline(never)]
    pub fn serve(tf: *mut TrapFrame) -> Outcome {
        x86_64::instructions::interrupts::disable();
        {
            let mut active = ACTIVE.lock();
            if *active {
                return Outcome::Continue;
            }
            *active = true;
        }
        // Compose the generic server from zero-sized types (no heap).
        let t = Com2Transport;
        let a = X86_64Core;
        let m = SectionMemory;

        let out = RspServer::run(t, a, m, tf);

        *ACTIVE.lock() = false;
        out
    }
}
