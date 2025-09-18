use x86_64::instructions::interrupts::without_interrupts;

use crate::{
    arch::x86_64::tables::ISR,
    debug::{self, Outcome, TrapFrame, breakpoint},
    kprintln,
    sched::exit_current,
};

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(tf: *mut TrapFrame) {
    if cfg!(debug_assertions) {
        without_interrupts(|| {
            let last_hit = {
                let t = unsafe { &mut *tf };
                breakpoint::on_breakpoint_enter(&mut t.rip)
            };

            match debug::rsp::serve(tf) {
                Outcome::Continue => {
                    breakpoint::on_resume_continue(last_hit);
                }
                Outcome::SingleStep => {
                    breakpoint::on_resume_step(last_hit);
                }
                Outcome::KillTask => exit_current(),
            }
        })
    } else {
        let tf = unsafe { &*tf };
        kprintln!(
            "[#GP] vec={} err={:#x}\n  rip={:#018x} rsp={:#018x} rflags={:#018x}\n  cs={:#06x} ss={:#06x}",
            tf.vec,
            tf.err,
            tf.rip,
            tf.rsp,
            tf.rflags,
            tf.cs as u16,
            tf.ss as u16
        );
        exit_current()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(tf: *mut TrapFrame) {
    if cfg!(debug_assertions) {
        without_interrupts(|| {
            let last_hit = {
                let t = unsafe { &mut *tf };
                breakpoint::on_breakpoint_enter(&mut t.rip)
            };

            match debug::rsp::serve(tf) {
                Outcome::Continue => {
                    breakpoint::on_resume_continue(last_hit);
                }
                Outcome::SingleStep => {
                    breakpoint::on_resume_step(last_hit);
                }
                Outcome::KillTask => exit_current(),
            }
        })
    } else {
        let tf = unsafe { &*tf };
        kprintln!(
            "[#PF] vec={} err={:#x}\n  rip={:#018x} rsp={:#018x} rflags={:#018x}\n  cs={:#06x} ss={:#06x}",
            tf.vec,
            tf.err,
            tf.rip,
            tf.rsp,
            tf.rflags,
            tf.cs as u16,
            tf.ss as u16
        );
        exit_current()
    }
}

#[cfg(debug_assertions)]
#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(tf: *mut TrapFrame) {
    if cfg!(debug_assertions) {
        without_interrupts(|| {
            let last_hit = {
                let t = unsafe { &mut *tf };
                breakpoint::on_breakpoint_enter(&mut t.rip)
            };

            match debug::rsp::serve(tf) {
                Outcome::Continue => {
                    breakpoint::on_resume_continue(last_hit);
                }
                Outcome::SingleStep => {
                    breakpoint::on_resume_step(last_hit);
                }
                Outcome::KillTask => exit_current(),
            }
        })
    } else {
        let tf = unsafe { &*tf };
        kprintln!(
            "[#DF] vec={} err={:#x}\n  rip={:#018x} rsp={:#018x} rflags={:#018x}\n  cs={:#06x} ss={:#06x}",
            tf.vec,
            tf.err,
            tf.rip,
            tf.rsp,
            tf.rflags,
            tf.cs as u16,
            tf.ss as u16
        );
        exit_current()
    }
}
unsafe extern "C" {
    unsafe fn isr_gp_stub();
    unsafe fn isr_pf_stub();
    unsafe fn isr_df_stub();
}
pub fn init() {
    ISR::registrate(0x0D, isr_gp_stub);
    ISR::registrate_without_stack(0x0E, isr_pf_stub);
    ISR::registrate(0x08, isr_df_stub);
}
