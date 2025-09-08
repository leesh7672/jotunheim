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

use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::idt;

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

    let mut last = 0;
    loop {
        let t = idt::TICKS.load(Ordering::Relaxed);
        if t.wrapping_sub(last) >= 500 {
            // ~0.5s at 1 kHz
            last = t;
            println!("[TIMER] ticks={}", t);
        }
        core::hint::spin_loop(); // or `unsafe { core::arch::asm!("hlt"); }` if you prefer
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
