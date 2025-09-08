#![allow(unused)]

use spin::Once;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

// Lives forever; fill rsp0/ISTs elsewhere if needed before init().
static TSS: TaskStateSegment = {
    let t = TaskStateSegment::new();
    t
};

// 'static singletons backed by spin::Once (Sync on no_std)
static GDT: Once<GlobalDescriptorTable> = Once::new();
static SELECTORS: Once<Selectors> = Once::new();

/// Build + load the GDT once; return selectors. Safe to call multiple times.
pub fn init() -> Selectors {
    if let Some(s) = SELECTORS.get() {
        return *s;
    }

    // Build temporary table, append entries (x86_64 = "0.15" uses `append`)
    let mut tmp = GlobalDescriptorTable::new();
    let code = tmp.append(Descriptor::kernel_code_segment());
    let data = tmp.append(Descriptor::kernel_data_segment());
    let tss = tmp.append(Descriptor::tss_segment(&TSS));

    // Move table into 'static storage, then load from that &'static ref
    let gdt_ref: &'static GlobalDescriptorTable = GDT.call_once(|| tmp);
    unsafe {
        gdt_ref.load();
    }

    let sels = Selectors { code, data, tss };
    let sels_ref: &'static Selectors = SELECTORS.call_once(|| sels);
    *sels_ref
}

// ---- Accessors used elsewhere ----
#[inline]
pub fn selectors() -> Selectors {
    *SELECTORS.get().expect("gdt::init() not called")
}
#[inline]
pub fn code_selector() -> SegmentSelector {
    SELECTORS.get().unwrap().code
}
#[inline]
pub fn data_selector() -> SegmentSelector {
    SELECTORS.get().unwrap().data
}
#[inline]
pub fn tss_selector() -> SegmentSelector {
    SELECTORS.get().unwrap().tss
}
