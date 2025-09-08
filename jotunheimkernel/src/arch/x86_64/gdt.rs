use spin::Once;
use x86_64::VirtAddr;

use x86_64::instructions::segmentation::{CS, Segment};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

const IST_DF: usize = 1;
const IST_NMI: usize = 2;
const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(align(16))]
struct Aligned([u8; IST_STACK_SIZE]);

static DF_STACK: Once<Aligned> = Once::new();
static NMI_STACK: Once<Aligned> = Once::new();

static TSS: Once<TaskStateSegment> = Once::new();
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

pub struct Selectors {
    pub code: SegmentSelector,
    pub tss: SegmentSelector,
}

fn build_tss() -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();

    let df = DF_STACK.call_once(|| Aligned([0u8; IST_STACK_SIZE]));
    let nmi = NMI_STACK.call_once(|| Aligned([0u8; IST_STACK_SIZE]));

    let df_top = VirtAddr::from_ptr(&df.0) + (IST_STACK_SIZE as u64);
    let nmi_top = VirtAddr::from_ptr(&nmi.0) + (IST_STACK_SIZE as u64);

    // Recent x86_64 crate API
    tss.interrupt_stack_table[IST_DF] = df_top;
    tss.interrupt_stack_table[IST_NMI] = nmi_top;

    tss
}

pub fn init() {
    TSS.call_once(build_tss);
    GDT.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        let code = g.append(Descriptor::kernel_code_segment());
        let tss = g.append(Descriptor::tss_segment(TSS.get().unwrap()));
        (g, Selectors { code, tss })
    });

    let (g, sel) = GDT.get().unwrap();
    g.load();

    // Load CS via the Segment trait
    unsafe { CS::set_reg(sel.code) };

    // Load TSS
    unsafe { x86_64::instructions::tables::load_tss(sel.tss) };
}

pub const fn ist_index_df() -> u16 {
    IST_DF as u16
}
pub const fn ist_index_nmi() -> u16 {
    IST_NMI as u16
}
pub fn tss() -> &'static TaskStateSegment {
    TSS.get().unwrap()
}

pub fn kernel_cs_selector() -> u16 {
    let (_gdt, sel) = GDT.get().expect("GDT not initialized");
    sel.code.0
}
