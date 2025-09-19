// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

//
// ─────────────────────────── Raw helpers (Rust 2024) ─────────────────────────
//

fn rdmsr(msr: u32) -> u64 {
    unsafe {
        let mut hi: u64;
        let mut lo: u64;
        core::arch::asm!("rdmsr", in("ecx") msr, out("edx") hi, out("eax") lo);
        ((hi as u64) << 32) | (lo as u64)
    }
}

#[inline]
fn wrmsr(msr: u32, val: u64) {
    unsafe {
        let hi = (val >> 32) as u32;
        let lo = val as u32;
        core::arch::asm!("wrmsr", in("ecx") msr, in("edx") hi, in("eax") lo);
    }
}

#[inline]
fn has_x2apic() -> bool {
    // Avoid inline-asm CPUID (EBX constraints). Use the intrinsic instead.
    let r = unsafe { core::arch::x86_64::__cpuid(1) };
    (r.ecx & (1 << 21)) != 0
}

//
// ───────────────────────────── MSR constants ─────────────────────────────────
//

const MSR_IA32_APIC_BASE: u32 = 0x0000_001B; // bit11=APIC_EN, bit10=X2APIC_EN
const MSR_IA32_TSC_DEADLINE: u32 = 0x0000_06E0; // (documented; optional)

// x2APIC MSR window (0x800..=0xBFF)
const MSR_X2APIC_APICID: u32 = 0x0000_0802;
const MSR_X2APIC_TPR: u32 = 0x0000_0808;
const MSR_X2APIC_EOI: u32 = 0x0000_080B;
const MSR_X2APIC_SIVR: u32 = 0x0000_080F;
const MSR_X2APIC_ICR: u32 = 0x0000_0830; // Interrupt Command Register
const MSR_X2APIC_LVT_TIMER: u32 = 0x0000_0832;
const MSR_X2APIC_INIT_COUNT: u32 = 0x0000_0838;

//
// ─────────────────────────── LAPIC MMIO (dword off) ─────────────────────────
// (Offsets are in DWORDS, not bytes. We add them to a *mut u32 base.)
//

const LAPIC_ID_OFF: usize = 0x20 / 4;
const LAPIC_TPR_OFF: usize = 0x80 / 4;
const LAPIC_EOI_OFF: usize = 0xB0 / 4;
const LAPIC_SIVR_OFF: usize = 0xF0 / 4;
const LAPIC_ICRLO: usize = 0x300 / 4;
const LAPIC_ICRHI: usize = 0x310 / 4;
const LAPIC_LVT_TMR: usize = 0x320 / 4;
const LAPIC_INITCNT: usize = 0x380 / 4;
const LAPIC_DCR: usize = 0x3E0 / 4;

const APIC_PHYS_MASK: u64 = 0xFFFF_F000;

// Public vectors (keep your values)
pub const TIMER_VECTOR: u8 = 0x40;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

//
// ───────────────────────────── Mode cache ────────────────────────────────────
//

#[derive(Copy, Clone, PartialEq, Eq)]
enum Mode {
    Unknown,
    X2Apic,                    // MSR-backed
    XApicPhys { phys: u64 },   // before HHDM (phase 1)
    XApic { base: *mut u32 },  // MMIO via HHDM (phase 2)
}

static MODE: AtomicU8 = AtomicU8::new(0); // 0=Unknown,1=X2,2=XPhys,3=X
static HHDM_BASE: AtomicU64 = AtomicU64::new(0);

#[inline]
fn store_mode(m: Mode) {
    MODE.store(
        match m {
            Mode::Unknown => 0,
            Mode::X2Apic => 1,
            Mode::XApicPhys { .. } => 2,
            Mode::XApic { .. } => 3,
        },
        Ordering::SeqCst,
    );
}

#[inline]
fn load_mode() -> Mode {
    match MODE.load(Ordering::SeqCst) {
        1 => Mode::X2Apic,
        2 => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            Mode::XApicPhys { phys }
        }
        3 => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let base = (HHDM_BASE.load(Ordering::Relaxed) + phys) as *mut u32;
            Mode::XApic { base }
        }
        _ => Mode::Unknown,
    }
}

