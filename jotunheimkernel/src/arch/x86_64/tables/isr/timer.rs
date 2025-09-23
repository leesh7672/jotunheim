// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use crate::{
    arch::x86_64::{apic, tables::ISR}, debug::TrapFrame, sched
};

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(tf: &mut TrapFrame) {
    *tf = sched::tick(*tf);
    apic::eoi();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust() {
    apic::eoi()
}

unsafe extern "C" {
    unsafe fn isr_timer_stub();
    unsafe fn isr_spurious_stub();
}

pub fn init() {
    ISR::registrate(0x40, isr_timer_stub);
    ISR::registrate(0xFF, isr_spurious_stub);
}
