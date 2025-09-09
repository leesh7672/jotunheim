pub mod apic;
pub mod context;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod mmio_map;
pub mod serial;
pub mod simd;
pub mod tsc;

use crate::allocator;
use crate::bootinfo::BootInfo;
use crate::println;
use crate::sched;

use x86_64::instructions;

pub fn init(boot: &BootInfo) {
    allocator::early_init_from_bootinfo(boot);
    simd::enable_sse_avx();
    gdt::init();
    idt::init();
    mmio_map::early_map_mmio_for_apics();
    sched::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::init();
    apic::open_all_irqs();
    apic::start_timer_hz(1_000);
}
