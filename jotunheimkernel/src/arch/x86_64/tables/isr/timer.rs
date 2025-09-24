// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project

use crate::{
    arch::x86_64::{apic, tables::Interrupt},
    debug::TrapFrame,
    sched,
};

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(tf: *mut TrapFrame) {
    unsafe { *tf = sched::tick(*tf) };
    apic::eoi();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust() {}

unsafe extern "C" {
    unsafe fn isr_timer_stub();
    unsafe fn isr_spurious_stub();
}

pub fn init() {
    Interrupt::register_without_stack(0x40, isr_timer_stub);
    Interrupt::register_without_stack(0xFF, isr_spurious_stub);
}