#[inline]
fn mmio() -> Option<*mut u32> {
    if matches!(load_mode(), Mode::XApic { .. }) {
        let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
        let base = (HHDM_BASE.load(Ordering::Relaxed) + phys) as *mut u32;
        Some(base)
    } else {
        None
    }
}

#[inline]
fn mmio_read(off: usize) -> u32 {
    if let Some(ptr) = mmio() {
        unsafe { read_volatile(ptr.add(off)) }
    } else {
        0
    }
}

#[inline]
fn mmio_write(off: usize, val: u32) {
    if let Some(ptr) = mmio() {
        unsafe { write_volatile(ptr.add(off), val) };
    }
}

//
// ───────────────────────────── Public API ────────────────────────────────────
//

/// Phase-1 (BSP): robustly enable APIC and pick mode (x2APIC preferred).
pub fn early_init() {
    let mut base = rdmsr(MSR_IA32_APIC_BASE);
    base |= 1 << 11; // APIC_EN
    if has_x2apic() {
        base |= 1 << 10; // X2APIC_EN
    } else {
        base &= !(1 << 10);
    }
    wrmsr(MSR_IA32_APIC_BASE, base);

    if (base & (1 << 10)) != 0 {
        store_mode(Mode::X2Apic);
    } else {
        let phys = base & APIC_PHYS_MASK;
        store_mode(Mode::XApicPhys { phys });
    }
}

/// Phase-2 (BSP): after paging/HHDM; finalize xAPIC mapping.
/// Pass your HHDM base here so APs can compute LAPIC MMIO.
pub fn paging(hhdm_base: u64) {
    HHDM_BASE.store(hhdm_base, Ordering::Relaxed);
    if let Mode::XApicPhys { phys } = load_mode() {
        let base = (hhdm_base + phys) as *mut u32;
        store_mode(Mode::XApic { base });
    }
}

/// Optional: call at the very top of `ap_entry(boot)` so each AP self-heals.
pub fn ap_init(hhdm_base: u64) {
    let mut base = rdmsr(MSR_IA32_APIC_BASE) | (1 << 11);
    if has_x2apic() {
        base |= 1 << 10;
    } else {
        base &= !(1 << 10);
    }
    wrmsr(MSR_IA32_APIC_BASE, base);
    HHDM_BASE.store(hhdm_base, Ordering::Relaxed);
    if (base & (1 << 10)) != 0 {
        store_mode(Mode::X2Apic);
    } else {
        let phys = base & APIC_PHYS_MASK;
        let mmio = (hhdm_base + phys) as *mut u32;
        store_mode(Mode::XApic { base: mmio });
    }
}

/// Safe on APs: never #GP/#PF (assumes BSP called `paging()` to set HHDM).
pub fn lapic_id() -> u32 {
    // Ensure THIS CPU has APIC/x2APIC enabled before reading.
    let mut base = rdmsr(MSR_IA32_APIC_BASE);
    let want_x2 = has_x2apic();
    let mut new_base = base | (1 << 11);
    if want_x2 {
        new_base |= 1 << 10;
    } else {
        new_base &= !(1 << 10);
    }
    if new_base != base {
        wrmsr(MSR_IA32_APIC_BASE, new_base);
        base = new_base;
        if (base & (1 << 10)) != 0 {
            store_mode(Mode::X2Apic);
        }
    }

    match load_mode() {
        Mode::X2Apic => rdmsr(MSR_X2APIC_APICID) as u32,
        Mode::XApic { .. } => mmio_read(LAPIC_ID_OFF) >> 24,
        Mode::XApicPhys { .. } | Mode::Unknown => {
            // Fallback: derive MMIO via cached HHDM (valid after BSP paging()).
            let phys = base & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let mmio = (hhdm + phys) as *const u32;
            unsafe { read_volatile(mmio.add(LAPIC_ID_OFF)) >> 24 }
        }
    }
}

