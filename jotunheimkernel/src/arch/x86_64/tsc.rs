use core::arch::x86_64::{__cpuid_count, _rdtsc};

#[inline]
pub fn rdtsc() -> u64 {
    // Safe to call on x86_64; serializing/ordering handled by callers.
    unsafe { _rdtsc() }
}

pub fn tsc_hz_estimate() -> u64 {
    // Try CPUID.15H (TSC/core crystal ratio)
    let leaf15 = unsafe { __cpuid_count(0x15, 0) };
    let (den, num) = (leaf15.eax, leaf15.ebx);
    let crystal = leaf15.ecx; // Hz
    if den != 0 && num != 0 && crystal != 0 {
        return (crystal as u64) * (num as u64) / (den as u64);
    }
    // Fallback: CPUID.16H base freq in MHz
    let leaf16 = unsafe { __cpuid_count(0x16, 0) };
    let mhz = leaf16.eax & 0xFFFF;
    if mhz != 0 {
        return (mhz as u64) * 1_000_000;
    }
    // Worst-case default
    3_000_000_000
}
