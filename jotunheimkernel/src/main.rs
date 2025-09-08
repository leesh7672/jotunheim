#![no_std]
#![no_main]

mod bootinfo;
mod sched;
mod util;
mod mem {
    pub mod bump;
    pub mod mapper;
    pub mod simple_alloc;
}
mod arch {
    pub mod x86_64 {
        pub mod apic;
        pub mod context;
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

static mut DEMO_STACK: [u8; 16 * 1024] = [0; 16 * 1024];

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

    let ptr = core::ptr::addr_of_mut!(DEMO_STACK) as *mut u8;
    const DEMO_STACK_LEN: usize = 16 * 1024;
    crate::sched::spawn_kthread(kthread_demo, 0, ptr, DEMO_STACK_LEN);

    x86_64::instructions::interrupts::enable();
    crate::sched::yield_now();
    loop {
        x86_64::instructions::hlt();
    }
}

extern "C" fn kthread_demo(_arg: usize) -> ! {
    loop {
        println!("[Threading]");
        crate::sched::yield_now();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
