//! IDT setup for x86_64 + NASM stubs, with LAPIC timer gate.

#![allow(clippy::missing_safety_doc)]

use core::mem::size_of;
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
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,  // bits 0..15 of handler
    selector: u16,    // code segment selector
    ist: u8,          // bits 0..2 = IST index
    options: u8,      // type=0xE (interrupt gate), DPL, P
    offset_mid: u16,  // bits 16..31
    offset_high: u32, // bits 32..63
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

// Our IDT table (256 entries).
static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

// Build a gate entry value.
fn make_gate(handler: unsafe extern "C" fn(), ist: u8, dpl: u8) -> IdtEntry {
    let addr = handler as usize as u64;
    let sel = CS::get_reg().0;

    let mut e = IdtEntry::missing();
    e.offset_low = (addr & 0xFFFF) as u16;
    e.selector = sel;
    e.ist = ist & 0x7;
    e.options = 0x80 | ((dpl & 0x3) << 5) | 0x0E; // P=1, type=interrupt gate
    e.offset_mid = ((addr >> 16) & 0xFFFF) as u16;
    e.offset_high = ((addr >> 32) & 0xFFFF_FFFF) as u32;
    e.reserved = 0;
    e
}

// Store a gate into IDT using a raw pointer (Rust 2024: no &mut static mut).
unsafe fn set_gate_raw(
    idt_base: *mut IdtEntry,
    idx: usize,
    handler: unsafe extern "C" fn(),
    ist: u8,
    dpl: u8,
) {
    let entry = make_gate(handler, ist, dpl);
    idt_base.add(idx).write(entry);
}

// ------------------------ Public init: build and load IDT -------------------
pub fn init() {
    unsafe {
        // Raw pointer to the first entry; avoids &mut to static mut.
        let idt_ptr: *mut IdtEntry = core::ptr::addr_of_mut!(IDT) as *mut IdtEntry;

        // Fill all entries with the default stub.
        for v in 0usize..256 {
            set_gate_raw(idt_ptr, v, isr_default_stub, 0, 0);
        }

        // Exceptions of interest
        set_gate_raw(idt_ptr, 13, isr_gp_stub, 0, 0); // #GP
        set_gate_raw(idt_ptr, 14, isr_pf_stub, 0, 0); // #PF
        set_gate_raw(idt_ptr, 8, isr_df_stub, 1, 0); // #DF on IST1 (ensure IST1 in TSS)
        set_gate_raw(idt_ptr, 6, isr_ud_stub, 0, 0);

        // LAPIC timer IRQ at TIMER_VECTOR (0x20)
        set_gate_raw(idt_ptr, TIMER_VECTOR as usize, isr_timer_stub, 0, 0);

        // Load the IDT using a raw base address (no shared ref to static mut).
        let idtr = Idtr {
            limit: (size_of::<IdtEntry>() * 256 - 1) as u16,
            base: idt_ptr as u64,
        };
        unsafe {
            core::arch::asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack, preserves_flags));
        }
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
    apic::timer_isr_eoi_and_rearm_deadline();
}
