// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(clippy::missing_safety_doc)]

use alloc::boxed::Box;
use spin::Mutex;

use crate::arch::x86_64::tables::access_mut;
use crate::arch::x86_64::tables::gdt::Selectors;

use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

const fn empty_entry() -> IdtEntry {
    IdtEntry {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_high: 0,
        zero: 0,
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Idtr {
    pub limit: u16,
    pub base: u64,
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Idt([IdtEntry; 256]);

// Stubs from NASM
unsafe extern "C" {
    unsafe fn isr_default_stub();
}

fn set_gate_raw(
    idt_base: *mut IdtEntry,
    idx: usize,
    handler: unsafe extern "C" fn(),
    ist: u8,
    dpl: u8,
    sel: Selectors,
) {
    let h = handler as usize;
    let entry = IdtEntry {
        offset_low: (h & 0xFFFF) as u16,
        selector: sel.code.0,
        ist: ist & 0x7,
        type_attr: 0x8E | ((dpl & 0x3) << 5),
        offset_mid: ((h >> 16) & 0xFFFF) as u16,
        offset_high: ((h >> 32) & 0xFFFF_FFFF) as u32,
        zero: 0,
    };
    unsafe {
        core::ptr::write(idt_base.add(idx), entry);
    }
}

impl Idt {
    fn set_gate(
        &mut self,
        idx: usize,
        handler: unsafe extern "C" fn(),
        ist: u8,
        dpl: u8,
        sel: Selectors,
    ) {
        let base: *mut IdtEntry = addr_of_mut!(self.0) as *mut IdtEntry;
        set_gate_raw(base, idx, handler, ist, dpl, sel);
    }
}

unsafe fn load_idt_ptr(ptr: *const IdtEntry) {
    let idtr = Idtr {
        limit: (size_of::<IdtEntry>() * 256 - 1) as u16,
        base: ptr as u64,
    };
    unsafe {
        core::arch::asm!(
            "lidt [{0}]",
            in(reg) &idtr,
            options(readonly, nostack, preserves_flags)
        );
    }
}

static BSP_IDT: Mutex<Option<Idt>> = Mutex::new(None);

pub fn load_bsp_idt<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let idt = BSP_IDT.lock().unwrap().0;
    unsafe { load_idt_ptr(&idt[0]) };
    let r = f();
    let _ = idt;
    r
}

pub fn init(sel: Selectors) {
    let idt = Box::leak(Box::new(Idt([empty_entry(); 256])));
    for v in 0..=255usize {
        idt.set_gate(v, isr_default_stub, 0, 0, sel);
    }
    access_mut(|isr| {
        if let (Some(vec), Some(stub)) = (isr.vector, isr.stub) {
            let index = match isr.index {
                Some(index) => index,
                None => 0,
            };
            idt.set_gate(vec as usize, stub, index as u8, 0, sel);
        } else {
        }
    });
    let idt_ptr: *const IdtEntry = addr_of!(idt.0) as *const IdtEntry;
    unsafe { load_idt_ptr(idt_ptr) };
    *BSP_IDT.lock() = Some(*idt);
}

pub fn ap_init(sel: Selectors) {
    let idt = Box::leak(Box::new(Idt([empty_entry(); 256])));
    for v in 0..=255usize {
        idt.set_gate(v, isr_default_stub, 0, 0, sel);
    }
    access_mut(|isr| {
        if let (Some(vec), Some(stub)) = (isr.vector, isr.stub) {
            let index = match isr.index {
                Some(index) => index,
                None => 0,
            };
            idt.set_gate(vec as usize, stub, index as u8, 0, sel);
        } else {
        }
    });
    let idt_ptr: *const IdtEntry = addr_of!(idt.0) as *const IdtEntry;
    unsafe { load_idt_ptr(idt_ptr) };
}
