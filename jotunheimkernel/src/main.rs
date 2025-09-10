#![no_std]
#![no_main]

mod bootinfo;
mod mem;
mod sched;
mod util;
mod arch {
    pub mod x86_64;
}

use crate::bootinfo::BootInfo;
use core::panic::PanicInfo;
use x86_64::instructions::{hlt, interrupts};

use crate::arch::x86_64::serial;

static mut STACK: [u8; 16 * 1024] = [0; 16 * 1024];
const STACK_LEN: usize = 16 * 1024;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot: &BootInfo) -> ! {
    interrupts::disable();
    unsafe {
        serial::init_com1(115_200);
    }
    println!("[JOTUNHEIM] The Kernel starts.");

    arch::x86_64::init(boot);

    let ptr = core::ptr::addr_of_mut!(STACK) as *mut u8;
    sched::spawn_kthread(main_thread, 0, ptr, STACK_LEN);
    interrupts::enable();
    loop {
        hlt();
    }
}

extern "C" fn main_thread(_arg: usize) {
    println!("[JOTUNHEIM] The Main thread is working.");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
