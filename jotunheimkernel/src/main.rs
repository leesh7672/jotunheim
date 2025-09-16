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
    arch::x86_64::{apic, smp::boot_all_aps},
    bootinfo::BootInfo,
    mem::reserved,
    sched::exit_current,
    util::zero_bss,
};

use core::panic::PanicInfo;
use x86_64::instructions::{
    hlt,
    interrupts::{self, without_interrupts},
};

use crate::arch::x86_64::{mmio_map, serial};

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

        kprintln!("[JOTUNHEIM] Enabled the memory management.");

        arch::x86_64::init();

        apic::paging();

        debug::setup();

        sched::init();
        kprintln!("[JOTUNHEIM] Prepared the scheduler.");

        sched::spawn(|| {
            kprintln!("[JOTUNHEIM] Started the main thread.");
            boot_all_aps(&boot);
            kprintln!("[JOTUNHEIM] Ends the main thread.");
            exit_current();
        });
    });
    interrupts::enable();
    loop {
        hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
