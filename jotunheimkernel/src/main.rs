#![no_std]
#![no_main]

mod arch;
mod bootinfo;
mod debug;
mod mem;
mod sched;
mod util;

use crate::bootinfo::BootInfo;
use core::panic::PanicInfo;
use x86_64::instructions::{
    hlt,
    interrupts::{self, without_interrupts},
};

use crate::arch::x86_64::{apic, mmio_map, serial, simd};

static mut MAIN_STACK: [u8; 16 * 1024] = [0; 16 * 1024];
const STACK_LEN: usize = 16 * 1024;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot: &BootInfo) -> ! {
    simd::enable_sse_avx();
    without_interrupts(|| {
        unsafe {
            serial::init_com1(115_200);
            serial::init_com2(115_200);
        }
        kprintln!("[JOTUNHEIM] Loaded the kernel.");
        arch::x86_64::init();

        sched::init();
        kprintln!("[JOTUNHEIM] Prepared the scheduler.");

        debug::setup();

        let boot_ptr = boot as *const BootInfo as usize;
        let main_stack_ptr = core::ptr::addr_of_mut!(MAIN_STACK) as *mut u8;
        sched::spawn_kthread(main_thread, boot_ptr, main_stack_ptr, STACK_LEN);
    });
    interrupts::enable();
    loop {
        hlt();
    }
}

extern "C" fn main_thread(arg: usize) {
    let boot_ptr: *const _ = arg as *const BootInfo;
    let boot: BootInfo = unsafe { *(boot_ptr) };
    mem::init(&boot);
    mmio_map::enforce_apic_mmio_flags(boot.hhdm_base);
    mem::init_heap();

    kprintln!("[JOTUNHEIM] The Main thread is working.");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
