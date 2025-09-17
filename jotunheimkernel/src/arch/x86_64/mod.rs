mod ap_trampoline;
pub mod apic;
pub mod context;
pub mod ioapic;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod smp;
pub mod tables;
pub mod tsc;

use crate::arch::x86_64::apic::lapic_id;
use crate::arch::x86_64::tables::isr;
use crate::bootinfo::BootInfo;
use tables::gdt;
use tables::idt;

pub fn init(boot: &BootInfo) {
    simd::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::early_init();
    isr::init();
    idt::init(gdt::init());
    apic::paging(boot.hhdm_base);
    apic::open_all_irqs();
    apic::start_timer_hz(1000);
}
