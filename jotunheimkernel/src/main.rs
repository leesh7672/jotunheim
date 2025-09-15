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
    arch::x86_64::{apic, smp::boot_all_aps}, bootinfo::BootInfo, mem::reserved, sched::exit_current,
    util::zero_bss,
};

use core::panic::PanicInfo;
use x86_64::instructions::{
    hlt,
    interrupts::{self, without_interrupts},
};

use crate::arch::x86_64::{mmio_map, serial};

static mut MAIN_STACK: [u8; 32 * 1024] = [0; 32 * 1024];
const STACK_LEN: usize = 32 * 1024;

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
        arch::x86_64::init();

        reserved::init(&boot);
        mem::init(&boot);
        mem::seed_usable_from_mmap(&boot);
        mem::init_heap();
        mmio_map::enforce_apic_mmio_flags();
        apic::paging();
        kprintln!("[JOTUNHEIM] Enabled the memory management.");

        debug::setup();

        sched::init();
        kprintln!("[JOTUNHEIM] Prepared the scheduler.");

        let boot_ptr = boot as *const BootInfo as usize;
        let main_stack_ptr = core::ptr::addr_of_mut!(MAIN_STACK) as *mut u8;
        sched::spawn_kthread(main_thread, boot_ptr, main_stack_ptr, STACK_LEN);
    });
    interrupts::enable();
    loop {
        hlt();
    }
}

extern "C" fn main_thread(arg: usize) -> ! {
    kprintln!("[JOTUNHEIM] Started the main thread.");
    let boot_ptr: *const _ = arg as *const BootInfo;
    let boot: BootInfo = unsafe { *(boot_ptr) };

    boot_all_aps(&boot);
    kprintln!("[JOTUNHEIM] Ends the main thread.");
    exit_current();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
