#![allow(dead_code)]

use core::arch::asm;
use core::ptr::addr_of;

use spin::Once;
use x86_64::instructions::segmentation::Segment; // brings CS::set_reg into scope
use x86_64::registers::segmentation::CS;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::{VirtAddr, instructions};

// One IST stack for DF etc. (16 KiB)
#[repr(align(16))]
struct Aligned([u8; 4096 * 4]);
static mut IST1_STACK: Aligned = Aligned([0u8; 4096 * 4]);

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub tss: SegmentSelector,
}

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

#[inline(always)]
fn ist1_top_u64() -> u64 {
    const IST1_STACK_BYTES: usize = 4096 * 4;
    let base = unsafe { addr_of!(IST1_STACK) as *const u8 as u64 };
    base + IST1_STACK_BYTES as u64
}

fn build_tss() -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();
    let top = ist1_top_u64();
    tss.interrupt_stack_table[0] = VirtAddr::new(top);
    tss
}

unsafe fn reload_cs_far(sel: SegmentSelector) {
    // Use the trait method if available (cleaner & avoids inline asm)
    CS::set_reg(sel);
}

pub fn init() {
    // Build TSS once and keep it forever.
    let tss_ref: &TaskStateSegment = TSS.call_once(build_tss);

    // Build GDT once and keep it forever.
    let (gdt, sel) = GDT.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        let code = g.append(Descriptor::kernel_code_segment());
        let tss = g.append(Descriptor::tss_segment(tss_ref));
        (g, Selectors { code, tss })
    });

    // Load GDT + TSS, then reload CS to the kernel code selector.
    gdt.load();
    unsafe {
        instructions::tables::load_tss(sel.tss);
        reload_cs_far(sel.code);
    }
}
