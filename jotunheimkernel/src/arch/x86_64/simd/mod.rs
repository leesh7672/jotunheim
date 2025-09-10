// src/arch/x86_64/simd.rs
#![allow(dead_code)]

pub mod caps;

use core::arch::asm;
use core::arch::x86_64::{__cpuid, __cpuid_count, _xgetbv, _xsetbv};

const CR0_EM: u64 = 1 << 2;
const CR0_MP: u64 = 1 << 1;
const CR0_NE: u64 = 1 << 5;
const CR0_TS: u64 = 1 << 3;

const CR4_OSFXSR: u64 = 1 << 9; // SSE/SSE2 fxsave/fxrstor
const CR4_OSXMMEXCPT: u64 = 1 << 10; // SSE exceptions
const CR4_OSXSAVE: u64 = 1 << 18; // XSAVE/XRSTOR + XGETBV/XSETBV

// XCR0 bits
const XCR0_X87: u64 = 1 << 0;
const XCR0_SSE: u64 = 1 << 1;
const XCR0_YMM: u64 = 1 << 2; // AVX (YMM upper halves)

#[inline]
fn rdcr0() -> u64 {
    let v;
    unsafe { asm!("mov {}, cr0", out(reg) v) };
    v
}
#[inline]
fn wrcr0(v: u64) {
    unsafe { asm!("mov cr0, {}", in(reg) v) }
}
#[inline]
fn rdcr4() -> u64 {
    let v;
    unsafe { asm!("mov {}, cr4", out(reg) v) };
    v
}
#[inline]
fn wrcr4(v: u64) {
    unsafe { asm!("mov cr4, {}", in(reg) v) }
}

pub struct XSaveInfo {
    pub xsave_supported: bool,
    pub avx_supported: bool,
    pub xsave_size: u32, // required size for current XCR0 (subleaf 0)
    pub xsave_mask: u64, // supported feature mask (subleaf 0)
}

/// Probe features & sizes using stable intrinsics (no inline-asm rbx).
pub fn probe() -> XSaveInfo {
    // Leaf 1: feature bits
    let l1 = unsafe { __cpuid(1) };
    let osxsave = (l1.ecx & (1 << 27)) != 0;
    let avx = (l1.ecx & (1 << 28)) != 0;
    let sse2 = (l1.edx & (1 << 26)) != 0;

    // Leaf 0xD, subleaf 0: sizes & mask (interpreted for current XCR0)
    let d0 = unsafe { __cpuid_count(0xD, 0) };
    let xsave_size_all = d0.eax; // size required when enabling all supported bits
    let xsave_feature_mask = ((d0.edx as u64) << 32) | (d0.ecx as u64);

    XSaveInfo {
        xsave_supported: osxsave && sse2,
        avx_supported: avx,
        xsave_size: xsave_size_all,
        xsave_mask: xsave_feature_mask,
    }
}

pub fn enable_sse_avx() -> (u64, usize) {
    // --- CPUID feature discovery ---
    let leaf1 = unsafe { __cpuid(1) };
    let ecx1 = leaf1.ecx;

    let has_xsave = (ecx1 & (1 << 26)) != 0;
    let has_osxsave = (ecx1 & (1 << 27)) != 0;
    let has_avx = (ecx1 & (1 << 28)) != 0;

    // --- Control registers: enable x87/SSE and (optionally) XSAVE ---
    let mut cr0 = rdcr0();
    cr0 &= !(CR0_EM | CR0_TS); // enable FPU, clear TS so FP/SSE don’t #NM
    cr0 |= CR0_MP | CR0_NE; // monitor coproc + native exceptions
    wrcr0(cr0);

    let mut cr4 = rdcr4();
    cr4 |= CR4_OSFXSR | CR4_OSXMMEXCPT;
    if has_xsave && has_osxsave {
        cr4 |= CR4_OSXSAVE;
    }
    wrcr4(cr4);

    // --- XCR0: enable x87 + SSE + (optionally) YMM if supported ---
    let d0 = unsafe { __cpuid_count(0xD, 0) };
    let xfeat_lo = d0.eax as u64; // supported bits [31:0]
    let xfeat_hi = d0.edx as u64; // supported bits [63:32]
    let xfeat_mask = xfeat_lo | (xfeat_hi << 32);

    let mut xcr0: u64 = 0;
    // x87 and SSE must be enabled together for SSE usage
    if (xfeat_mask & (XCR0_X87 | XCR0_SSE)) == (XCR0_X87 | XCR0_SSE) {
        xcr0 |= XCR0_X87 | XCR0_SSE;
    }
    if has_avx && (xfeat_mask & XCR0_YMM) != 0 {
        xcr0 |= XCR0_YMM;
    }

    if (cr4 & CR4_OSXSAVE) != 0 {
        unsafe {
            _xsetbv(0, xcr0);
        } // XCR0
    } else {
        // No OSXSAVE: remain on legacy FXSAVE/FXRSTOR path if you have one.
        xcr0 = XCR0_X87 | XCR0_SSE; // logical state only
    }

    // --- XSAVE area size for current XCR0 ---
    // CPUID.(EAX=0xD,ECX=0).EBX = size required for *current* XCR0 mask
    let d0_after = unsafe { __cpuid_count(0xD, 0) };
    let mut size = d0_after.ebx as usize;
    // Align to 64 for safety (XSAVE area is naturally 64B-aligned in practice)
    if size & 63 != 0 {
        size = (size + 63) & !63;
    }

    (xcr0, size)
}

#[inline(always)]
pub fn save(area: *mut u8) {
    let c = caps::caps();
    if c.has_xsave && c.has_osxsave && (caps::simd_ready()) {
        // Use XSAVEOPT if available; else XSAVE
        let mask_lo = (c.xcr0 & 0xFFFF_FFFF) as u32;
        let mask_hi = (c.xcr0 >> 32) as u32;
        if c.has_xsaveopt {
            unsafe {
                core::arch::asm!("xsaveopt [{buf}]", buf = in(reg) area,
                             in("eax") mask_lo, in("edx") mask_hi,
                             options(nostack, preserves_flags));
            }
        } else {
            unsafe {
                core::arch::asm!("xsave [{buf}]", buf = in(reg) area,
                             in("eax") mask_lo, in("edx") mask_hi,
                             options(nostack, preserves_flags));
            }
        }
    } else {
        // Legacy fallback
        unsafe {
            core::arch::asm!("fxsave [{buf}]", buf = in(reg) area,
                         options(nostack, preserves_flags));
        }
    }
}

#[inline(always)]
pub fn restore(area: *const u8) {
    unsafe {
        let c = caps::caps();
        if c.has_xsave && c.has_osxsave && (caps::simd_ready()) {
            let mask_lo = (c.xcr0 & 0xFFFF_FFFF) as u32;
            let mask_hi = (c.xcr0 >> 32) as u32;
            {
                core::arch::asm!("xrstor [{buf}]", buf = in(reg) area,
                         in("eax") mask_lo, in("edx") mask_hi,
                         options(nostack, preserves_flags));
            }
        } else {
            {
                core::arch::asm!("fxrstor [{buf}]", buf = in(reg) area,
                         options(nostack, preserves_flags));
            }
        }
    }
}

/// Optional: read back XCR0 (for logging/debug)
pub fn read_xcr0() -> u64 {
    unsafe { _xgetbv(0) }
}
