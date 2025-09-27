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

use crate::acpi::cpuid::{self, CpuId};
use crate::arch::x86_64::apic;
use crate::arch::x86_64::tables::gdt::{GdtLoader, load_temp_gdt};
use crate::arch::x86_64::tables::idt::load_idt;
use crate::debug::TrapFrame;
use crate::kprintln;
use crate::sched::exec;

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(tf: &mut TrapFrame) {
    let tf = unsafe { &*tf };
    kprintln!(
        "[#INT] vec={} err={:#x}\n  rip={:#018x} rsp={:#018x} rflags={:#018x}\n  cs={:#06x} ss={:#06x}",
        tf.vec,
        tf.err,
        tf.rip,
        tf.rsp,
        tf.rflags,
        tf.cs as u16,
        tf.ss as u16
    );
    apic::eoi();
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct Slot {
    pub dump: Box<[u8]>,
}

#[derive(Clone, Debug)]
pub struct Stack {
    stacks: Vec<Box<Slot>>,
    cpu: CpuId,
}

impl Stack {
    pub fn new(cpu: CpuId) -> Self {
        Self {
            stacks: vec![Box::new(Slot::new()); 8],
            cpu,
        }
    }
}

impl Slot {
    pub fn new() -> Self {
        const STACK_SIZE: usize = 0x4_0000;
        let dump = vec![0u8; STACK_SIZE].into_boxed_slice();
        Self { dump }
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
            let stacks = Box::new(Vec::new());
            *guard = Some(stacks)
        }
    }
}

pub fn register_cpu(cpu: CpuId) -> Box<Stack> {
    let mut guard = STACKS.lock();
    let stack = Stack::new(cpu);
    guard.as_mut().unwrap().insert(0, Box::new(stack));
    guard.as_mut().unwrap()[0].clone()
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
pub fn find_or_allocate_stack_for_cpu(cpu: CpuId) -> Box<Stack> {
    init();
    let mut guard = STACKS.lock();
    let iter = guard.as_mut().unwrap().iter();
    for e in iter {
        if e.cpu == cpu {
            return e.clone();
        }
    }
    return register_cpu(cpu);
}

pub fn ap_init() {
    load_temp_gdt(|| {
        load_idt();
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
        gdt::load_inner(gdt.unwrap());
    })
}
