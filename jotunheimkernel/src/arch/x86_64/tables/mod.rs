// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub mod gdt;
pub mod idt;
pub mod isr;

use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use spin::mutex::Mutex;
use x86_64::instructions::interrupts::{self, without_interrupts};

use crate::acpi::cpuid::CpuId;
use crate::arch::x86_64::apic;
use crate::arch::x86_64::tables::gdt::load_temp_gdt;
use crate::arch::x86_64::tables::idt::load_bsp_idt;
use crate::kprintln;
use crate::sched::spawn;

static THROTTLED_ONCE: AtomicBool = AtomicBool::new(false);

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    if !THROTTLED_ONCE.swap(true, Ordering::Relaxed) {
        kprintln!("[INT] default vec={:#04x} err={:#018x}", vec, err);
    }
    apic::eoi();
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct CpuStack {
    pub dump: Box<[u8]>,
    cpu: CpuId,
}

#[derive(Clone, Debug)]
pub struct Stack {
    stacks: Vec<Box<CpuStack>>,
}

impl Stack {
    pub fn new() -> Self {
        Self { stacks: Vec::new() }
    }
    pub fn registrate(&mut self, cpu: CpuId) {
        self.stacks.insert(0, Box::new(CpuStack::new(cpu)));
    }
    pub fn me(&self, apic: CpuId) -> Option<&Box<CpuStack>> {
        for stack in &self.stacks {
            if stack.cpu == apic {
                return Some(stack);
            }
        }
        return None;
    }
}

impl CpuStack {
    pub fn new(cpu: CpuId) -> Self {
        const STACK_SIZE: usize = 0x4_0000;
        let dump = vec![0u8; STACK_SIZE].into_boxed_slice();
        Self { dump, cpu }
    }
}

#[derive(Clone, Debug)]
pub struct ISR {
    pub stack: Option<Box<Stack>>,
    pub vector: Option<u16>,
    pub index: Option<u16>,
    pub stub: Option<unsafe extern "C" fn()>,
}

impl ISR {
    pub fn registrate(vector: u16, stub: unsafe extern "C" fn()) {
        Self::new(Some(vector), Some(stub), Some(Box::new(Stack::new())));
    }
    pub fn registrate_without_stack(vector: u16, stub: unsafe extern "C" fn()) {
        Self::new(Some(vector), Some(stub), None);
    }
    pub fn new(
        vector: Option<u16>,
        stub: Option<unsafe extern "C" fn()>,
        stack: Option<Box<Stack>>,
    ) {
        without_interrupts(move || {
            loop {
                let mut guard = IST.lock();
                match guard.clone() {
                    Some(_) => {
                        guard.as_mut().unwrap().insert(
                            0,
                            Box::new(Self {
                                index: None,
                                vector: vector,
                                stack,
                                stub,
                            }),
                        );
                        break;
                    }
                    None => {
                        drop(guard);
                        init()
                    }
                }
            }
        })
    }
}

static IST: Mutex<Option<Box<Vec<Box<ISR>>>>> = Mutex::new(None);

pub fn init() {
    let mut guard = IST.lock();
    *guard = Some(Box::new(Vec::new()));
}

pub fn registrate(cpu: CpuId) {
    access_mut(|e| {
        if let Some(stack) = e.stack.as_mut() {
            stack.registrate(cpu);
        }
    });
}

pub fn access_mut<F>(mut func: F)
where
    F: FnMut(&mut ISR) -> (),
{
    let mut guard = IST.lock();
    let iter = guard.as_mut().unwrap().iter_mut();
    for e in iter {
        func(e);
    }
}

pub fn ap_init() {
    load_temp_gdt(|| {
        load_bsp_idt(|| {
            let id = CpuId::me();
            registrate(id);
            let gdt = gdt::generate(id);
            idt::ap_init(gdt::load_inner(gdt));
        })
    })
}
