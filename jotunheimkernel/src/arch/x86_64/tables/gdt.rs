// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project

use alloc::{boxed::Box};

use spin::Mutex;
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
    acpi::cpuid::CpuId,
    arch::x86_64::tables::{
        access_stack,
        idt,
        register_cpu,
    }, kprintln,
};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

pub struct GdtLoader {
    sels: Selectors,
    gdt: &'static mut GlobalDescriptorTable,
}

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

pub fn generate(cpu: CpuId) -> GdtLoader {
    let gdt = Box::leak(Box::new(GlobalDescriptorTable::new()));
    GdtLoader {
        sels: generate_inner(cpu, gdt),
        gdt,
    }
}

pub(super) fn generate_inner(cpu: CpuId, gdt_ref: *mut GlobalDescriptorTable) -> Selectors {
    // Build TSS once; it needs 'static for Descriptor::tss_segment
    let tss_ref: &'static mut TaskStateSegment = {
        let mut t = TaskStateSegment::new();
        let mut i_idx = 0;
        access_stack(|e| {
            let dump = &e.me(cpu).unwrap().dump;
            if i_idx > 0 {
                t.interrupt_stack_table[i_idx - 1] = top_raw(&raw const dump[0], dump.len());
            } else {
                t.privilege_stack_table[0] = top_raw(&raw const dump[0], dump.len());
            }
            i_idx += 1;
        });
        Box::leak(Box::new(t))
    };

    unsafe {
        let code = (*gdt_ref).append(Descriptor::kernel_code_segment());
        let data = (*gdt_ref).append(Descriptor::kernel_data_segment());
        let tss = (*gdt_ref).append(Descriptor::tss_segment(tss_ref));

        Selectors { code, data, tss }
    }
}

static TEMP_GDT: Mutex<Option<GlobalDescriptorTable>> = Mutex::new(None);
static TEMP_SEL: Mutex<Option<Selectors>> = Mutex::new(None);

pub fn kernel_cs() -> u16 {
    8
}

pub fn kernel_ds() -> u16 {
    16
}

/// Build + load GDT/TSS once; return selectors.
pub fn init() -> Selectors {
    register_cpu(CpuId::dummy());
    let mut gdt = GlobalDescriptorTable::new();
    let sel = Some(generate_inner(CpuId::dummy(), &mut gdt));
    *TEMP_SEL.lock() = sel;
    *TEMP_GDT.lock() = Some(gdt);
    load_temp_gdt(|| {
        idt::init(sel.unwrap());
        register_cpu(CpuId::me());
        let gdtinfo = generate(CpuId::me());
        load_inner(gdtinfo)
    })
}

pub fn load_temp_gdt<R, F>(func: F) -> R
where
    F: FnOnce() -> R,
{
    unsafe {
        let mut g = TEMP_GDT.lock();
        let gsels = TEMP_SEL.lock().unwrap();
        let x = g.as_mut().unwrap();
        x.load_unsafe();
        CS::set_reg(gsels.code);
        DS::set_reg(gsels.data);
        ES::set_reg(gsels.data);
        SS::set_reg(gsels.data);
        load_tss(gsels.tss);
        let r = func();
        r
    }
}

pub(crate) fn load_inner(gdtinfo: GdtLoader) -> Selectors {
    unsafe {
        gdtinfo.gdt.load();
        let sels = gdtinfo.sels;
        CS::set_reg(sels.code);
        DS::set_reg(sels.data);
        ES::set_reg(sels.data);
        SS::set_reg(sels.data);
        load_tss(sels.tss);

        if sels.code.0 != kernel_cs() || sels.data.0 != kernel_ds() {
            kprintln!("Error on a segment! It must be {} for code and {} for data.", sels.code.0, sels.data.0);
        }
        sels
    }
}

unsafe impl Send for GdtLoader {}
unsafe impl Sync for GdtLoader {}
