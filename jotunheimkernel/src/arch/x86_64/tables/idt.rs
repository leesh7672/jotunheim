// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(clippy::missing_safety_doc)]


use crate::arch::x86_64::tables::access_interrupt_mut;

use core::mem::size_of;
use core::ptr::addr_of_mut;

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
) {
    let h = handler as usize;
    let entry = IdtEntry {
        offset_low: (h & 0xFFFF) as u16,
        selector: 0x8,
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
    fn set_gate(&mut self, idx: usize, handler: unsafe extern "C" fn(), ist: u8, dpl: u8) {
        let base: *mut IdtEntry = addr_of_mut!(self.0) as *mut IdtEntry;
        set_gate_raw(base, idx, handler, ist, dpl);
    }
}

unsafe fn load_idt_ptr(ptr: *const Idt) {
    unsafe {
        let idtr = Idtr {
            limit: (size_of::<IdtEntry>() * 256 - 1) as u16,
            base: &raw const (*ptr).0[0] as u64,
        };
        core::arch::asm!(
            "lidt [{0}]",
            in(reg) &idtr,
            options(readonly, nostack, preserves_flags)
        );
    }
}

static mut IDT: Idt = Idt([empty_entry(); 256]);

pub fn load_idt() {
    unsafe { load_idt_ptr(&raw const IDT) };
}

pub fn init() {
    let mut idt = Idt([empty_entry(); 256]);
    for v in 0..=255usize {
        idt.set_gate(v, isr_default_stub, 0, 0);
    }
    access_interrupt_mut(|isr| {
        idt.set_gate(isr.vector as usize, isr.stub, isr.ist as u8, 0);
    });
    unsafe {
        IDT = idt;
        load_idt_ptr(&raw const IDT);
    }
}
