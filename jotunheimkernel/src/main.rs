#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod bootinfo;
mod util;
mod mem {
    pub mod bump;
}
mod arch {
    pub mod x86_64 {
        pub mod apic;
        pub mod gdt;
        pub mod idt;
        pub mod serial;
        pub mod tsc;
    }
}

use arch::x86_64::{apic, gdt, idt, serial};
use bootinfo::BootInfo;
use core::panic::PanicInfo;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot_info_ptr: *const BootInfo) -> ! {
    let boot = unsafe { &*boot_info_ptr };

    unsafe {
        serial::init_com1(115_200);
    }
    println!("[JOTUNHEIM] Kernel starts.");

    gdt::init();
    idt::init();
    crate::println!("[JOTUNHEIM] GDT/IDT is initialised.");

    apic::init();
    let _ = apic::start_timer_hz(1000);

    x86_64::instructions::interrupts::enable();

    crate::println!("[JOTUNHEIM] Interrupts are enabled.");

    loop {
        x86_64::instructions::hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
