use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use x86_64::instructions::interrupts::without_interrupts;
use x86_64::registers::control::Cr2;

use crate::arch::x86_64::apic;
use crate::debug::{Outcome, TrapFrame, breakpoint};
use crate::{debug, kprintln, sched};

static THROTTLED_ONCE: AtomicBool = AtomicBool::new(false);

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    if !THROTTLED_ONCE.swap(true, Ordering::Relaxed) {
        kprintln!("[INT] default vec={:#04x} err={:#018x}", vec, err);
    }
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(tf: *mut TrapFrame) -> ! {
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
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust() {
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_tf: *mut TrapFrame) -> ! {
    kprintln!("[PF]");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(tf: *mut TrapFrame) {
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
            Outcome::KillTask => crate::sched::exit_current(),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust() -> ! {
    kprintln!("[#UD] undefined");
    sched::exit_current();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_bp_rust(tf: *mut TrapFrame) {
    without_interrupts(|| {
        let last_hit = {
            let t = unsafe { &mut *tf };
            breakpoint::on_breakpoint_enter(&mut t.rip)
        };

        // hand control to the gdb stub (RSP)
        match debug::rsp::serve(tf) {
            Outcome::Continue => {
                // re-arm the bp if GDB continued
                breakpoint::on_resume_continue(last_hit);
            }
            Outcome::SingleStep => {
                // defer re-arming until the #DB we’ll get after this step
                breakpoint::on_resume_step(last_hit);
            }
            Outcome::KillTask => crate::sched::exit_current(),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_db_rust(tf: *mut TrapFrame) {
    without_interrupts(|| {
        let last_hit = {
            let t = unsafe { &mut *tf };
            breakpoint::on_breakpoint_enter(&mut t.rip)
        };

        // hand control to the gdb stub (RSP)
        match debug::rsp::serve(tf) {
            Outcome::Continue => {
                // re-arm the bp if GDB continued
                breakpoint::on_resume_continue(last_hit);
            }
            Outcome::SingleStep => {
                // defer re-arming until the #DB we’ll get after this step
                breakpoint::on_resume_step(last_hit);
            }
            Outcome::KillTask => crate::sched::exit_current(),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(tf: &mut TrapFrame) {
    apic::timer_isr_eoi_and_rearm_deadline();
    sched::tick()
}
