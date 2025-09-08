use crate::arch::x86_64::{apic, gdt, idt, ioapic, mmio_map};
use crate::sched;

use x86_64::instructions;

pub fn init_arch() {
    gdt::init();
    idt::init();

    mmio_map::early_map_mmio_for_apics();
    apic::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::open_all_irqs();

    instructions::interrupts::enable();
    sched::init();
    apic::start_best_timer_hz(1_000);
}
