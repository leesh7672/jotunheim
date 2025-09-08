// was: use core::sync::OnceLock;  OR core::cell::OnceCell
use spin::Once;

use x86_64::registers::segmentation::{Segment, CS};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub struct Selectors {
    pub code: SegmentSelector,
    pub tss: SegmentSelector,
}

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

fn build_tss() -> TaskStateSegment {
    let mut t = TaskStateSegment::new();
    // set IST stacks here if you want (DF/NMI, etc.)
    t
}

pub fn init() {
    let tss_ref = TSS.call_once(build_tss);

    let (gdt, sel) = GDT.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        let code = g.append(Descriptor::kernel_code_segment());
        let tss = g.append(Descriptor::tss_segment(tss_ref));
        (g, Selectors { code, tss })
    });

    gdt.load();
    unsafe { CS::set_reg(sel.code) };
    unsafe {
        x86_64::instructions::tables::load_tss(sel.tss);
    }
}
