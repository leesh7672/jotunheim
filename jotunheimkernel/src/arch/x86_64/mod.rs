mod ap_trampoline;
pub mod apic;
pub mod context;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod isr;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod smp;
pub mod tsc;

use crate::kprintln;

pub fn init() {
    simd::init();
    gdt::init();
    kprintln!("[JOTUNHEIM] Loaded GDT.");
    idt::init();
    kprintln!("[JOTUNHEIM] Loaded IDT.");
    unsafe {
        ioapic::mask_all();
    }
    kprintln!("[JOTUNHEIM] Masked all IOAPIC.");
    apic::early_init();
    kprintln!("[JOTUNHEIM] Initialised APIC.");
    apic::open_all_irqs();
    kprintln!("[JOTUNHEIM] Opened all IRQs.");
    apic::start_timer_hz(1_000);
    kprintln!("[JOTUNHEIM] Tunned the timer.");
}
