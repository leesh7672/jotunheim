//! Local APIC / x2APIC bring-up + timer (Periodic or TSC-Deadline).

use core::arch::x86_64::__cpuid_count;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::registers::model_specific::Msr;

use crate::arch::x86_64::tsc;

pub const SPURIOUS_VECTOR: u8 = 0xFF;
pub const TIMER_VECTOR: u8 = 0x20; // Safe zone, away from 0x20..=0x2F legacy PIC

// MSRs
const IA32_APIC_BASE: u32 = 0x1B;
const IA32_TSC_DEADLINE: u32 = 0x6E0;

// IA32_APIC_BASE bits
const APIC_GLOBAL_ENABLE: u64 = 1 << 11;
const APIC_X2_ENABLE: u64 = 1 << 10;

// xAPIC register offsets
const REG_TPR: u32 = 0x080;
const REG_EOI: u32 = 0x0B0;
const REG_SVR: u32 = 0x0F0;
const REG_LVT_TIMER: u32 = 0x320;
const REG_LVT_LINT0: u32 = 0x350;
const REG_LVT_LINT1: u32 = 0x360;
const REG_LVT_ERROR: u32 = 0x370;
const REG_INIT_CNT: u32 = 0x380;
const REG_CURR_CNT: u32 = 0x390;
const REG_DIVIDE: u32 = 0x3E0;
const REG_ISR0: u32 = 0x100;

// Which vector is currently in-service (per LAPIC ISR registers).
pub fn current_isr_vector() -> Option<u8> {
    for i in (0..8u32).rev() {
        let v = unsafe { super::apic::apic_read(REG_ISR0 + i * 0x10) };
        if v != 0 {
            let bit = 31 - v.leading_zeros();
            return Some((i * 32 + bit) as u8);
        }
    }
    None
}
// x2APIC MSR mapping base and helper (index = offset >> 4)
const X2_BASE: u32 = 0x800;
const fn x2(reg: u32) -> u32 {
    X2_BASE + (reg >> 4)
}

// LVT bits
const LVT_MASKED: u32 = 1 << 16;
const LVT_PERIODIC: u32 = 1 << 17;
const LVT_TSC_DEADLINE: u32 = 0b10 << 17;

// Spurious Vector Register (SVR) bits
const SVR_APIC_ENABLE: u32 = 1 << 8;

#[derive(Copy, Clone, Debug)]
enum Mode {
    XApic { base: *mut u32 },
    X2Apic,
}

static mut MODE: Mode = Mode::XApic {
    base: core::ptr::null_mut(),
};

// For deadline re-arm: period in TSC cycles (0 = not using deadline mode)
static DEADLINE_PERIOD_CYC: AtomicU64 = AtomicU64::new(0);

#[inline]
fn has_x2apic() -> bool {
    unsafe { (__cpuid_count(1, 0).ecx & (1 << 21)) != 0 }
}

unsafe fn read_mmio(base: *mut u32, reg: u32) -> u32 {
    let p = (base as usize + reg as usize) as *mut u32;
    unsafe { core::ptr::read_volatile(p) }
}
unsafe fn write_mmio(base: *mut u32, reg: u32, val: u32) {
    let p = (base as usize + reg as usize) as *mut u32;
    unsafe { core::ptr::write_volatile(p, val) }
}
unsafe fn msr_read_u32(reg: u32) -> u32 {
    unsafe { Msr::new(reg).read() as u32 }
}
unsafe fn msr_write_u32(reg: u32, val: u32) {
    unsafe { Msr::new(reg).write(val as u64) }
}

unsafe fn apic_read(reg: u32) -> u32 {
    match unsafe { MODE } {
        Mode::XApic { base } => unsafe { read_mmio(base, reg) },
        Mode::X2Apic => unsafe { msr_read_u32(x2(reg)) },
    }
}

unsafe fn apic_write(reg: u32, val: u32) {
    match unsafe { MODE } {
        Mode::XApic { base } => unsafe { write_mmio(base, reg, val) },
        Mode::X2Apic => unsafe { msr_write_u32(x2(reg), val) },
    }
}

pub unsafe fn eoi() {
    match unsafe { MODE } {
        Mode::XApic { .. } => unsafe { apic_write(REG_EOI, 0) },
        Mode::X2Apic => unsafe { Msr::new(0x80B).write(0) },
    }
}

