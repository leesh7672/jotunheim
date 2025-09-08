#![no_std]
#![no_main]

mod bootinfo;
mod util;
mod mem {
    pub mod bump;
    pub mod mapper;
    pub mod simple_alloc;
}
mod arch {
    pub mod x86_64 {
        pub mod apic;
        pub mod gdt;
        pub mod idt;
        pub mod init;
        pub mod ioapic;
        pub mod mmio_map;
        pub mod serial;
        pub mod split_huge;
        pub mod tsc;
    }
}

use arch::x86_64::init;
use bootinfo::BootInfo;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::{mmio_map::map_identity_uc, split_huge::split_huge_2m};
use crate::mem::mapper::active_offset_mapper;
use crate::mem::simple_alloc::TinyBump;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start() -> ! {
    x86_64::instructions::interrupts::disable();
    unsafe {
        crate::arch::x86_64::serial::init_com1(115_200);
    }
    println!("[JOTUNHEIM] Kernel starts.");

    crate::arch::x86_64::init::init_arch();

    crate::arch::x86_64::apic::snapshot_debug();

    loop {
        core::hint::spin_loop()
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
