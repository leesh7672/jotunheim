use core::arch::x86_64::__cpuid_count;
use core::ptr::{read_volatile, write_volatile};
use x86_64::registers::model_specific::Msr;

use crate::arch::x86_64::tsc;

const IA32_APIC_BASE: u32 = 0x1B;

// IA32_APIC_BASE bits
const APIC_GLOBAL_ENABLE: u64 = 1 << 11;
const APIC_X2_ENABLE: u64 = 1 << 10;

// Default xAPIC MMIO base (bits 12..35 if MSR base is zero)
const DEFAULT_LAPIC_MMIO_PADDR: u64 = 0xFEE0_0000;

// xAPIC registers (offsets)
const REG_ID: u32 = 0x020;
const REG_VERSION: u32 = 0x030;
const REG_TPR: u32 = 0x080;
const REG_EOI: u32 = 0x0B0;
const REG_SVR: u32 = 0x0F0;
const REG_LVT_TIMER: u32 = 0x320;
const REG_INIT_CNT: u32 = 0x380;
const REG_CURR_CNT: u32 = 0x390;
const REG_DIVIDE: u32 = 0x3E0;

// x2APIC MSR base (register_index = offset >> 4)
const X2_BASE: u32 = 0x800;
const fn x2(reg: u32) -> u32 {
    X2_BASE + (reg >> 4)
}

// SVR bits
const SVR_APIC_ENABLE: u32 = 1 << 8;

// LVT Timer bits
const LVT_MASKED: u32 = 1 << 16;
const LVT_PERIODIC: u32 = 1 << 17;

// Vector we use for the APIC timer
pub const TIMER_VECTOR: u8 = 0x20;

#[derive(Copy, Clone, Debug)]
enum Mode {
    XApic { base: *mut u32 },
    X2Apic,
}

static mut MODE: Mode = Mode::XApic {
    base: core::ptr::null_mut(),
};

#[inline]
fn has_x2apic() -> bool {
    // CPUID.01H:ECX[21] = x2APIC support
    unsafe { (__cpuid_count(1, 0).ecx & (1 << 21)) != 0 }
}

unsafe fn read_mmio(base: *mut u32, reg: u32) -> u32 {
    let p = (base as usize + reg as usize) as *mut u32;
    unsafe { read_volatile(p) }
}
unsafe fn write_mmio(base: *mut u32, reg: u32, val: u32) {
    let p = (base as usize + reg as usize) as *mut u32;
    unsafe {
        write_volatile(p, val);
    }
}

unsafe fn msr_read_u32(reg: u32) -> u32 {
    let v = unsafe { Msr::new(reg).read() };
    (v & 0xFFFF_FFFF) as u32
}
unsafe fn msr_write_u32(reg: u32, val: u32) {
    unsafe {
        Msr::new(reg).write(val as u64);
    }
}

unsafe fn apic_read(reg: u32) -> u32 {
    unsafe {
        match MODE {
            Mode::XApic { base } => read_mmio(base, reg),
            Mode::X2Apic => msr_read_u32(x2(reg)),
        }
    }
}
unsafe fn apic_write(reg: u32, val: u32) {
    unsafe {
        match MODE {
            Mode::XApic { base } => write_mmio(base, reg, val),
            Mode::X2Apic => msr_write_u32(x2(reg), val),
        }
    }
}

pub unsafe fn eoi() {
    unsafe {
        match MODE {
            Mode::XApic { .. } => apic_write(REG_EOI, 0),
            Mode::X2Apic => Msr::new(0x80B).write(0), // x2APIC EOI MSR
        }
    }
}

fn apic_base_from_msr() -> u64 {
    let msr = unsafe { Msr::new(IA32_APIC_BASE).read() };
    let base = msr & 0xFFFFF000; // bits 12..35
    if base != 0 {
        base
    } else {
        DEFAULT_LAPIC_MMIO_PADDR
    }
}

pub fn init() {
    unsafe {
        // Mask legacy PIC (we'll use APIC)
        use x86_64::instructions::port::Port;
        let mut pic1 = Port::<u8>::new(0x21);
        let mut pic2 = Port::<u8>::new(0xA1);
        pic1.write(0xFF);
        pic2.write(0xFF);

        // Enable APIC in MSR
        let mut base = Msr::new(IA32_APIC_BASE).read();
        base |= APIC_GLOBAL_ENABLE;
        let want_x2 = has_x2apic();
        if want_x2 {
            base |= APIC_X2_ENABLE;
        }
        Msr::new(IA32_APIC_BASE).write(base);

        // Choose mode
        if want_x2 {
            MODE = Mode::X2Apic;
            crate::println!("[JOTUNHEIM] APIC: Using x2APIC mode.");
        } else {
            let mmio = apic_base_from_msr() as *mut u32;
            MODE = Mode::XApic { base: mmio };
            crate::println!(
                "[JOTUNHEIM] APIC: Using xAPIC MMIO at {:#x}.",
                mmio as usize
            );
        }

        // Spurious vector (enable APIC)
        let svr = (0xFFu32) | SVR_APIC_ENABLE;
        apic_write(REG_SVR, svr);

        // Accept all priorities
        match MODE {
            Mode::XApic { .. } => apic_write(REG_TPR, 0),
            Mode::X2Apic => Msr::new(0x808).write(0), // x2APIC TPR MSR
        }
    }
}

/// Calibrate LAPIC timer with TSC and start periodic mode at `hz`.
pub fn start_timer_hz(hz: u32) -> (u32, u32) {
    const DIVIDE_VAL: u32 = 0x3; // divide by 16
    const CAL_MS: u64 = 50;

    unsafe {
        // Divider
        apic_write(REG_DIVIDE, DIVIDE_VAL);

        // One-shot: load max, wait ~50ms via TSC
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_MASKED);
        apic_write(REG_INIT_CNT, 0xFFFF_FFFF);

        let tsc_hz = tsc::tsc_hz_estimate();
        let target_delta = (tsc_hz / 1000) * CAL_MS;

        let t0 = tsc::rdtsc();
        while tsc::rdtsc().wrapping_sub(t0) < target_delta {
            core::hint::spin_loop();
        }

        let remained = apic_read(REG_CURR_CNT);
        let elapsed = 0xFFFF_FFFFu32.wrapping_sub(remained);

        let ticks_per_ms = (elapsed as u64) / CAL_MS;
        let want_ms = 1000u64 / (hz as u64);
        let init = core::cmp::max(1, (ticks_per_ms * want_ms) as u32);

        // Periodic mode
        apic_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | LVT_PERIODIC);
        apic_write(REG_INIT_CNT, init);

        crate::println!(
            "[JOTUNHEIM] APIC Timer: {} Hz (init={} div=16, tsc={} Hz).",
            hz,
            init,
            tsc_hz
        );

        (init, 16)
    }
}