fn apic_base_from_msr() -> u64 {
    let msr = unsafe { Msr::new(IA32_APIC_BASE).read() };
    let base = msr & 0xFFFF_F000;
    if base != 0 { base } else { 0xFEE0_0000 }
}

pub fn init() {
    // We'll print the chosen mode after the unsafe section.
    let _mode_str;

    unsafe {
        // 1) Mask legacy PIC completely (we use LAPIC only).
        use x86_64::instructions::port::Port;
        Port::<u8>::new(0x21).write(0xFF);
        Port::<u8>::new(0xA1).write(0xFF);

        // 2) Enable local APIC (+ x2APIC if the CPU supports it).
        let mut apic_base = Msr::new(IA32_APIC_BASE).read();
        apic_base |= APIC_GLOBAL_ENABLE;
        let want_x2 = has_x2apic();
        if want_x2 {
            apic_base |= APIC_X2_ENABLE;
        }
        Msr::new(IA32_APIC_BASE).write(apic_base);

        // 3) Select mode (MMIO xAPIC base from the MSR if not x2APIC).
        if want_x2 {
            MODE = Mode::X2Apic;
            _mode_str = "x2APIC";
        } else {
            MODE = Mode::XApic {
                base: apic_base_from_msr() as *mut u32,
            };
            _mode_str = "xAPIC";
        }

        // 4) Program Spurious Vector Register: enable APIC + set vector.
        apic_write(REG_SVR, (SPURIOUS_VECTOR as u32) | SVR_APIC_ENABLE);

        // 5) Accept all priorities (TPR = 0).
        match MODE {
            Mode::XApic { .. } => apic_write(REG_TPR, 0),
            Mode::X2Apic => Msr::new(0x808).write(0),
        }

        // 6) Mask all LVT entries so nothing can interrupt yet.
        apic_write(REG_LVT_TIMER, LVT_MASKED);
        apic_write(REG_LVT_LINT0, LVT_MASKED);
        apic_write(REG_LVT_LINT1, LVT_MASKED);
        apic_write(REG_LVT_ERROR, LVT_MASKED);
    }
    tpr_write(0xFF);
}

/// Start the best available timer at `hz`.
/// Prefers TSC-Deadline when supported; otherwise periodic LAPIC (div=16).
pub fn start_best_timer_hz(hz: u32) {
    if tsc::has_tsc_deadline() {
        start_timer_deadline_hz(hz);
    } else {
        start_timer_periodic_hz(hz);
    }
}
/// TSC-Deadline mode: set vector+mode, then arm the first deadline,
/// THEN drop priorities once.
pub fn start_timer_deadline_hz(hz: u32) {
    let tsc_hz = tsc::tsc_hz_estimate();
    let per = core::cmp::max(1, tsc_hz / (hz as u64));
    DEADLINE_PERIOD_CYC.store(per, Ordering::Relaxed);

    unsafe {
        // 1) Program LVT to deadline mode on our timer vector
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_TSC_DEADLINE);

        // 2) Arm the first deadline (now LVT is ready to deliver)
        Msr::new(IA32_TSC_DEADLINE).write(tsc::rdtsc().wrapping_add(per));
    }

    // 3) Open priorities exactly once (no direct MSR/TPr writes elsewhere)
    tpr_write(0x00);
}

/// Periodic mode: calibrate with masked LVT, then program periodic vector
/// and initial count, THEN drop priorities once.
pub fn start_timer_periodic_hz(hz: u32) {
    const DIV: u32 = 0x3; // ÷16
    const CAL_MS: u64 = 50;

    unsafe {
        apic_write(REG_DIVIDE, DIV);

        // Mask while calibrating
        apic_write(REG_LVT_TIMER, LVT_MASKED);
        apic_write(REG_INIT_CNT, 0xFFFF_FFFF);

        // short TSC-based calibration
        let tsc_hz = crate::arch::x86_64::tsc::tsc_hz_estimate();
        let target = (tsc_hz / 1000) * CAL_MS;
        let t0 = crate::arch::x86_64::tsc::rdtsc();
        while crate::arch::x86_64::tsc::rdtsc().wrapping_sub(t0) < target {
            core::hint::spin_loop();
        }

        // compute initial count
        let cur = apic_read(REG_CURR_CNT);
        let elapsed = 0xFFFF_FFFFu32.wrapping_sub(cur);
        let ticks_per_ms = (elapsed as u64) / CAL_MS;
        let want_ms = 1000u64 / (hz as u64);
        let init = core::cmp::max(1, (ticks_per_ms * want_ms) as u32);

        // Program periodic and UNMASK
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_PERIODIC);
        apic_write(REG_INIT_CNT, init);
    }

    // drop TPR once so TIMER_VECTOR (0x20) can be delivered
    tpr_write(0x00);
}

