//! Local APIC / x2APIC bring-up + timer (Periodic or TSC-Deadline).
//! Two-phase init:
//!   - early_init_no_paging(): use x2APIC MSRs or xAPIC *physical* MMIO
//!   - finalize_after_paging_on(): if xAPIC, map phys → VA (UC) and switch to VA path

use core::arch::x86_64::__cpuid_count;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use x86_64::registers::model_specific::Msr;

use crate::arch::x86_64::tsc;

// Public vectors (adjust to your IDT layout)
pub const SPURIOUS_VECTOR: u8 = 0xFF;
pub const TIMER_VECTOR: u8 = 0x40;

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
const REG_ICR_LOW: u32 = 0x300;
const REG_ICR_HIGH: u32 = 0x310;

// ICR bits
const ICR_DM_INIT: u32 = 0b101 << 8;
const ICR_DM_STARTUP: u32 = 0b110 << 8;
const ICR_LEVEL_ASSERT: u32 = 1 << 14;
const ICR_TRIG_EDGE: u32 = 0 << 15;
const ICR_DST_NONE: u32 = 0 << 18;

// x2APIC MSR mapping base helper (index = offset >> 4)
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
    /// Early boot: paging OFF (or identity-mapped), use physical MMIO base
    XApicPhys { phys: u64 },
    /// Normal xAPIC: paging ON, base is a mapped VA
    XApic { base: *mut u32 },
    /// x2APIC: MSR path only
    X2Apic,
}

static mut MODE: Mode = Mode::XApicPhys { phys: 0 };
static INIT_EARLY: AtomicBool = AtomicBool::new(false);
static INIT_FINAL: AtomicBool = AtomicBool::new(false);

// For deadline re-arm: period in TSC cycles (0 = not using deadline mode)
static DEADLINE_PERIOD_CYC: AtomicU64 = AtomicU64::new(0);

/* ---------- helpers ---------- */

fn has_x2apic() -> bool {
    unsafe { (__cpuid_count(1, 0).ecx & (1 << 21)) != 0 }
}

fn apic_base_from_msr() -> u64 {
    let msr = unsafe { Msr::new(IA32_APIC_BASE).read() };
    let base = msr & 0xFFFF_F000;
    if base != 0 { base } else { 0xFEE0_0000 }
}

/* ---------- raw IO ---------- */

unsafe fn read_phys32(phys: u64, reg: u32) -> u32 {
    unsafe { core::ptr::read_volatile((phys + reg as u64) as *const u32) }
}

unsafe fn write_phys32(phys: u64, reg: u32, val: u32) {
    unsafe { core::ptr::write_volatile((phys + reg as u64) as *mut u32, val) }
}

unsafe fn read_mmio(base: *mut u32, reg: u32) -> u32 {
    unsafe { core::ptr::read_volatile((base as usize + reg as usize) as *const u32) }
}

unsafe fn write_mmio(base: *mut u32, reg: u32, val: u32) {
    unsafe { core::ptr::write_volatile((base as usize + reg as usize) as *mut u32, val) }
}

unsafe fn msr_read_u32(reg: u32) -> u32 {
    unsafe { Msr::new(reg).read() as u32 }
}

unsafe fn msr_write_u32(reg: u32, val: u32) {
    unsafe { Msr::new(reg).write(val as u64) }
}

/* ---------- unified accessors ---------- */

unsafe fn apic_read(reg: u32) -> u32 {
    match unsafe { MODE } {
        Mode::XApicPhys { phys } => unsafe { read_phys32(phys, reg) },
        Mode::XApic { base } => unsafe { read_mmio(base, reg) },
        Mode::X2Apic => unsafe { msr_read_u32(x2(reg)) },
    }
}

unsafe fn apic_write(reg: u32, val: u32) {
    match unsafe { MODE } {
        Mode::XApicPhys { phys } => unsafe { write_phys32(phys, reg, val) },
        Mode::XApic { base } => unsafe { write_mmio(base, reg, val) },
        Mode::X2Apic => unsafe { msr_write_u32(x2(reg), val) },
    }
}

/* ---------- PHASE 1: call BEFORE paging is enabled ---------- */

pub fn early_init() {
    if INIT_EARLY.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        // 1) Mask legacy PIC
        use x86_64::instructions::port::Port;
        Port::<u8>::new(0x21).write(0xFF);
        Port::<u8>::new(0xA1).write(0xFF);

        // 2) Enable APIC (+ x2 if supported)
        let mut base = Msr::new(IA32_APIC_BASE).read();
        base |= APIC_GLOBAL_ENABLE;
        let want_x2 = has_x2apic();
        if want_x2 {
            base |= APIC_X2_ENABLE;
        }
        Msr::new(IA32_APIC_BASE).write(base);

        // 3) Choose mode (no paging: use physical for xAPIC)
        if want_x2 {
            MODE = Mode::X2Apic;
        } else {
            MODE = Mode::XApicPhys {
                phys: apic_base_from_msr(),
            };
        }

        // 4) SVR enable + vector
        apic_write(REG_SVR, (SPURIOUS_VECTOR as u32) | SVR_APIC_ENABLE);

        // 5) TPR = 0
        match MODE {
            Mode::X2Apic => Msr::new(0x808).write(0),
            _ => apic_write(REG_TPR, 0),
        }

        // 6) Mask LVTs
        apic_write(REG_LVT_TIMER, LVT_MASKED);
        apic_write(REG_LVT_LINT0, LVT_MASKED);
        apic_write(REG_LVT_LINT1, LVT_MASKED);
        apic_write(REG_LVT_ERROR, LVT_MASKED);
    }
}

