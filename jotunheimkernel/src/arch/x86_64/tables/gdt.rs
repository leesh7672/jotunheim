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

use crate::{
    arch::x86_64::{
        apic::lapic_id,
        tables::{registrate, Stack, ISR},
    }, kprintln
};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

pub fn generate(apic: u32) -> (Selectors, &'static mut GlobalDescriptorTable) {
    registrate(apic);

    let tss_ref = {
        // Singletons
        let mut tss: Box<TaskStateSegment> = Box::new(TaskStateSegment::new());

        // Materialise TSS with real stacks (once)
        *tss.as_mut() = {
            let mut t = TaskStateSegment::new();
            let mut i = 0;
            let mut p = 0;
            super::access(|isr| {
                if let Some(stack) = &isr.stack {
                    let stack = stack.me(apic).unwrap();
                    if let (Some(_), Some(_)) = (isr.vector, isr.stub) {
                        isr.index = Some(i);
                        t.interrupt_stack_table[i as usize] =
                            top_raw(&raw const stack.dump.as_ref()[0], stack.dump.len() - 1);
                        i += 1;
                    } else {
                        t.privilege_stack_table[p as usize] =
                            top_raw(&raw const stack.dump.as_ref()[0], stack.dump.len() - 1);
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

    let mut gdt = Box::new(GlobalDescriptorTable::new());
    let code = gdt.as_mut().append(Descriptor::kernel_code_segment());
    let data = gdt.as_mut().append(Descriptor::kernel_data_segment());
    let tss = gdt.as_mut().append(Descriptor::tss_segment(tss_ref));

    // Move into 'static storage and load from that &'static
    let sels = Selectors { code, data, tss };
    let gdt = Box::leak(gdt);
    (sels, gdt)
}

/// Build + load GDT/TSS once; return selectors.
pub fn init() -> Selectors {
    ISR::new(None, None, Some(Box::new(Stack::new())));
    let (sels, gdt) = generate(lapic_id());

    unsafe {
        gdt.load();
        CS::set_reg(sels.code);
        DS::set_reg(sels.data);
        ES::set_reg(sels.data);
        SS::set_reg(sels.data);
        load_tss(sels.tss);
    }
    sels
}

pub fn load(gdt: *const (Selectors, &'static mut GlobalDescriptorTable)) -> Selectors {
    let (sels, gdt) = unsafe { &*gdt };
    unsafe {
        gdt.load();
        CS::set_reg(sels.code);
        DS::set_reg(sels.data);
        ES::set_reg(sels.data);
        SS::set_reg(sels.data);
        load_tss(sels.tss);
    }
    *sels
}
