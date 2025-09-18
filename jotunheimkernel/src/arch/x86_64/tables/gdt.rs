use core::f64::consts;

use alloc::boxed::Box;

use spin::Mutex;
use x86_64::{
    VirtAddr,
    instructions::{
        segmentation::{CS, DS, ES, SS, Segment},
        tables::load_tss,
    },
    structures::{
        gdt::{self, Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
};

use crate::{
    arch::x86_64::{
        apic::lapic_id,
        tables::{
            ISR, Stack,
            idt::{self, prepare},
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

fn top_raw(base: *const u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) };
    VirtAddr::from_ptr(end).align_down(16u64)
}

pub fn generate(apic: u32) -> (Selectors, *mut Mutex<Option<GlobalDescriptorTable>>) {
    let gdt = Box::leak(Box::new(Mutex::new(None)));
    registrate(apic);
    (generate_inner(apic, gdt), gdt)
}
fn generate_inner(apic: u32, gdt_ref: &Mutex<Option<GlobalDescriptorTable>>) -> Selectors {
    // Build TSS once; it needs 'static for Descriptor::tss_segment
    let tss_ref: &'static mut TaskStateSegment = {
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
            }
        });
        Box::leak(Box::new(t))
    };

    // Build descriptors directly into the long-lived GDT that we will later load
    let mut guard = gdt_ref.lock();
    let gdt = guard.get_or_insert_with(GlobalDescriptorTable::new);

    let code = gdt.append(Descriptor::kernel_code_segment());
    let data = gdt.append(Descriptor::kernel_data_segment());
    let tss = gdt.append(Descriptor::tss_segment(tss_ref));

    Selectors { code, data, tss }
}

static BSP_GDT: Mutex<Option<GlobalDescriptorTable>> = Mutex::new(None);
static BSP_SEL: Mutex<Option<Selectors>> = Mutex::new(None);

/// Build + load GDT/TSS once; return selectors.
pub fn init() -> Selectors {
    let gdt = Box::leak(Box::new(Mutex::new(None)));
    ISR::new(None, None, Some(Box::new(Stack::new())));
    registrate(lapic_id());
    *BSP_SEL.lock() = Some(generate_inner(lapic_id(), &BSP_GDT));
    let sels = generate_inner(lapic_id(), &gdt);
    load_inner(&mut (sels, gdt));
    sels
}

fn load_bsp<R, F>(func: F) -> R
where
    F: Fn() -> R,
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

pub fn load(gdtinfo: u64) -> Selectors {
    load_bsp(|| prepare(|| load_inner(gdtinfo as *mut (Selectors, *mut Mutex<Option<GlobalDescriptorTable>>))))
}

fn load_inner(gdtinfo: *mut (Selectors, *mut Mutex<Option<GlobalDescriptorTable>>)) -> Selectors {
    unsafe {
        let gdtinfo = gdtinfo
            as *mut (
                Selectors,
                *mut spin::mutex::Mutex<Option<GlobalDescriptorTable>>,
            );
        let (sels, gdt) = &mut *gdtinfo;
        (**gdt).get_mut().as_mut().unwrap().load();
        CS::set_reg(sels.code);
        DS::set_reg(sels.data);
        ES::set_reg(sels.data);
        SS::set_reg(sels.data);
        load_tss(sels.tss);
        *sels
    }
}
