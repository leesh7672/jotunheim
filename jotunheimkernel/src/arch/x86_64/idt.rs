use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use x86_64::instructions::segmentation::Segment;
use x86_64::registers::segmentation::CS;

use spin::Once;

use crate::arch::x86_64::apic::{self, TIMER_VECTOR};

#[repr(C, packed)]
#[derive(Copy, Clone)]
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

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

#[repr(transparent)]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([empty_entry(); 256]);

unsafe extern "C" {
    fn isr_default_stub();
    fn isr_gp_stub();
    fn isr_pf_stub();
    fn isr_df_stub();
    fn isr_ud_stub();
    fn isr_timer_stub();
}

pub static TICKS: AtomicU64 = AtomicU64::new(0);
static THROTTLED_ONCE: AtomicBool = AtomicBool::new(false);

unsafe fn set_gate_raw(
    idt_base: *mut IdtEntry,
    idx: usize,
    handler: unsafe extern "C" fn(),
    ist: u8,
    dpl: u8,
) {
    let h = handler as usize;
    let cs = CS::get_reg().0 as u16;

    let entry = IdtEntry {
        offset_low: (h & 0xFFFF) as u16,
        selector: cs,
        ist: ist & 0x7,
        type_attr: 0x8E | ((dpl & 0x3) << 5),
        offset_mid: ((h >> 16) & 0xFFFF) as u16,
        offset_high: ((h >> 32) & 0xFFFF_FFFF) as u32,
        zero: 0,
    };

    core::ptr::write(idt_base.add(idx), entry);
}

unsafe fn set_gate(idx: usize, handler: unsafe extern "C" fn(), ist: u8, dpl: u8) {
    let base: *mut IdtEntry = addr_of_mut!(IDT.0) as *mut IdtEntry;
    set_gate_raw(base, idx, handler, ist, dpl);
}

unsafe fn load_idt_ptr(ptr: *const IdtEntry) {
    let idtr = Idtr {
        limit: (size_of::<IdtEntry>() * 256 - 1) as u16,
        base: ptr as u64,
    };
    core::arch::asm!(
        "lidt [{0}]",
        in(reg) &idtr,
        options(readonly, nostack, preserves_flags)
    );
}

pub fn init() {
    unsafe {
        for v in 0..=255usize {
            set_gate(v, isr_default_stub, 0, 0);
        }
        set_gate(13, isr_gp_stub, 0, 0);
        set_gate(14, isr_pf_stub, 0, 0);
        set_gate(8, isr_df_stub, 1, 0);
        set_gate(6, isr_ud_stub, 0, 0);
        set_gate(TIMER_VECTOR as usize, isr_timer_stub, 0, 0);

        let idt_ptr: *const IdtEntry = addr_of!(IDT.0) as *const IdtEntry;
        load_idt_ptr(idt_ptr);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    if !THROTTLED_ONCE.swap(true, Ordering::Relaxed) {
        crate::println!("[INT] default vec={:#04x} err={:#018x}", vec, err);
    }
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(_: u64, err: u64) -> ! {
    crate::println!("[#GP] err={:#018x}", err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_: u64, err: u64) -> ! {
    crate::println!("[#PF] err={:#018x}", err);
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(_: u64, _: u64) -> ! {
    crate::println!("[#DF] double fault");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust(_: u64, _: u64) -> ! {
    crate::println!("[#UD] invalid opcode");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust(_: u64, _: u64) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    apic::timer_isr_eoi_and_rearm_deadline();
}
