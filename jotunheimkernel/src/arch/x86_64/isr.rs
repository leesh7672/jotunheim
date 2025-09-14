use core::arch::asm;
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
pub extern "C" fn isr_gp_rust(tf: *mut TrapFrame) -> ! {
    let tf = unsafe { &*tf };
    kprintln!(
        "[#GP] vec={} err={:#x}\n  rip={:#018x} rsp={:#018x} rflags={:#018x}\n  cs={:#06x} ss={:#06x}",
        tf.vec, tf.err, tf.rip, tf.rsp, tf.rflags, tf.cs as u16, tf.ss as u16
    );
    loop { x86_64::instructions::hlt(); }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust() {
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(tf: *mut TrapFrame) -> ! {
    let tf = unsafe { &*tf };
    let cr2: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2);
    }

    // walk page tables (adapt PHYS_TO_VIRT_OFFSET/HHDM as in your mapper)
    unsafe fn read64(p: u64) -> u64 {
        (p as *const u64).read_volatile()
    }

    let va = cr2;
    let pml4_idx = ((va >> 39) & 0x1ff) as usize;
    let pdpt_idx = ((va >> 30) & 0x1ff) as usize;
    let pd_idx = ((va >> 21) & 0x1ff) as usize;
    let pt_idx = ((va >> 12) & 0x1ff) as usize;

    let cr3: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) cr3);
    }
    let pml4 = cr3 & !0xfff;

    let pml4e = unsafe { read64(pml4 + 8 * (pml4_idx as u64)) };
    let pdpte = if pml4e & 1 != 0 {
        let pdpt = pml4e & !0xfff;
        unsafe { read64(pdpt + 8 * (pdpt_idx as u64)) }
    } else {
        0
    };
    let pde = if pdpte & 1 != 0 && (pdpte & (1 << 7)) == 0 {
        let pd = pdpte & !0xfff;
        unsafe { read64(pd + 8 * (pd_idx as u64)) }
    } else {
        0
    };
    let pte = if pde & 1 != 0 && (pde & (1 << 7)) == 0 {
        let pt = pde & !0xfff;
        unsafe { read64(pt + 8 * (pt_idx as u64)) }
    } else {
        0
    };

    kprintln!("[#PF] cr2={:#018x} err={:#x}", va, tf.err);
    kprintln!(
        "      rip={:#018x} rsp={:#018x} rflags={:#018x}",
        tf.rip,
        tf.rsp,
        tf.rflags
    );
    kprintln!(
        "      pml4e={:#x} pdpte={:#x} pde={:#x} pte={:#x}",
        pml4e,
        pdpte,
        pde,
        pte
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
pub extern "C" fn isr_timer_rust(tf: &mut TrapFrame) {
    apic::timer_isr_eoi_and_rearm_deadline();
    sched::tick()
}
