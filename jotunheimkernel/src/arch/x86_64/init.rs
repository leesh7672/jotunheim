use crate::arch::x86_64::{apic, gdt, idt, ioapic, mmio_map};
use crate::println;

use x86_64::instructions;
use x86_64::instructions::segmentation::{CS, Segment};

#[inline(always)]
unsafe fn load_kernel_data(sel: u16) {
    unsafe {
        core::arch::asm!(
            "mov ds, {0:x}",
            "mov es, {0:x}",
            "mov ss, {0:x}",
            in(reg) sel,
            options(nostack, preserves_flags),
        );
    }
}

#[inline(always)]
fn read_tr() -> u16 {
    let mut tr;
    unsafe { core::arch::asm!("str {0:x}", out(reg) tr) };
    tr
}

pub fn init_arch() {
    // 1) Build+load GDT, get selectors (no CS/TSS side effects here)
    let sel = gdt::init();

    // 2) Now do the segment switch and TSS load ONCE, here.
    unsafe {
        CS::set_reg(sel.code); // CS
        load_kernel_data(sel.data.0); // DS/ES/SS
        x86_64::instructions::tables::load_tss(sel.tss); // TR (LOWER TSS slot)
    }

    // 3) IDT after CS/DS/SS/TSS are valid
    idt::init();

    mmio_map::early_map_mmio_for_apics();
    // 4) APIC bring-up
    apic::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::open_all_irqs();

    // 5) Sanity print (should show TR == sel.tss)
    let cs_now = x86_64::registers::segmentation::CS::get_reg().0;
    let tr_now = read_tr();
    println!(
        "[SEGS] CS={:#06x} TR={:#06x} (expect {:#06x})",
        cs_now, tr_now, sel.tss.0
    );

    // 6) Enable interrupts + start timer
    instructions::interrupts::enable();
    apic::start_best_timer_hz(1_000);
}
