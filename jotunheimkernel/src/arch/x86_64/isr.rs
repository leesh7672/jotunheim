use core::sync::atomic::{AtomicBool, Ordering};

use x86_64::instructions::hlt;
use x86_64::instructions::interrupts::without_interrupts;

use crate::arch::x86_64::{apic, context, simd};
use crate::debug::{Outcome, TrapFrame, breakpoint};
use crate::sched::PreemptPack;
use crate::{debug, kprintln, sched, serial};

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
pub extern "C" fn isr_gp_rust(_vec: u64, err: u64) -> ! {
    kprintln!("[#GP] err={:#018x}", err);
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
pub extern "C" fn isr_df_rust(tf: *mut crate::arch::x86_64::context::TrapFrame) -> ! {
    kprintln!("[#DF] double fault");
    loop {
        hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust() -> ! {
    kprintln!("[#UD] undefined");
    sched::exit_current();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_bp_rust(tf: *mut TrapFrame) {
    without_interrupts(|| {
        kprintln!("[JOTUNHEIM] Debugging.");
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
        kprintln!("[JOTUNHEIM] Debugging.");
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

pub extern "C" fn isr_timer_rust(hbase: u64) -> *const PreemptPack {
    apic::timer_isr_eoi_and_rearm_deadline();
    if let Some(b) = serial::com2_getc_nb() {
        if b == 0x03 {
            let rip = unsafe { *(hbase as *const u64) };
            let _ = breakpoint::insert(rip);
            return core::ptr::null();
        } else {
            sched::tick()
        }
    } else {
        sched::tick()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn preempt(pack: *const PreemptPack) {
    unsafe {
        let p = &*pack;
        if !p.prev_simd.is_null() {
            simd::save(p.prev_simd);
        }
        context::switch(p.prev_ctx, p.next_ctx);
    }
}
