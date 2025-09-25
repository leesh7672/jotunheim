// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
mod ap_trampoline;
pub mod apic;
pub mod trapframe;
pub mod ioapic;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod smp;
pub mod tables;
pub mod tsc;
use crate::arch::x86_64::tables::isr;
use crate::bootinfo::BootInfo;
use tables::gdt;
use tables::idt;

pub fn init(boot: &BootInfo) {
    simd::init();
    apic::early_init();
    isr::init();
    gdt::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::paging(boot.hhdm_base);
}
