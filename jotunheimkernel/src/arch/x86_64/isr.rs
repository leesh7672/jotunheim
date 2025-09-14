use core::sync::atomic::{AtomicBool, Ordering};

use x86_64::instructions::interrupts::without_interrupts;

use crate::arch::x86_64::{apic, context, simd};
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
pub extern "C" fn isr_gp_rust(tf: &mut TrapFrame) -> ! {
    let dr6: u64;
    let dr7: u64;
    let cr0: u64;
    let cr4: u64;
    let rip = tf.rip;
    let rflags = tf.rflags;
    let cs = tf.cs;
    let ss = tf.ss;
    let rsp = tf.rsp;
    unsafe {
        core::arch::asm!("mov {}, dr6", out(reg) dr6);
        core::arch::asm!("mov {}, dr7", out(reg) dr7);
        core::arch::asm!("mov {}, cr0", out(reg) cr0);
        core::arch::asm!("mov {}, cr4", out(reg) cr4); // or split edx:eax
    }

    kprintln!(
        "[#GP] rip={:#018x} rsp={:#018x} rflags={:#018x}",
        rip,
        rsp,
        rflags
    );
    kprintln!(
        "[#GP] cs={:#06x} ss={:#06x} cr0={:#018x} cr4={:#018x}",
        cs,
        ss,
        cr0,
        cr4
    );
    kprintln!("[#GP] dr6={:#018x} dr7={:#018x}", dr6, dr7);
    kprintln!("[#GP]");
    loop {
        x86_64::instructions::hlt();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust(_vec: u64, _err: u64) {
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_vec: u64, err: u64, rip: u64) -> ! {
    use x86_64::registers::control::Cr2;
    let cr2 = Cr2::read().expect("CR2 read failed").as_u64();
    crate::arch::x86_64::mmio_map::log_va_mapping("PF-cr2", cr2, 0);

    kprintln!(
        "[#PF] err={:#018x} cr2={:#016x} rip={:#016x}",
        err,
        cr2,
        rip
    );
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

pub extern "C" fn isr_timer_rust() {
    apic::timer_isr_eoi_and_rearm_deadline();
    sched::tick()
}
