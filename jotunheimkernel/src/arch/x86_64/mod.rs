mod ap_trampoline;
pub mod apic;
pub mod context;
pub mod ioapic;
pub mod tables;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod smp;
pub mod tsc;

use tables::gdt;
use tables::idt;
use crate::arch::x86_64::tables::isr;
use crate::kprintln;

pub fn init() {
    simd::init();
    unsafe {
        ioapic::mask_all();
    }
    kprintln!("[JOTUNHEIM] Masked all IOAPIC.");
    apic::early_init();
    isr::init();
    gdt::init();
    kprintln!("[JOTUNHEIM] Loaded GDT.");
    idt::init();
    kprintln!("[JOTUNHEIM] Loaded IDT.");
    kprintln!("[JOTUNHEIM] Initialised APIC.");
    apic::open_all_irqs();
    kprintln!("[JOTUNHEIM] Opened all IRQs.");
    apic::start_timer_hz(1_000);
    kprintln!("[JOTUNHEIM] Tunned the timer.");
}