/// Unmask all priorities on this CPU (TPR=0).
pub fn open_all_irqs() {
    match load_mode() {
        Mode::X2Apic => wrmsr(MSR_X2APIC_TPR, 0),
        Mode::XApic { .. } => mmio_write(LAPIC_TPR_OFF, 0),
        _ => {}
    }
}

/// Program Spurious-Interrupt Vector register (bit8 enables when set).
pub fn set_svr(vector: u8, enable: bool) {
    let val = (vector as u32) | if enable { 1 << 8 } else { 0 };
    match load_mode() {
        Mode::X2Apic => wrmsr(MSR_X2APIC_SIVR, val as u64),
        Mode::XApic { .. } => mmio_write(LAPIC_SIVR_OFF, val),
        _ => {
            // Best-effort write via cached HHDM
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let base = (hhdm + phys) as *mut u32;
            unsafe { write_volatile(base.add(LAPIC_SIVR_OFF), val) };
        }
    }
}

/// End Of Interrupt.
pub fn eoi() {
    match load_mode() {
        Mode::X2Apic => wrmsr(MSR_X2APIC_EOI, 0),
        Mode::XApic { .. } => mmio_write(LAPIC_EOI_OFF, 0),
        _ => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let base = (hhdm + phys) as *mut u32;
            unsafe { write_volatile(base.add(LAPIC_EOI_OFF), 0) };
        }
    }
}

/// Send a fixed IPI to `dest_apic`.
pub fn ipi_fixed(dest_apic: u32, vector: u8) {
    match load_mode() {
        Mode::X2Apic => {
            let hi = (dest_apic as u64) << 32;
            let lo = (0b000 << 8) | (vector as u64); // fixed delivery
            wrmsr(MSR_X2APIC_ICR, hi | lo);
        }
        Mode::XApic { .. } => {
            mmio_write(LAPIC_ICRHI, (dest_apic as u32) << 24);
            mmio_write(LAPIC_ICRLO, (0b000 << 8) | (vector as u32));
        }
        _ => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let base = (hhdm + phys) as *mut u32;
            unsafe {
                write_volatile(base.add(LAPIC_ICRHI), (dest_apic as u32) << 24);
                write_volatile(base.add(LAPIC_ICRLO), (0b000 << 8) | (vector as u32));
            }
        }
    }
}

/// Start per-CPU local timer (periodic). Replace with calibration later.
pub fn start_timer_hz(hz: u32) {
    // Coarse initial count that behaves under QEMU/TCG; replace with real calibration.
    let init = if hz == 0 { 100_000 } else { 10_000_000 / hz.max(1) };
    match load_mode() {
        Mode::X2Apic => {
            // LVT Timer MSR: periodic (bit17), vector = TIMER_VECTOR
            let lvt = ((1u64) << 17) | (TIMER_VECTOR as u64);
            wrmsr(MSR_X2APIC_LVT_TIMER, lvt);
            // Initial Count
            wrmsr(MSR_X2APIC_INIT_COUNT, init as u64);

            // Alternatively: use TSC-deadline via MSR_IA32_TSC_DEADLINE with calibration:
            let _ = MSR_IA32_TSC_DEADLINE; // documented but not used here
        }
        Mode::XApic { .. } => {
            mmio_write(LAPIC_DCR, 0b1011); // divide by 1 (common)
            mmio_write(LAPIC_LVT_TMR, (1 << 17) | (TIMER_VECTOR as u32)); // periodic
            mmio_write(LAPIC_INITCNT, init);
        }
        _ => {}
    }
}

// ===== INIT/SIPI helpers expected by smp.rs =====

#[inline]
fn icr_busy_x2() -> bool {
    // Bit12 (Delivery Status) is 1 while in progress
    (rdmsr(MSR_X2APIC_ICR) & (1 << 12)) != 0
}

#[inline]
fn icr_busy_x() -> bool {
    // Read LO dword to check delivery status bit12
    (mmio_read(LAPIC_ICRLO) & (1 << 12)) != 0
}

#[inline]
fn icr_wait() {
    // Small spin until hardware clears the in-progress bit
    match load_mode() {
        Mode::X2Apic => { while icr_busy_x2() {} }
        Mode::XApic { .. } => { while icr_busy_x() {} }
        _ => {}
    }
}

