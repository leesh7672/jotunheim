// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![no_std]
#![no_main]

mod acpi;
mod arch;
mod bootinfo;
mod debug;
mod mem;
mod sched;
mod util;

extern crate alloc;

use crate::{
    arch::native::smp::boot_all_aps, bootinfo::BootInfo, mem::reserved, sched::exec, util::zero_bss,
};

use core::panic::PanicInfo;
use x86_64::instructions::{
    hlt,
    interrupts::{self, without_interrupts},
};

use crate::arch::native::{self, mmio_map, serial};

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot: &BootInfo) -> ! {
    without_interrupts(|| {
        unsafe {
            zero_bss();
            serial::init_com1(115_200);
            serial::init_com2(115_200);
        }
        kprintln!("[JOTUNHEIM] Loaded the kernel.");

        reserved::init(&boot);
        mem::init(&boot);
        mem::seed_usable_from_mmap(&boot);
        mem::init_heap();
        mmio_map::enforce_apic_mmio_flags();
        native::init(&boot);
        sched::init();
        sched::spawn(|| {
            exec::init();
            kprintln!("[JOTUNHEIM] Started the kernel main thread.");
            boot_all_aps(boot);
            kprintln!("[JOTUNHEIM] Ended the kernel main thread.");
        });
        debug::setup();
        sched::enter();
    });
    interrupts::enable();
    loop {
        hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n*** KERNEL PANIC ***\n{}", info);
    if cfg!(debug_assertions) {
        interrupts::int3();
    }
    loop {
        x86_64::instructions::hlt();
    }
}
