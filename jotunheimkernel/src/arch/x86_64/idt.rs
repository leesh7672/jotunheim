use core::mem::size_of;
use spin::Once;
use x86_64::VirtAddr;
use x86_64::instructions::tables::lidt;
use x86_64::registers::control::Cr2;
use x86_64::structures::DescriptorTablePointer;

use crate::arch::x86_64::{apic, gdt};
use crate::println; // <-- bring the println! macro into this module

// ---- IDT entry ----
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    off_lo: u16,
    sel: u16,
    ist: u8,
    flags: u8,
    off_mid: u16,
    off_hi: u32,
    zero: u32,
}
impl IdtEntry {
    const fn missing() -> Self {
        Self {
            off_lo: 0,
            sel: 0x08,
            ist: 0,
            flags: 0,
            off_mid: 0,
            off_hi: 0,
            zero: 0,
        }
    }
}

static IDT: Once<[IdtEntry; 256]> = Once::new();

#[inline(always)]
fn set_gate(
    tbl: &mut [IdtEntry; 256],
    idx: usize,
    handler: unsafe extern "C" fn(),
    ist: u8,
    dpl: u8,
) {
    let addr = handler as u64;
    tbl[idx] = IdtEntry {
        off_lo: (addr & 0xFFFF) as u16,
        sel: 0x08, // kernel CS
        ist: ist & 0x7,
        flags: 0b1000_1110 | ((dpl & 0b11) << 5), // P=1, DPL, Type=1110
        off_mid: ((addr >> 16) & 0xFFFF) as u16,
        off_hi: ((addr >> 32) & 0xFFFF_FFFF) as u32,
        zero: 0,
    };
}

// ---- NASM stubs (unsafe externs) ----
unsafe extern "C" {
    fn isr_default_stub();
    fn isr_gp_stub();
    fn isr_pf_stub();
    fn isr_ud_stub();
    fn isr_df_stub();
}

// ---- Rust handlers called by NASM (exported, C ABI) ----

#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    // Spurious LAPIC vector (0xFF) happens right after enable — ignore it.
    if vec == 0xFF {
        static mut SEEN: bool = false;
        unsafe {
            if !SEEN {
                crate::println!("[INT] spurious vec=0xff err={:#018x}", err);
                SEEN = true;
            }
        }
        // No EOI for spurious; just return.
        return;
    }

    // APIC IRQs (>= 0x20) need EOI; exceptions (< 0x20) do not.
    if vec >= 0x20 {
        unsafe {
            crate::arch::x86_64::apic::eoi();
        }
    }

    // Optional: lightweight log for anything else
    crate::println!("[INT] other vec={:#04x} err={:#018x}", vec, err);
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(_vec: u64, err: u64) -> ! {
    println!("[#GP] err={:#018x}", err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust(_vec: u64, _err: u64) -> ! {
    println!("[#UD] invalid opcode");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_vec: u64, err: u64) -> ! {
    let cr2 = Cr2::read_raw();
    println!("[#PF] cr2={:#018x} err={:#018x}", cr2, err);
    loop {
        x86_64::instructions::hlt();
    }
}

// ---- Install and load IDT ----
pub fn init() {
    let mut idt = [IdtEntry::missing(); 256];

    // Default for all vectors (handles spurious)
    for v in 0..256 {
        set_gate(&mut idt, v, isr_default_stub, 0, 0);
    }

    // Explicit exceptions
    set_gate(&mut idt, 6, isr_ud_stub, 0, 0); // #UD
    set_gate(&mut idt, 13, isr_gp_stub, 0, 0); // #GP
    set_gate(&mut idt, 14, isr_pf_stub, 0, 0); // #PF
    set_gate(&mut idt, 8, isr_df_stub, gdt::ist_index_df() as u8, 0); // #DF on IST1

    IDT.call_once(|| idt);

    let base = VirtAddr::from_ptr(IDT.get().unwrap().as_ptr());
    let ptr = DescriptorTablePointer {
        limit: (256 * size_of::<IdtEntry>() - 1) as u16,
        base,
    };
    unsafe {
        lidt(&ptr);
    }
}
