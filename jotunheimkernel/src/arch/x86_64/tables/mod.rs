// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub mod gdt;
pub mod idt;
pub mod isr;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use spin::mutex::Mutex;
use x86_64::instructions::interrupts::without_interrupts;

use crate::acpi::cpuid::CpuId;
use crate::arch::x86_64::apic;
use crate::arch::x86_64::tables::gdt::{GdtLoader, load_temp_gdt};
use crate::arch::x86_64::tables::idt::load_bsp_idt;
use crate::debug::TrapFrame;
use crate::kprintln;
use crate::sched::exec;

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(_tf: &mut TrapFrame) {
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
    pub fn register_cpu(&mut self, cpu: CpuId) {
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
pub struct Interrupt {
    pub vector: u16,
    pub ist: u16,
    pub stub: unsafe extern "C" fn(),
}

impl Interrupt {
    pub fn register_with_stack(vector: u16, stub: unsafe extern "C" fn(), ist: u16) {
        Self::new(vector, stub, ist);
    }
    pub fn register_without_stack(vector: u16, stub: unsafe extern "C" fn()) {
        Self::new(vector, stub, 0);
    }
    fn new(vector: u16, stub: unsafe extern "C" fn(), ist: u16) {
        without_interrupts(move || {
            loop {
                let mut guard = INTERRUPTS.lock();
                match guard.clone() {
                    Some(_) => {
                        guard.as_mut().unwrap().insert(
                            0,
                            Box::new(Self {
                                ist: ist,
                                vector: vector,
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

static INTERRUPTS: Mutex<Option<Box<Vec<Box<Interrupt>>>>> = Mutex::new(None);
static STACKS: Mutex<Option<Box<Vec<Box<Stack>>>>> = Mutex::new(None);

pub fn init() {
    {
        let mut guard = INTERRUPTS.lock();
        if guard.is_none() {
            *guard = Some(Box::new(Vec::new()));
        }
    }
    {
        let mut guard = STACKS.lock();
        if guard.is_none() {
            let mut stacks = Box::new(Vec::new());
            for _ in 0..8{
                stacks.insert(0, Box::new(Stack::new()));
            }
            *guard = Some(stacks)
        }
    }
}

pub fn register_cpu(cpu: CpuId) {
    access_stack(|e| {
        e.register_cpu(cpu);
    });
}
pub fn access_interrupt_mut<F>(mut func: F)
where
    F: FnMut(&mut Interrupt) -> (),
{
    init();
    let mut guard = INTERRUPTS.lock();
    let iter = guard.as_mut().unwrap().iter_mut();
    for e in iter {
        func(e);
    }
}
pub fn access_stack<F>(mut func: F)
where
    F: FnMut(&mut Stack) -> (),
{
    init();
    let mut guard = STACKS.lock();
    let iter = guard.as_mut().unwrap().iter_mut();
    for e in iter {
        func(e);
    }
}

pub fn ap_init() {
    load_temp_gdt(|| {
        load_bsp_idt(|| {
            let id = CpuId::me();
            let mut gdt: Option<GdtLoader> = None;
            let addr = &raw mut gdt as usize;
            exec::submit(move || unsafe {
                kprintln!("A");
                register_cpu(id);
                let gdt: &mut Option<GdtLoader> = &mut *(addr as *mut Option<GdtLoader>);
                *gdt = Some(gdt::generate(id));
            })
            .unwrap();
            while gdt.is_none() {}
            idt::ap_init(gdt::load_inner(gdt.unwrap()));
        })
    })
}
