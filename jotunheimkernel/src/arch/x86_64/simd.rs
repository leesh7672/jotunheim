// src/arch/x86_64/simd.rs
#![allow(dead_code)]
use core::arch::asm;
use core::arch::x86_64::{__cpuid, __cpuid_count, _xgetbv, _xsetbv};

const CR0_EM: u64 = 1 << 2;
const CR0_MP: u64 = 1 << 1;
const CR0_NE: u64 = 1 << 5;

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

/// Enable SSE/AVX in CR0/CR4 and set XCR0 (x87+SSE+YMM if available).
/// Returns (xcr0_mask, xsave_bytes_for_current_xcr0).
pub fn enable_sse_avx() -> (u64, u32) {
    // 1) Turn on x87/SSE/OSXSAVE in control regs
    let mut cr0 = rdcr0();
    cr0 &= !CR0_EM; // FPU present
    cr0 |= CR0_MP | CR0_NE; // monitor coproc + native exceptions
    wrcr0(cr0);

    let mut cr4 = rdcr4();
    cr4 |= CR4_OSFXSR | CR4_OSXMMEXCPT | CR4_OSXSAVE;
    wrcr4(cr4);

    // 2) Decide XCR0 bits
    let info = probe();
    let mut xcr0 = XCR0_X87 | XCR0_SSE;
    if info.avx_supported {
        xcr0 |= XCR0_YMM; // enable AVX (YMM upper)
    }

    // 3) Apply XCR0 via intrinsic
    unsafe {
        _xsetbv(0, xcr0);
    }

    // 4) Query the required XSAVE area size for the current XCR0
    let d0 = unsafe { __cpuid_count(0xD, 0) };
    let size = d0.eax;

    (xcr0, size)
}

/// XSAVE/XRSTOR helpers (save/restore all enabled bits in XCR0)
#[inline]
pub unsafe fn xsave_all(save_area: *mut u8) {
    // Save all XCR0-enabled components (pass EDX:EAX mask = all 1s)
    unsafe {
        core::arch::asm!(
            "xsave [{buf}]",
            buf = in(reg) save_area,
            in("eax") u32::MAX,
            in("edx") u32::MAX,
            options(nostack)
        );
    }
}

#[inline]
pub unsafe fn xrstor_all(save_area: *const u8) {
    unsafe {
        core::arch::asm!(
            "xrstor [{buf}]",
            buf = in(reg) save_area,
            in("eax") u32::MAX,
            in("edx") u32::MAX,
            options(nostack)
        );
    }
}

/// Optional: read back XCR0 (for logging/debug)
pub fn read_xcr0() -> u64 {
    unsafe { _xgetbv(0) }
}
