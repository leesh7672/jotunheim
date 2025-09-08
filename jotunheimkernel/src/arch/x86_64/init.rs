// src/arch/x86_64/init.rs
#![allow(unused)]

use crate::println;

use x86_64::instructions;
use x86_64::instructions::segmentation::Segment; // for CS::get_reg()
use x86_64::structures::gdt::SegmentSelector;

use crate::arch::x86_64::{apic, gdt, idt, ioapic};

// Your logs showed CS=0x0008; use the standard trio:
const CODE_SEL: SegmentSelector = SegmentSelector(0x0008);
const DATA_SEL: SegmentSelector = SegmentSelector(0x0010);
const TSS_SEL: SegmentSelector = SegmentSelector(0x0028);

pub fn init_arch() {
    // 1) GDT/TSS (side-effects only; your gdt::init() returns ())
    gdt::init();

    // 2) Reload CS, set DS/ES/SS, load TR
    unsafe {
        reload_cs_far(CODE_SEL);
        load_kernel_data(DATA_SEL.0);
        x86_64::instructions::tables::load_tss(TSS_SEL);
    }

    // 3) IDT
    idt::init();

    // 4) APIC bring-up (your module)
    apic::init();
    unsafe {
        ioapic::mask_all();
    }
    apic::open_all_irqs();

    // 5) Sanity prints
    let cs_now = x86_64::registers::segmentation::CS::get_reg().0;
    let tr_now = read_tr();
    println!(
        "[SEGS] CS={:#06x} TR={:#06x} (expect TR == {:#06x})",
        cs_now, tr_now, TSS_SEL.0
    );
    println!("[JOTUNHEIM] GDT/IDT is initialised.");

    // 6) Enable interrupts + start LAPIC timer
    instructions::interrupts::enable();
    apic::start_best_timer_hz(1_000);
}

// ---------- helpers (one copy in this module) ----------
#[inline(always)]
fn read_tr() -> u16 {
    let mut tr = 0;
    unsafe { core::arch::asm!("str {0:x}", out(reg) tr) };
    tr
}

#[inline(always)]
unsafe fn load_kernel_data(sel: u16) {
    core::arch::asm!(
        "mov ds, {0:x}",
        "mov es, {0:x}",
        "mov ss, {0:x}",
        in(reg) sel,
        options(nostack, preserves_flags),
    );
}

#[inline(always)]
pub unsafe fn reload_cs_far(sel: x86_64::structures::gdt::SegmentSelector) {
    core::arch::asm!(
        "push {sel}",                 // push target CS (16-bit selector is fine)
        "lea  rax, [rip + 2f]",       // compute RIP of label 2
        "push rax",                   // push target RIP
        "retf",                       // far return → loads new CS:RIP
        "2:",                         // numeric local label (allowed; not 0/1)
        sel = in(reg) u64::from(sel.0),
        lateout("rax") _,
        options(nostack)
    );
}
