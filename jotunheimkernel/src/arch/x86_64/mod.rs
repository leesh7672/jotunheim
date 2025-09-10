pub mod apic;
pub mod context;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod tsc;

use crate::bootinfo::BootInfo;
use crate::{mem, sched};

pub fn init(boot: &BootInfo) {
    simd::enable_sse_avx();
    gdt::init();
    idt::init();
    mmio_map::enforce_apic_mmio_flags(boot.hhdm_base);
    mem::init(boot);
    mem::init_heap();
    sched::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::init();
    apic::open_all_irqs();
    apic::start_timer_hz(1_00);
}