/// Called from the timer ISR: EOI and (if deadline mode) arm next deadline.
/// No printing here — keep ISRs lean and re-entrant-safe.
pub fn timer_isr_eoi_and_rearm_deadline() {
    let per = DEADLINE_PERIOD_CYC.load(Ordering::Relaxed);
    if per != 0 {
        unsafe {
            Msr::new(IA32_TSC_DEADLINE).write(tsc::rdtsc().wrapping_add(per));
        }
    }
    unsafe {
        eoi();
    }
}

#[inline(always)]
fn tpr_write(val: u32) {
    unsafe {
        match MODE {
            Mode::XApic { .. } => apic_write(REG_TPR, val),
            Mode::X2Apic => Msr::new(0x808).write(val as u64),
        }
    }
}

#[inline(always)]
pub fn lvt_timer_read() -> u32 {
    unsafe { apic_read(REG_LVT_TIMER) }
}

#[inline(always)]
pub fn lvt_timer_write(val: u32) {
    unsafe { apic_write(REG_LVT_TIMER, val) }
}

#[inline(always)]
pub fn lvt_timer_mask(mask: bool) {
    let mut v = lvt_timer_read();
    if mask {
        v |= 1 << 16;
    } else {
        v &= !(1 << 16);
    }
    lvt_timer_write(v);
}

// --- add near the top of apic.rs ---
#[derive(Copy, Clone)]
pub struct ApicSnapshot {
    pub mode: &'static str,
    pub svr: u32,
    pub tpr: u32,
    pub lvt_timer: u32,
    pub divide: u32,
    pub curr_cnt: u32,
    pub init_cnt: u32,
}

pub fn snapshot_debug() {
    unsafe {
        let lvt = match MODE {
            Mode::XApic { base } => read_mmio(base, REG_LVT_TIMER),
            Mode::X2Apic => msr_read_u32(x2(REG_LVT_TIMER)),
        };
        let div = match MODE {
            Mode::XApic { base } => read_mmio(base, REG_DIVIDE),
            Mode::X2Apic => msr_read_u32(x2(REG_DIVIDE)),
        };
        let init = match MODE {
            Mode::XApic { base } => read_mmio(base, REG_INIT_CNT),
            Mode::X2Apic => msr_read_u32(x2(REG_INIT_CNT)),
        };
        let cur = match MODE {
            Mode::XApic { base } => read_mmio(base, REG_CURR_CNT),
            Mode::X2Apic => msr_read_u32(x2(REG_CURR_CNT)),
        };
        crate::println!(
            "[APIC] LVT={:#010x} DIV={:#010x} INIT={:#010x} CUR={:#010x}",
            lvt,
            div,
            init,
            cur
        );
    }
}

pub fn open_all_irqs() {
    unsafe {
        match MODE {
            Mode::XApic { .. } => apic_write(REG_TPR, 0x00),
            Mode::X2Apic => Msr::new(0x808).write(0x00),
        }
    }
}
pub fn start_timer_periodic_rough() {
    const DIV: u32 = 0b011; // divide by 16
    const INIT: u32 = 50_000; // ~coarse period; adjust as needed
    unsafe {
        // enable APIC + spurious vector was already done in init()
        apic_write(REG_DIVIDE, DIV);
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | (1 << 17)); // periodic
        apic_write(REG_INIT_CNT, INIT);
    }
    // drop threshold after everything is armed
    unsafe {
        match MODE {
            Mode::XApic { .. } => apic_write(REG_TPR, 0),
            Mode::X2Apic => Msr::new(0x808).write(0),
        }
    }
}
