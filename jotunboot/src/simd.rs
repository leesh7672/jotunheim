// boot/src/cpu_simd.rs
#![allow(dead_code)]

use core::arch::x86_64::{__cpuid, __cpuid_count, _xsetbv};

const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR0_MP: u64 = 1 << 1;
const CR0_NE: u64 = 1 << 5;

const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;
const CR4_OSXSAVE: u64 = 1 << 18;

#[inline(always)]
fn rdcr0() -> u64 {
    let mut v;
    unsafe { core::arch::asm!("mov {}, cr0", out(reg) v) }
    v
}
#[inline(always)]
fn wrcr0(v: u64) {
    unsafe { core::arch::asm!("mov cr0, {}", in(reg) v, options(nostack, preserves_flags)) }
}
#[inline(always)]
fn rdcr4() -> u64 {
    let mut v;
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) v) }
    v
}
#[inline(always)]
fn wrcr4(v: u64) {
    unsafe { core::arch::asm!("mov cr4, {}", in(reg) v, options(nostack, preserves_flags)) }
}

#[derive(Copy, Clone, Debug)]
pub struct SimdCaps {
    pub has_xsave: bool,
    pub has_osxsave: bool,
    pub has_avx: bool,
    pub has_xsaveopt: bool,
    pub xcr0: u64,         // x87|SSE|AVX bits we actually set
    pub xsave_size: usize, // CPUID.(D,0).EBX for current XCR0
}

pub fn enable_sse_avx_boot() -> SimdCaps {
    // CPUID(1): feature bits
    let l1 = unsafe { __cpuid(1) };
    let ecx = l1.ecx;
    let has_xsave = (ecx & (1 << 26)) != 0;
    let has_osxsave = (ecx & (1 << 27)) != 0;
    let has_avx = (ecx & (1 << 28)) != 0;

    // CPUID(D,1): xsaveopt
    let d1 = unsafe { __cpuid_count(0xD, 1) };
    let has_xsaveopt = (d1.eax & 1) != 0;

    // Enable FPU/SSE in CR0, and OS* in CR4
    let mut cr0 = rdcr0();
    cr0 &= !(CR0_EM | CR0_TS); // enable FPU, clear task-switched
    cr0 |= CR0_MP | CR0_NE; // monitor coproc + native exceptions
    wrcr0(cr0);

    let mut cr4 = rdcr4();
    cr4 |= CR4_OSFXSR | CR4_OSXMMEXCPT; // SSE save/restore + SSE exceptions
    if has_xsave && has_osxsave {
        cr4 |= CR4_OSXSAVE;
    }
    wrcr4(cr4);

    // Supported xfeatures mask (CPUID.D.0 EDX:EAX)
    let d0 = unsafe { __cpuid_count(0xD, 0) };
    let supported = (d0.eax as u64) | ((d0.edx as u64) << 32);

    // Decide XCR0 (x87 + SSE always; AVX only if supported)
    const X87: u64 = 1 << 0;
    const SSE: u64 = 1 << 1;
    const YMM: u64 = 1 << 2;
    let mut xcr0: u64 = 0;
    if (supported & (X87 | SSE)) == (X87 | SSE) {
        xcr0 |= X87 | SSE;
    }
    if has_avx && (supported & YMM) != 0 {
        xcr0 |= YMM;
    }

    // Apply XCR0 only if CR4.OSXSAVE is actually set now
    if (rdcr4() & CR4_OSXSAVE) != 0 {
        unsafe {
            _xsetbv(0, xcr0);
        }
    } else {
        // Fallback path (legacy FXSAVE/FXRSTOR): keep logical xcr0 = x87|SSE
        xcr0 = X87 | SSE;
    }

    // XSAVE area size for *current* XCR0 (use EBX)
    let d0_after = unsafe { __cpuid_count(0xD, 0) };
    let mut size = d0_after.ebx as usize;
    if size & 63 != 0 {
        size = (size + 63) & !63;
    } // 64B align for safety

    SimdCaps {
        has_xsave,
        has_osxsave,
        has_avx,
        has_xsaveopt,
        xcr0,
        xsave_size: size,
    }
}
