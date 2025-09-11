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
use crate::{debug, mem, sched};

pub fn init() {
    simd::enable_sse_avx();
    gdt::init();
    idt::init();
    sched::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::init();
    apic::open_all_irqs();
    apic::start_timer_hz(1_000);
}
