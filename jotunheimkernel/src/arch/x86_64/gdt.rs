// arch/x86_64/gdt.rs
#![allow(clippy::missing_safety_doc)]

use core::arch::asm;
use spin::Once;

use x86_64::VirtAddr;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub struct Selectors {
    pub code: SegmentSelector,
    pub tss: SegmentSelector,
}

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
const IST1_STACK_SIZE: usize = 16 * 1024;
#[repr(transparent)]
pub struct IstStack(pub [u8; IST1_STACK_SIZE]);

// A dedicated IST1 stack for double-faults (or other IST use).
// 16-byte align to keep SysV rules happy when we enter Rust.
#[repr(align(16))]
struct AlignedStack([u8; 4096 * 5]);
static mut IST1_STACK: AlignedStack = AlignedStack([0u8; 4096 * 5]);

fn build_tss() -> TaskStateSegment {
    let mut t = TaskStateSegment::new();

    // Compute top using raw pointer + const size (no shared refs)
    let top = unsafe {
        let base = core::ptr::addr_of!(IST1_STACK.0) as u64;
        base + IST1_STACK_SIZE as u64
    };
    t.interrupt_stack_table[0] = VirtAddr::new(top);

    t
}

unsafe fn reload_cs_far(sel: SegmentSelector) {
    // Far return to reload CS with the new code selector
    asm!(
        "push {cs}",
        "lea rax, [rip + 2f]",
        "push rax",
        "retfq",
        "2:",
        cs = in(reg) u64::from(sel.0),
        out("rax") _,
        options(nostack)
    );
}

pub fn init() {
    // Build TSS exactly once
    let tss_ref = TSS.call_once(build_tss);

    // Build GDT exactly once
    let (gdt, sel) = GDT.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        let code = g.append(Descriptor::kernel_code_segment());
        let tss = g.append(Descriptor::tss_segment(tss_ref));
        (g, Selectors { code, tss })
    });

    // Load GDT and TSS
    gdt.load();
    unsafe {
        x86_64::instructions::tables::load_tss(sel.tss);
        reload_cs_far(sel.code);
    }
}
