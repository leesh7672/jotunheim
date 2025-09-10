#![allow(clippy::missing_safety_doc)]

use crate::arch::x86_64::{apic, context, gdt, simd};
use crate::sched::PreemptPack;
use crate::{println, sched};
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
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
pub struct Idtr {
    pub limit: u16,
    pub base: u64,
}

#[repr(transparent)]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([empty_entry(); 256]);

// Stubs from NASM
unsafe extern "C" {
    fn isr_default_stub();
    fn isr_gp_stub();
    fn isr_pf_stub();
    fn isr_df_stub();
    fn isr_ud_stub();
    fn isr_timer_stub();
    fn isr_spurious_stub();
}

static THROTTLED_ONCE: AtomicBool = AtomicBool::new(false);

fn kernel_cs_u16() -> u16 {
    gdt::code_selector().0
}

const IST_DF: u8 = 1; // uses interrupt_stack_table[0]
const IST_PF: u8 = 2; // uses interrupt_stack_table[1]
const IST_TIMER: u8 = 3;
const IST_GP: u8 = 4;
const IST_UD: u8 = 5;

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
        selector: kernel_cs_u16(), // <- use the real CS
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

fn set_gate(idx: usize, handler: unsafe extern "C" fn(), ist: u8, dpl: u8) {
    unsafe {
        let base: *mut IdtEntry = addr_of_mut!(IDT.0) as *mut IdtEntry;
        set_gate_raw(base, idx, handler, ist, dpl);
    }
}

unsafe fn load_idt_ptr(ptr: *const IdtEntry) {
    let idtr = Idtr {
        limit: (size_of::<IdtEntry>() * 256 - 1) as u16,
        base: ptr as u64,
    };
    unsafe {
        core::arch::asm!(
            "lidt [{0}]",
            in(reg) &idtr,
            options(readonly, nostack, preserves_flags)
        );
    }
}

pub fn init() {
    unsafe {
        for v in 0..=255usize {
            set_gate(v, isr_default_stub, 0, 0);
        }

        // Faults + timer
        set_gate(13, isr_gp_stub, IST_GP, 0); // #GP
        set_gate(14, isr_pf_stub, IST_PF, 0); // #PF
        set_gate(8, isr_df_stub, IST_DF, 0); // #DF with IST1
        set_gate(6, isr_ud_stub, IST_UD, 0); // #UD
        set_gate(apic::TIMER_VECTOR as usize, isr_timer_stub, IST_TIMER, 0);
        set_gate(0xFF as usize, isr_spurious_stub, 0, 0);
        let idt_ptr: *const IdtEntry = addr_of!(IDT.0) as *const IdtEntry;
        load_idt_ptr(idt_ptr);
    }
}

// ---------- Rust ISR targets that NASM stubs call ----------
#[unsafe(no_mangle)]
pub extern "C" fn isr_default_rust(vec: u64, err: u64) {
    if !THROTTLED_ONCE.swap(true, Ordering::Relaxed) {
        println!("[INT] default vec={:#04x} err={:#018x}", vec, err);
    }
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_gp_rust(_vec: u64, err: u64) -> ! {
    println!("[#GP] err={:#018x}", err);
    loop {
        x86_64::instructions::hlt();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn isr_spurious_rust(_vec: u64, _err: u64) {
    unsafe { apic::eoi() };
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_pf_rust(_vec: u64, err: u64, rip: u64) -> ! {
    use x86_64::registers::control::Cr2;
    let cr2 = Cr2::read().expect("CR2 read failed").as_u64();
    crate::arch::x86_64::mmio_map::log_va_mapping("PF-cr2", cr2, 0);

    println!(
        "[#PF] err={:#018x} cr2={:#016x} rip={:#016x}",
        err, cr2, rip
    );
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_df_rust(_vec: u64, _err: u64) -> ! {
    println!("[#DF] double fault");
    loop {
        x86_64::instructions::hlt();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_ud_rust(_vec: u64, _err: u64, rip: u64, rsp: u64) -> ! {
    println!("[#UD] rip: {:#x}, rsp: {:#x}", rip, rsp);
    sched::exit_current();
}

#[unsafe(no_mangle)]
pub extern "C" fn isr_timer_rust() -> *const PreemptPack {
    apic::timer_isr_eoi_and_rearm_deadline();
    sched::tick()
}

#[unsafe(no_mangle)]
pub extern "C" fn preempt(pack: *const PreemptPack) {
    unsafe {
        let p = &*pack;
        if !p.prev_simd.is_null() {
            simd::save(p.prev_simd);
        }
        context::switch(p.prev_ctx, p.next_ctx);
    }
}
