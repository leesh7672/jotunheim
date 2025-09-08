use crate::arch::x86_64::{apic, gdt, idt, ioapic, mmio_map};
use crate::{println, sched};
use x86_64::instructions;
use x86_64::registers::control::Cr3;

fn log_cr3(tag: &str) {
    let (lvl4, _) = Cr3::read();
    println!("[PAGING] {tag}: CR3 = {:#x}", lvl4.start_address().as_u64());
}

pub fn init_arch() {
    gdt::init();
    idt::init();

    log_cr3("before early_map_mmio_for_apics");

    // IMPORTANT: set correct phys_offset for your current mapping.
    // If physical memory is identity-mapped (loader tables), phys_offset = 0.
    // If you have a phys window (e.g., 0xffff_8000_0000_0000), put it here.
    let phys_offset: u64 = 0;
    mmio_map::log_va_mapping("IOAPIC-before", 0xFEC0_0000, phys_offset);
    mmio_map::log_va_mapping("LAPIC-before", 0xFEE0_0000, phys_offset);

    // Do not touch IOAPIC/LAPIC here—only map PTEs.
    mmio_map::early_map_mmio_for_apics().unwrap_or_else(|e| {
        crate::println!("[MMIO_MAP][FATAL] {:?}", e);
        loop {
            instructions::hlt();
        }
    });

    log_cr3("after  early_map_mmio_for_apics");
    mmio_map::log_va_mapping("IOAPIC-after", 0xFEC0_0000, phys_offset);
    mmio_map::log_va_mapping("LAPIC-after", 0xFEE0_0000, phys_offset);

    // Now it should be safe to touch APICs
    apic::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::open_all_irqs();

    sched::init();
    apic::start_best_timer_hz(1_000);
    instructions::interrupts::enable();
}