/// Send INIT IPI to `dest_apic`.
/// Intel SDM recommends: INIT (level=1, trigger=level) then deassert.
/// We: assert, wait, then deassert.
pub fn send_init(dest_apic: u32) {
    match load_mode() {
        Mode::X2Apic => {
            // delivery mode INIT (0b101<<8), level=1 (bit14), trigger=level (bit15)
            let lo_assert = (0b101u64 << 8) | (1 << 15) | (1 << 14);
            wrmsr(MSR_X2APIC_ICR, ((dest_apic as u64) << 32) | lo_assert);
            icr_wait();

            // deassert: same but level=0
            let lo_deassert = (0b101u64 << 8) | (1 << 15);
            wrmsr(MSR_X2APIC_ICR, ((dest_apic as u64) << 32) | lo_deassert);
            icr_wait();
        }
        Mode::XApic { .. } => {
            // HI must be written before LO in xAPIC MMIO mode
            mmio_write(LAPIC_ICRHI, (dest_apic as u32) << 24);
            let lo_assert = (0b101u32 << 8) | (1 << 15) | (1 << 14);
            mmio_write(LAPIC_ICRLO, lo_assert);
            icr_wait();

            mmio_write(LAPIC_ICRHI, (dest_apic as u32) << 24);
            let lo_deassert = (0b101u32 << 8) | (1 << 15);
            mmio_write(LAPIC_ICRLO, lo_deassert);
            icr_wait();
        }
        // Best effort fallback via HHDM if someone calls too early
        Mode::XApicPhys { .. } | Mode::Unknown => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let base = (hhdm + phys) as *mut u32;
            unsafe {
                write_volatile(base.add(LAPIC_ICRHI), (dest_apic as u32) << 24);
                write_volatile(base.add(LAPIC_ICRLO), (0b101u32 << 8) | (1 << 15) | (1 << 14));
            }
            // coarse wait
            while (unsafe { read_volatile(base.add(LAPIC_ICRLO)) } & (1 << 12)) != 0 {}
            unsafe {
                write_volatile(base.add(LAPIC_ICRHI), (dest_apic as u32) << 24);
                write_volatile(base.add(LAPIC_ICRLO), (0b101u32 << 8) | (1 << 15));
            }
            while (unsafe { read_volatile(base.add(LAPIC_ICRLO)) } & (1 << 12)) != 0 {}
        }
    }
}

/// Send SIPI (Startup IPI) to `dest_apic`.
/// `vector` is the 4KiB page number of the real-mode entry (i.e., entry >> 12).
pub fn send_startup(dest_apic: u32, vector: u8) {
    let vec = (vector & 0xFF) as u64; // low 8 bits carry the page number
    match load_mode() {
        Mode::X2Apic => {
            // delivery mode SIPI (0b110<<8), edge trigger, level ignored
            let lo = vec | (0b110u64 << 8);
            wrmsr(MSR_X2APIC_ICR, ((dest_apic as u64) << 32) | lo);
            icr_wait();
        }
        Mode::XApic { .. } => {
            mmio_write(LAPIC_ICRHI, (dest_apic as u32) << 24);
            let lo = (vec as u32) | (0b110u32 << 8);
            mmio_write(LAPIC_ICRLO, lo);
            icr_wait();
        }
        Mode::XApicPhys { .. } | Mode::Unknown => {
            let phys = rdmsr(MSR_IA32_APIC_BASE) & APIC_PHYS_MASK;
            let hhdm = HHDM_BASE.load(Ordering::Relaxed);
            let base = (hhdm + phys) as *mut u32;
            unsafe {
                write_volatile(base.add(LAPIC_ICRHI), (dest_apic as u32) << 24);
                write_volatile(base.add(LAPIC_ICRLO), (vec as u32) | (0b110u32 << 8));
            }
            while (unsafe { read_volatile(base.add(LAPIC_ICRLO)) } & (1 << 12)) != 0 {}
        }
    }
}