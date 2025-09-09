pub mod apic;
pub mod context;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod tsc;

use crate::sched;
use x86_64::instructions;

pub fn init() {
    simd::enable_sse_avx();
    gdt::init();
    idt::init();
    mmio_map::early_map_mmio_for_apics();
    apic::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::open_all_irqs();

    sched::init();

    apic::start_timer_periodic_hz(1_000);
    instructions::interrupts::enable();
}
