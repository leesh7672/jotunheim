pub mod gdt;
pub mod idt;
pub mod isr;

use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::vec::Vec;
use spin::mutex::Mutex;
use x86_64::instructions::interrupts::without_interrupts;

use crate::arch::x86_64::apic::{self, lapic_id};
use crate::kprintln;

static THROTTLED_ONCE: AtomicBool = AtomicBool::new(false);

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    if !THROTTLED_ONCE.swap(true, Ordering::Relaxed) {
        kprintln!("[INT] default vec={:#04x} err={:#018x}", vec, err);
    }
    unsafe { apic::eoi() };
}

const STACK_SIZE: usize = 0x8000;

#[derive(Clone, Debug)]
#[repr(C)]
pub struct CpuStack {
    pub dump: [u8; STACK_SIZE],
    acpi_id: u32,
}

#[derive(Clone, Debug)]
pub struct Stack {
    stacks: Vec<Box<CpuStack>>,
}

impl Stack {
    pub fn new() -> Self {
        Self { stacks: Vec::new() }
    }
    pub fn registrate(&mut self) {
        self.stacks.insert(0, Box::new(CpuStack::new()));
    }
    pub fn me(&self) -> Option<&Box<CpuStack>> {
        let acpi_id = lapic_id();
        for stack in &self.stacks {
            if stack.acpi_id == acpi_id {
                return Some(stack);
            }
        }
        return None;
    }
}

impl CpuStack {
    pub fn new() -> Self {
        Self {
            dump: [0; STACK_SIZE],
            acpi_id: lapic_id(),
        }
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

pub fn registrate_me() {
    access(|e| {
        if let Some(stack) = e.stack.as_mut() {
            stack.registrate();
        }
    });
}

pub fn access<F>(mut func: F)
where
    F: FnMut(&mut ISR) -> (),
{
    without_interrupts(|| {
        let mut guard = IST.lock();
        for e in guard.as_mut().unwrap().iter_mut() {
            func(e.as_mut());
        }
    })
}
