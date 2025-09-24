use x86_64::instructions::hlt;

// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use crate::{arch::x86_64::tables::Interrupt, kprintln, sched};

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust() -> ! {
    kprintln!("[#UD] undefined");
    sched::exit_current();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_bad_iret_frame() -> ! {
    kprintln!("A Bad IRET Frame.");
    loop {
        hlt();
    }
}

unsafe extern "C" {
    unsafe fn isr_ud_stub();
}

pub fn init() {
    Interrupt::register_without_stack(0x06, isr_ud_stub);
}
