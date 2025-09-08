//! IDT setup for x86_64 + NASM stubs, with LAPIC timer gate.

#![allow(clippy::missing_safety_doc)]

use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::instructions::segmentation::{CS, Segment};
use x86_64::registers::control::Cr2;

use crate::arch::x86_64::apic::{self, TIMER_VECTOR};

// --------- Extern NASM stubs (defined in asm/x86_64/isr_stubs.asm) ----------
unsafe extern "C" {
    fn isr_default_stub();
    fn isr_gp_stub();
    fn isr_pf_stub();
    fn isr_df_stub();
    fn isr_ud_stub();
    fn isr_timer_stub();
}

// Public heartbeat incremented by the timer ISR (read from idle loop).
pub static TICKS: AtomicU64 = AtomicU64::new(0);

// ----------------- Raw IDT entry / IDTR structs (x86_64) -------------------
// src/arch/x86_64/idt.rs (gate builder)
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,       // 3 bits used
    type_attr: u8, // P | DPL | 0 | type
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }
}

#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

#[unsafe(no_mangle)]
static mut IDT: Idt = Idt([IdtEntry::missing(); 256]);

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

#[inline(always)]
unsafe fn load_idt(idt_base: *const IdtEntry) {
    // size is 256 entries
    let limit = (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16;
    let idtr = Idtr {
        limit,
        base: idt_base as u64,
    };

    // SAFETY: We’re loading the IDTR with the address of our static IDT.
    unsafe {
        core::arch::asm!(
            "lidt [{}]",
            in(reg) &idtr,
            options(readonly, nostack, preserves_flags)
        );
    }
}
unsafe fn set_gate_raw(
    idt_base: *mut IdtEntry,
    idx: usize,
    handler: unsafe extern "C" fn(),
    ist: u8,
    dpl: u8,
) {
    let addr = handler as usize;
    let entry = IdtEntry {
        offset_low: (addr & 0xFFFF) as u16,
        selector: 0x08, // your kernel CS
        ist: ist & 0x7,
        type_attr: 0x8E | ((dpl & 0x3) << 5),
        offset_mid: ((addr >> 16) & 0xFFFF) as u16,
        offset_high: ((addr >> 32) & 0xFFFF_FFFF) as u32,
        zero: 0,
    };
    core::ptr::write(idt_base.add(idx), entry);
}
unsafe fn set_gate(idx: usize, handler: unsafe extern "C" fn(), ist: u8, dpl: u8) {
    // Raw pointer to the array inside the static, without taking & or &mut
    let base: *mut IdtEntry = addr_of_mut!(IDT.0) as *mut IdtEntry;
    set_gate_raw(base, idx, handler, ist, dpl);
}

unsafe fn load_idt_from_global() {
    let idt_ptr: *const IdtEntry = addr_of!(IDT.0) as *const IdtEntry;
    load_idt(idt_ptr);
}
// ------------------------ Public init: build and load IDT -------------------
pub fn init() {
    unsafe {
        for v in 0u8..=255 {
            set_gate(v as usize, isr_default_stub, 0, 0);
        }
        set_gate(13, isr_gp_stub, 0, 0);
        set_gate(14, isr_pf_stub, 0, 0);
        set_gate(8, isr_df_stub, 1, 0); // DF on an IST if you want
        set_gate(0x20, isr_timer_stub, 0, 0);

        load_idt_from_global();
    }
}

/// ----------------------- Rust-level ISR targets ----------------------------
/// These are called by the NASM stubs. Keep them lean and re-entrant safe.

#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, _err: u64) {
    let _ = vec;
    unsafe {
        apic::eoi();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(_vec: u64, err: u64) -> ! {
    crate::println!("[#GP] err={:#018x}", err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_vec: u64, err: u64) -> ! {
    let cr2_u64 = Cr2::read().expect("CR2 not canonical").as_u64();
    crate::println!("[#PF] cr2={:#018x} err={:#018x}", cr2_u64, err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(_vec: u64, _err: u64) -> ! {
    crate::println!("[#DF]");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust(_vec: u64, _err: u64) -> ! {
    crate::println!("[#UD] invalid opcode");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(_vec: u64, _err: u64) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        apic::eoi();
    }
}
