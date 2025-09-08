use core::mem::size_of;
use spin::Once;

use x86_64::VirtAddr;
use x86_64::instructions::segmentation::Segment;
use x86_64::instructions::tables::lidt;
use x86_64::registers::control::Cr2;
use x86_64::registers::segmentation::CS;
use x86_64::structures::DescriptorTablePointer;

use crate::arch::x86_64::{apic, gdt};
use crate::println;

// ---- IDT entry (x86_64) ----
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
            sel: 0,
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
    let cs = CS::get_reg().0; // live code selector (e.g., 0x38 in your setup)
    tbl[idx] = IdtEntry {
        off_lo: (addr & 0xFFFF) as u16,
        sel: cs,
        ist: ist & 0x7,
        flags: 0b1000_1110 | ((dpl & 0b11) << 5), // P=1, DPL, Type=0xE (interrupt gate)
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
    fn isr_timer_stub();
    fn isr_df_stub();
}

// ---- Rust handlers called by NASM (exported, C ABI) ----
// Keep them non-diverging so stubs can iretq after returning.

#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(_vec: u64, _err: u64) {
    // Belt-and-suspenders: always EOI so a stray IRQ never wedges the CPU.
    unsafe {
        apic::eoi();
    }
    // Optional: comment out to keep logs quiet
    // println!("[INT] default");
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(_vec: u64, err: u64) -> ! {
    let sel = (err & 0xFFFF) as u16;
    let rpl = sel & 0b11;
    let ti = (sel >> 2) & 1; // 0=GDT, 1=LDT
    let idx = (sel & !0b111) >> 3;
    println!(
        "[#GP] err={:#06x} sel={:#06x} idx={} TI={} RPL={}",
        err as u16, sel, idx, ti, rpl
    );
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
    let cr2_u64 = Cr2::read().expect("CR2 not a canonical VirtAddr").as_u64();
    println!("[#PF] cr2={:#018x} err={:#018x}", cr2_u64, err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(_vec: u64, _err: u64) {
    use core::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(0);

    let v = CTR.fetch_add(1, Ordering::Relaxed) + 1;
    if v % 1000 == 0 {
        println!("[tick] {}", v);
    }

    unsafe {
        apic::eoi();
    }
}

// ---- Install and load IDT ----
pub fn init() {
    let mut idt = [IdtEntry::missing(); 256];

    // Default for all vectors (EOI inside default handler prevents wedges)
    for v in 0..256 {
        set_gate(&mut idt, v, isr_default_stub, 0, 0);
    }

    // Explicit exceptions
    set_gate(&mut idt, 6, isr_ud_stub, 0, 0); // #UD
    set_gate(&mut idt, 13, isr_gp_stub, 0, 0); // #GP
    set_gate(&mut idt, 14, isr_pf_stub, 0, 0); // #PF
    set_gate(&mut idt, 8, isr_df_stub, gdt::ist_index_df() as u8, 0); // #DF on IST1

    // LAPIC timer at TIMER_VECTOR (we’ll use 0x20)
    set_gate(&mut idt, apic::TIMER_VECTOR as usize, isr_timer_stub, 0, 0);

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
