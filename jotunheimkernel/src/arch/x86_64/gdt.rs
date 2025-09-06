use spin::Once;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
static TSS: Once<TaskStateSegment> = Once::new();

struct Selectors {
    _code: SegmentSelector,
    tss: SegmentSelector,
}

// Prefix with underscore to silence "unused" until IST wiring
const _DOUBLE_FAULT_IST_INDEX: u16 = 0;

fn build_tss() -> TaskStateSegment {
    let tss = TaskStateSegment::new();
    // TODO: allocate IST stack and set:
    // tss.interrupt_stack_table[_DOUBLE_FAULT_IST_INDEX as usize] = VirtAddr::from_ptr(stack_top);
    tss
}

pub fn init() {
    // Initialize TSS once and get a stable &'static reference (no static mut refs)
    TSS.call_once(build_tss);
    let tss_ref: &'static TaskStateSegment = TSS.get().unwrap();

    // Build GDT once
    GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        // x86_64 0.15.x uses `append`
        let code = gdt.append(Descriptor::kernel_code_segment());
        let tss = gdt.append(Descriptor::tss_segment(tss_ref));
        (gdt, Selectors { _code: code, tss })
    });

    let (gdt, sel) = GDT.get().unwrap();
    gdt.load();

    // load TSS
    unsafe {
        x86_64::instructions::tables::load_tss(sel.tss);
    }
}
