// simd/caps.rs
#![allow(dead_code)]

use core::arch::x86_64::{__cpuid, __cpuid_count, _xsetbv};
use core::sync::atomic::{AtomicU32, Ordering};
use spin::Once;

/* ------------------------ Public capabilities record ------------------------ */

#[derive(Copy, Clone)]
pub struct XSaveCaps {
    pub has_xsave: bool,
    pub has_osxsave: bool,
    pub has_avx: bool,
    pub has_xsaveopt: bool,
    /// CPUID.(EAX=0xD,ECX=0) EDX:EAX — xfeature mask supported in XCR0
    pub xcr0_mask_supported: u64,
    /// XSAVE area size for the **current** XCR0 (EBX of CPUID.(D,0))
    pub xsave_size: usize,
    /// The XCR0 value we actually set (bit0=x87, bit1=SSE, bit2=AVX upper)
    pub xcr0: u64,
}

/* ------------------------------ Global storage ----------------------------- */

static CAPS: Once<XSaveCaps> = Once::new();
static READY: AtomicU32 = AtomicU32::new(0);

pub fn simd_ready() -> bool {
    READY.load(Ordering::Acquire) != 0
}

pub fn caps() -> &'static XSaveCaps {
    if CAPS.is_completed() {
        CAPS.get().unwrap()
    } else {
        enable_xsave_path();
        CAPS.get().unwrap()
    }
}

/* ------------------------------- CR helpers -------------------------------- */

const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR0_MP: u64 = 1 << 1;
const CR0_NE: u64 = 1 << 5;

const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;
const CR4_OSXSAVE: u64 = 1 << 18;


fn rdcr0() -> u64 {
    let mut v: u64;
    unsafe { core::arch::asm!("mov {}, cr0", out(reg) v) }
    v
}

fn wrcr0(v: u64) {
    unsafe { core::arch::asm!("mov cr0, {}", in(reg) v, options(nostack, preserves_flags)) }
}

fn rdcr4() -> u64 {
    let mut v: u64;
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) v) }
    v
}

fn wrcr4(v: u64) {
    unsafe { core::arch::asm!("mov cr4, {}", in(reg) v, options(nostack, preserves_flags)) }
}

/* ------------------------------ Initialization ----------------------------- */

pub fn enable_xsave_path() {
    // Discover baseline features
    let l1 = unsafe { __cpuid(1) };
    let ecx = l1.ecx;
    let has_xsave = (ecx & (1 << 26)) != 0;
    let has_osxsave = (ecx & (1 << 27)) != 0;
    let has_avx = (ecx & (1 << 28)) != 0;

    // Subleaf 1: XSAVEOPT support
    let d1 = unsafe { __cpuid_count(0xD, 1) };
    let has_xsaveopt = (d1.eax & 1) != 0;

    // Enable x87/SSE; clear EM/TS so FP/SSE won’t #NM
    let mut cr0 = rdcr0();
    cr0 &= !(CR0_EM | CR0_TS);
    cr0 |= CR0_MP | CR0_NE;
    wrcr0(cr0);

    // Turn on OSXSAVE in CR4 only if CPU says XSAVE+OSXSAVE exist
    let mut cr4 = rdcr4();
    cr4 |= CR4_OSFXSR | CR4_OSXMMEXCPT;
    if has_xsave && has_osxsave {
        cr4 |= CR4_OSXSAVE;
    }
    wrcr4(cr4);

    // Supported xfeature mask from CPUID.(D,0)
    let d0 = unsafe { __cpuid_count(0xD, 0) };
    let supported_mask = (d0.eax as u64) | ((d0.edx as u64) << 32);

    // Choose XCR0
    const X87: u64 = 1 << 0;
    const SSE: u64 = 1 << 1;
    const YMM: u64 = 1 << 2;

    let mut xcr0 = 0u64;
    if (supported_mask & (X87 | SSE)) == (X87 | SSE) {
        xcr0 |= X87 | SSE;
    }
    if has_avx && (supported_mask & YMM) != 0 {
        xcr0 |= YMM;
    }

    // Apply XCR0 only when CR4.OSXSAVE is actually set now
    if (rdcr4() & CR4_OSXSAVE) != 0 {
        unsafe {
            _xsetbv(0, xcr0);
        }
    } else {
        // Stay on legacy fxsave/fxrstor path; keep logical xcr0 for mask
        xcr0 = X87 | SSE;
    }

    // XSAVE area size for current XCR0 (use EBX, not EAX)
    let d0_after = unsafe { __cpuid_count(0xD, 0) };
    let mut size = d0_after.ebx as usize;
    if size & 63 != 0 {
        size = (size + 63) & !63;
    }

    let caps_val = XSaveCaps {
        has_xsave,
        has_osxsave,
        has_avx,
        has_xsaveopt,
        xcr0_mask_supported: supported_mask,
        xsave_size: size,
        xcr0,
    };
    READY.store(1, Ordering::Release);
    CAPS.call_once(|| caps_val);
}
