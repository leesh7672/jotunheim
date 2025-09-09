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
        pub mod tsc;
    }
}

use core::panic::PanicInfo;
use x86_64::instructions::interrupts;

use crate::arch::x86_64::{apic, init, serial};

static mut DEMO_STACK: [u8; 16 * 1024] = [0; 16 * 1024];
static mut DEMO_STACK2: [u8; 16 * 1024] = [0; 16 * 1024];
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start() -> ! {
    interrupts::disable();
    unsafe {
        serial::init_com1(115_200);
    }
    println!("[JOTUNHEIM] Kernel starts.");

    init::init_arch();
    apic::snapshot_debug();

    let ptr = core::ptr::addr_of_mut!(DEMO_STACK) as *mut u8;
    const DEMO_STACK_LEN: usize = 16 * 1024;
    sched::spawn_kthread(kthread_demo, 0, ptr, DEMO_STACK_LEN);

    let ptr2 = core::ptr::addr_of_mut!(DEMO_STACK2) as *mut u8;
    sched::spawn_kthread(kthread_demo2, 0, ptr2, DEMO_STACK_LEN);

    interrupts::enable();

    loop {
        sched::yield_now();
    }
}
extern "C" fn kthread_demo(_arg: usize) -> ! {
    let mut a = 0u128;
    loop {
        println!("[Threading 1] {a}");
        a += 1;
    }
}
extern "C" fn kthread_demo2(_arg: usize) -> ! {
    let mut b = 0u128;
    loop {
        println!("[Threading 2] {b}");
        b += 1;
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
