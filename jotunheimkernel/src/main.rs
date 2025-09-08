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
        pub mod ioapic;
        pub mod mmio_map;
        pub mod serial;
        pub mod split_huge;
        pub mod tsc;
    }
}

use arch::x86_64::{apic, gdt, idt, ioapic, serial};
use bootinfo::BootInfo;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::{mmio_map::map_identity_uc, split_huge::split_huge_2m};
use crate::mem::mapper::active_offset_mapper;
use crate::mem::simple_alloc::TinyBump;

static mut ALLOC: TinyBump = TinyBump::new(0x0030_0000, 0x0031_0000);

pub fn early_map_mmio_for_apics() {
    // 1) active mapper (identity offset here; adjust if needed)
    let mut mapper = unsafe { active_offset_mapper(0) };

    // 2) get a RAW pointer to the static (allowed under the lint)
    let alloc: *mut TinyBump = &raw mut ALLOC;

    // 3) use it only within a very small unsafe region
    unsafe {
        // split any covering 2MiB pages once
        let _ = split_huge_2m(&mut mapper, &mut *alloc, 0xFEC0_0000);
        let _ = split_huge_2m(&mut mapper, &mut *alloc, 0xFEE0_0000);

        // map UC pages you need
        let _ = map_identity_uc(&mut mapper, &mut *alloc, 0xFEC0_0000); // IOAPIC
        let _ = map_identity_uc(&mut mapper, &mut *alloc, 0xFEE0_0000); // LAPIC
    }
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot_info_ptr: *const BootInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    unsafe {
        serial::init_com1(115_200);
    }
    println!("[JOTUNHEIM] Kernel starts.");

    gdt::init();
    idt::init();
    early_map_mmio_for_apics();
    println!("[JOTUNHEIM] GDT/IDT is initialised.");

    apic::init();
    println!("[JOTUNHEIM] APIC is initialised.");

    apic::open_all_irqs();

    unsafe {
        ioapic::mask_all();
    }

    println!("[JOTUNHEIM] IOAPIC is masked all.");

    apic::open_all_irqs();
    x86_64::instructions::interrupts::enable();
    println!("[JOTUNHEIM] Interrupts are enabled.");

    apic::start_best_timer_hz(1_000);
    println!("[JOTUNHEIM] Timer starts.");

    apic::snapshot_debug();

    let mut last = 0u64;
    loop {
        let cur = idt::TICKS.load(Ordering::Relaxed);
        if last != cur {
            last = cur;
            println!("[tick] {}", cur);
        }
        x86_64::instructions::interrupts::enable_and_hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n*** KERNEL PANIC ***\n{}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
