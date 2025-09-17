use alloc::boxed::Box;
use spin::Once;
use x86_64::{
    VirtAddr,
    instructions::{
        segmentation::{CS, DS, ES, SS, Segment},
        tables::load_tss,
    },
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

use crate::{arch::x86_64::tables::{registrate_me, Stack, ISR, STACK_SIZE}, kprintln};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub _data: SegmentSelector,
    pub _tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

// Singletons
static GDT: Once<GlobalDescriptorTable> = Once::new();
static SELECTORS: Once<Selectors> = Once::new();
static TSS: Once<TaskStateSegment> = Once::new();

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

/// Build + load GDT/TSS once; return selectors. Safe to call multiple times.
pub fn init() {
    ISR::new(None, None, Some(Box::new(Stack::new())));
    load();
}

pub fn load() {
    registrate_me();
    if let Some(_s) = SELECTORS.get() {
        return;
    }

    // Materialise TSS with real stacks (once)
    let tss_ref = TSS.call_once(|| {
        let mut t = TaskStateSegment::new();
        let mut i = 0;
        let mut p = 0;
        super::access(|isr| {
            if let Some(_) = &isr.stack {
                if let (Some(_), Some(_)) = (isr.vector, isr.stub) {
                    isr.index = Some(i);
                    t.interrupt_stack_table[i as usize] = top_raw(
                        isr.stack.clone().unwrap().me().unwrap().dump.as_ptr(),
                        STACK_SIZE,
                    );
                    i += 1;
                } else {
                    t.privilege_stack_table[p as usize] = top_raw(
                        isr.stack.clone().unwrap().me().unwrap().dump.as_ptr(),
                        STACK_SIZE,
                    );
                    p += 1;
                }
            } else {
            }
        });
        t
    });

    // Build temporary GDT and append entries
    let mut tmp = GlobalDescriptorTable::new();
    let code = tmp.append(Descriptor::kernel_code_segment());
    let data = tmp.append(Descriptor::kernel_data_segment());
    let tss = tmp.append(Descriptor::tss_segment(tss_ref));

    // Move into 'static storage and load from that &'static
    let gdt_ref: &'static GlobalDescriptorTable = GDT.call_once(|| tmp);
    unsafe {
        gdt_ref.load();
        CS::set_reg(code);
        DS::set_reg(data);
        ES::set_reg(data);
        SS::set_reg(data);
        load_tss(tss);
    }
    let sels = Selectors {
        code,
        _data: data,
        _tss: tss,
    };
    let _ = SELECTORS.call_once(|| sels);
}

// ---- Accessors ----

pub fn selectors() -> Selectors {
    *SELECTORS.get().expect("gdt::init() not called")
}

pub fn code_selector() -> SegmentSelector {
    selectors().code
}