/* ---------- PHASE 2: call AFTER paging/HHDM/mapper are ready ---------- */

pub fn paging() {
    if INIT_FINAL.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        if let Mode::XApicPhys { phys } = MODE {
            // Map LAPIC phys → VA (UC) and switch to VA mode
            let va = crate::mem::map_mmio(phys, 0x1000);
            MODE = Mode::XApic {
                base: va as *mut u32,
            };
        }
        // x2APIC needs no change
    }
}

/* ---------- common ops ---------- */

pub unsafe fn eoi() {
    match unsafe { MODE } {
        Mode::X2Apic => unsafe { Msr::new(0x80B).write(0) },
        _ => unsafe { apic_write(REG_EOI, 0) },
    }
}

fn tpr_write(val: u32) {
    unsafe {
        match MODE {
            Mode::X2Apic => Msr::new(0x808).write(val as u64),
            _ => apic_write(REG_TPR, val),
        }
    }
}

pub fn open_all_irqs() {
    tpr_write(0x00);
}

/* ---------- timer ---------- */

pub fn start_timer_hz(hz: u32) {
    if tsc::has_tsc_deadline() {
        start_timer_deadline_hz(hz);
    } else {
        start_timer_periodic_hz(hz);
    }
}

pub fn start_timer_deadline_hz(hz: u32) {
    let tsc_hz = tsc::tsc_hz_estimate();
    let per = core::cmp::max(1, tsc_hz / (hz as u64));
    DEADLINE_PERIOD_CYC.store(per, Ordering::Relaxed);

    unsafe {
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_TSC_DEADLINE);
        Msr::new(IA32_TSC_DEADLINE).write(tsc::rdtsc().wrapping_add(per));
    }
    tpr_write(0x00);
}

pub fn start_timer_periodic_hz(hz: u32) {
    const DIV: u32 = 0x3; // ÷16
    const CAL_MS: u64 = 50;

    unsafe {
        apic_write(REG_DIVIDE, DIV);
        apic_write(REG_LVT_TIMER, LVT_MASKED);
        apic_write(REG_INIT_CNT, 0xFFFF_FFFF);

        // short TSC-based calibration
        let tsc_hz = tsc::tsc_hz_estimate();
        let target = (tsc_hz / 1000) * CAL_MS;
        let t0 = tsc::rdtsc();
        while tsc::rdtsc().wrapping_sub(t0) < target {
            core::hint::spin_loop();
        }

        // compute initial count
        let cur = apic_read(REG_CURR_CNT);
        let elapsed = 0xFFFF_FFFFu32.wrapping_sub(cur);
        let ticks_per_ms = (elapsed as u64) / CAL_MS;
        let want_ms = 1000u64 / (hz as u64);
        let init = core::cmp::max(1, (ticks_per_ms * want_ms) as u32);

        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_PERIODIC);
        apic_write(REG_INIT_CNT, init);
    }
    tpr_write(0x00);
}

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

/* ---------- ICR / bring-up helpers ---------- */

pub fn icr_write(dest_apic_id: u32, low: u32) {
    match unsafe { MODE } {
        Mode::X2Apic => {
            let val = ((dest_apic_id as u64) << 32) | (low as u64);
            unsafe {
                Msr::new(0x830).write(val);
            }
        }
        _ => unsafe {
            apic_write(REG_ICR_HIGH, dest_apic_id << 24);
            apic_write(REG_ICR_LOW, low);
            while (apic_read(REG_ICR_LOW) & (1 << 12)) != 0 {
                core::hint::spin_loop();
            }
        },
    }
}

pub fn send_init(apic_id: u32) {
    icr_write(
        apic_id,
        ICR_DM_INIT | ICR_LEVEL_ASSERT | ICR_TRIG_EDGE | ICR_DST_NONE,
    );
}

pub unsafe fn send_startup(apic_id: u32, vector: u8) {
    icr_write(apic_id, ICR_DM_STARTUP | ((vector as u32) & 0xFF));
}

pub fn lapic_id() -> u32 {
    match unsafe { MODE } {
        Mode::X2Apic => (unsafe { Msr::new(0x802).read() } & 0xFFFF_FFFF) as u32,
        _ => (unsafe { apic_read(0x20) } >> 24) as u32,
    }
}
