// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use core::f64::consts;

use alloc::boxed::Box;

use spin::Mutex;
use x86_64::{
    instructions::{
        interrupts, segmentation::{Segment, CS, DS, ES, SS}, tables::load_tss
    }, structures::{
        gdt::{self, Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    }, VirtAddr
};

use crate::{
    acpi::cpuid::CpuId,
    arch::x86_64::{
        apic::lapic_id,
        tables::{
            ISR, Stack,
            idt::{self, load_bsp_idt},
            registrate,
        },
    },
    kprint, kprintln,
};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

pub struct GdtLoader {
    sels: Selectors,
    gdt: *mut GlobalDescriptorTable,
}

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

pub fn generate(cpu: CpuId) -> GdtLoader {
    let gdt = Box::into_raw(Box::new(GlobalDescriptorTable::new()));
    GdtLoader {
        sels: generate_inner(cpu, gdt),
        gdt,
    }
}

fn generate_inner(cpu: CpuId, gdt_ref: *mut GlobalDescriptorTable) -> Selectors {
    // Build TSS once; it needs 'static for Descriptor::tss_segment
    let tss_ref: &'static mut TaskStateSegment = {
        let mut t = TaskStateSegment::new();
        let mut i = 0;
        let mut p = 0;
        super::access_mut(|isr| {
            if let Some(stack) = &isr.stack {
                let stack = stack.me(cpu).unwrap();
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
            }
        });
        Box::leak(Box::new(t))
    };

    // Build descriptors directly into the long-lived GDT that we will later load

    unsafe {
        let code = (*gdt_ref).append(Descriptor::kernel_code_segment());
        let data = (*gdt_ref).append(Descriptor::kernel_data_segment());
        let tss = (*gdt_ref).append(Descriptor::tss_segment(tss_ref));

        Selectors { code, data, tss }
    }
}

static BSP_GDT: Mutex<Option<GlobalDescriptorTable>> = Mutex::new(None);
static BSP_SEL: Mutex<Option<Selectors>> = Mutex::new(None);

/// Build + load GDT/TSS once; return selectors.
pub fn init() -> Selectors {
    ISR::new(None, None, Some(Box::new(Stack::new())));
    registrate(CpuId::dummy());
    let mut gdt = GlobalDescriptorTable::new();
    let sel = Some(generate_inner(CpuId::dummy(), &mut gdt));
    *BSP_SEL.lock() = sel;
    *BSP_GDT.lock() = Some(gdt);
    load_bsp_gdt(|| {
        idt::init(sel.unwrap());
        registrate(CpuId::me());
        let gdtinfo = generate(CpuId::me());
        load_inner(gdtinfo)
    })
}

fn load_bsp_gdt<R, F>(func: F) -> R
where
    F: FnOnce() -> R,
{
    unsafe {
        let g = BSP_GDT.lock();
        let gsels = BSP_SEL.lock().unwrap();
        let x = g.clone().unwrap();
        x.load_unsafe();
        CS::set_reg(gsels.code);
        DS::set_reg(gsels.data);
        ES::set_reg(gsels.data);
        SS::set_reg(gsels.data);
        load_tss(gsels.tss);
        let r = func();
        drop(x);
        r
    }
}

pub(super) fn load_inner(gdtinfo: GdtLoader) -> Selectors {
    unsafe {
        let gdt = gdtinfo.gdt;
        (*gdt).load();
        let sels = gdtinfo.sels;
        CS::set_reg(sels.code);
        DS::set_reg(sels.data);
        ES::set_reg(sels.data);
        SS::set_reg(sels.data);
        load_tss(sels.tss);
        sels
    }
}
