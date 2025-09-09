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
    // Do not touch IOAPIC/LAPIC here—only map PTEs.
    let _ = mmio_map::early_map_mmio_for_apics();
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
