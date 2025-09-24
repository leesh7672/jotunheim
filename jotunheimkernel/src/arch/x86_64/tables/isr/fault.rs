// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use x86_64::instructions::interrupts::without_interrupts;

use crate::{
    arch::x86_64::tables::Interrupt,
    debug::{self, Outcome, TrapFrame, breakpoint},
    kprintln,
    sched::exit_current,
};

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(tf: *mut TrapFrame) {
    {
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
    }
    if cfg!(debug_assertions) {
        without_interrupts(|| unsafe{
            breakpoint::insert((*tf).rip);
        })
    } else {
        exit_current();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(tf: *mut TrapFrame) {
    {
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
    }
    if cfg!(debug_assertions) {
        without_interrupts(|| unsafe{
            breakpoint::insert((*tf).rip);
        })
    } else {
        exit_current();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(tf: *mut TrapFrame) {
    {
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
    }
    if cfg!(debug_assertions) {
        without_interrupts(|| unsafe{
            breakpoint::insert((*tf).rip);
        })
    } else {
        exit_current();
    }
}
unsafe extern "C" {
    unsafe fn isr_gp_stub();
    unsafe fn isr_pf_stub();
    unsafe fn isr_df_stub();
}
pub fn init() {
    Interrupt::register_with_stack(0x0D, isr_gp_stub, 1);
    Interrupt::register_with_stack(0x0E, isr_pf_stub, 1);
    Interrupt::register_with_stack(0x08, isr_df_stub, 2);
}
