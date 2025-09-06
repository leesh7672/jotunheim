use core::arch::x86_64::{__cpuid_count, _rdtsc};

#[inline]
pub fn rdtsc() -> u64 {
    unsafe { _rdtsc() }
}
#[allow(dead_code)]

pub fn has_invariant_tsc() -> bool {
    let l = unsafe { __cpuid_count(0x8000_0007, 0) };
    (l.edx & (1 << 8)) != 0
}

pub fn has_tsc_deadline() -> bool {
    let l = unsafe { __cpuid_count(0x01, 0) };
    (l.ecx & (1 << 24)) != 0
}

pub fn tsc_hz_estimate() -> u64 {
    // Try CPUID.15H first
    let l15 = unsafe { __cpuid_count(0x15, 0) };
    let (den, num, ecx) = (l15.eax, l15.ebx, l15.ecx);
    if den != 0 && num != 0 && ecx != 0 {
        let mut hz = (ecx as u64) * (num as u64) / (den as u64);
        // Heuristic: if result is implausibly low, interpret units (kHz/MHz) like some QEMU configs
        if hz < 10_000_000 {
            if hz >= 1_000 && hz < 10_000 {
                hz *= 1_000_000;
            }
            // looked like MHz (e.g., 2859)
            else if hz >= 10_000 {
                hz *= 1_000;
            } // looked like kHz
        }
        return hz;
    }
    // Fallback: CPUID.16H MHz
    let l16 = unsafe { __cpuid_count(0x16, 0) };
    let mhz = (l16.eax & 0xFFFF) as u64;
    if mhz != 0 {
        return mhz * 1_000_000;
    }

    3_000_000_000
}
