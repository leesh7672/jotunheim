// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use x86_64::instructions::interrupts::without_interrupts;

use crate::{
    arch::x86_64::tables::ISR,
    debug::{self, Outcome, TrapFrame, breakpoint},
    sched::exit_current,
};
use crate::faultsvc::{self, TrapFrameView};
use crate::arch::x86_64::tsc;
use x86_64::registers::control::Cr2;

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(tf: *mut TrapFrame) {
    // Log the general protection fault immediately. Do not print here.
    let tf_ref = unsafe { &*tf };
    let view = TrapFrameView {
        rip: tf_ref.rip,
        cs: tf_ref.cs as u64,
        rflags: tf_ref.rflags,
        rsp: tf_ref.rsp,
        ss: tf_ref.ss as u64,
    };
    let cr2 = 0;
    let tsc = tsc::rdtsc();
    faultsvc::log_from_isr(0x0D, tf_ref.err as u64, true, &view, cr2, tsc);

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
        // In non-debug builds, avoid printing inside an ISR. Just terminate the task.
        exit_current()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(tf: *mut TrapFrame) {
    // Log the page fault immediately. Do not print here.
    let tf_ref = unsafe { &*tf };
    let view = TrapFrameView {
        rip: tf_ref.rip,
        cs: tf_ref.cs as u64,
        rflags: tf_ref.rflags,
        rsp: tf_ref.rsp,
        ss: tf_ref.ss as u64,
    };
    let cr2 = Cr2::read().ok().map(|v| v.as_u64()).unwrap_or(0);
    let tsc = tsc::rdtsc();
    faultsvc::log_from_isr(0x0E, tf_ref.err as u64, true, &view, cr2, tsc);

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
        // In non-debug builds, avoid printing inside an ISR. Just terminate the task.
        exit_current()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(tf: *mut TrapFrame) {
    // Log the double fault immediately. Do not print here.
    let tf_ref = unsafe { &*tf };
    let view = TrapFrameView {
        rip: tf_ref.rip,
        cs: tf_ref.cs as u64,
        rflags: tf_ref.rflags,
        rsp: tf_ref.rsp,
        ss: tf_ref.ss as u64,
    };
    let cr2 = 0;
    let tsc = tsc::rdtsc();
    faultsvc::log_from_isr(0x08, tf_ref.err as u64, true, &view, cr2, tsc);

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
        // In non-debug builds, avoid printing inside an ISR. Just terminate the task.
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
