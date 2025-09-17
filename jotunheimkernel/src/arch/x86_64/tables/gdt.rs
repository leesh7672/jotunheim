use alloc::boxed::Box;

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

use crate::arch::x86_64::tables::{ISR, STACK_SIZE, Stack, registrate_me};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub _data: SegmentSelector,
    pub _tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

/// Build + load GDT/TSS once; return selectors. Safe to call multiple times.
pub fn init() -> Selectors {
    ISR::new(None, None, Some(Box::new(Stack::new())));
    load()
}

pub fn load() -> Selectors {
    let tss_ref = {
        // Singletons
        let mut tss: Box<TaskStateSegment> = Box::new(TaskStateSegment::new());

        registrate_me();

        // Materialise TSS with real stacks (once)
        *tss.as_mut() = {
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
        };

        // Build temporary GDT and append entries
        Box::leak(tss)
    };

    let mut tmp = Box::new(GlobalDescriptorTable::new());
    let code = tmp.as_mut().append(Descriptor::kernel_code_segment());
    let data = tmp.as_mut().append(Descriptor::kernel_data_segment());
    let tss = tmp.as_mut().append(Descriptor::tss_segment(tss_ref));

    // Move into 'static storage and load from that &'static
    let gdt_ref = Box::leak(tmp);

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
    sels
}
