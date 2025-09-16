// src/mem/reserved.rs
#![allow(dead_code)]

use heapless::Vec as HVec;
use spin::Mutex;

use crate::bootinfo::BootInfo;

#[derive(Copy, Clone, Debug)]
pub enum ResvKind {
    Firmware(u32), // from BootInfo.memory_map.typ (non-usable)
    Kernel,        // kernel image (text/rodata/data/bss)
    Framebuffer,   // linear framebuffer
    Mmio,          // device MMIO carved out of RAM ranges (rare, but keep)
    Trampoline,    // SIPI trampoline (e.g., 0x8000)
    Other(u32),
}

#[derive(Copy, Clone, Debug)]
pub struct Resv {
    pub start: u64, // inclusive physical start
    pub end: u64,   // exclusive physical end
    pub kind: ResvKind,
}

const MAX_RESV: usize = 128;

static RESV: Mutex<HVec<Resv, MAX_RESV>> = Mutex::new(HVec::new());


fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + (a - 1)) & !(a - 1)
}

/// Reset table (use only at boot).
pub fn reset() {
    *RESV.lock() = HVec::new();
}

/// Try to insert a reserved range. Returns false if table is full.
pub fn reserve_range(start: u64, len: u64, kind: ResvKind) -> bool {
    if len == 0 {
        return true;
    }
    let s = align_down(start, 0x1000);
    let e = align_up(start + len, 0x1000);
    let mut v = RESV.lock();

    // Best-effort coalesce with same-kind neighbors
    // (simple: append; coalescing not required for correctness)
    v.push(Resv {
        start: s,
        end: e,
        kind,
    })
    .is_ok()
}

/// Is any page in [phys, phys+len) reserved?
pub fn is_reserved_range(phys: u64, len: u64) -> bool {
    if len == 0 {
        return false;
    }
    let s = align_down(phys, 0x1000);
    let e = align_up(phys + len, 0x1000);
    let v = RESV.lock();
    for r in v.iter() {
        if s < r.end && e > r.start {
            return true;
        }
    }
    false
}

pub fn is_reserved_page(phys: u64) -> bool {
    is_reserved_range(phys, 0x1000)
}

pub fn init(boot: &BootInfo) {
    reset();

    // 1.a) from BootInfo memory map:
    // Convention: typ==1 => usable RAM; everything else => reserved.
    // (Adjust if your enum values differ.)
    let l32_lo = boot.low32_pool_paddr;
    let l32_hi = l32_lo + boot.low32_pool_len;

    unsafe {
        let mm_ptr = boot.memory_map;
        let mm_len = boot.memory_map_len;
        for i in 0..mm_len {
            let mr = *mm_ptr.add(i);

            // Skip any overlap with the low32 pool.
            let mr_lo = mr.phys_start;
            let mr_hi = mr.phys_start.saturating_add(mr.len);
            let overlaps_low32 = !(mr_hi <= l32_lo || mr_lo >= l32_hi);

            if mr.typ != 1 && !overlaps_low32 {
                let _ = reserve_range(mr.phys_start, mr.len, ResvKind::Firmware(mr.typ));
            }
        }
    }

    // 1.b) framebuffer
    if boot.framebuffer.addr != 0 && boot.framebuffer.pitch != 0 {
        let fb_len = (boot.framebuffer.pitch as u64) * (boot.framebuffer.height as u64);
        let _ = reserve_range(boot.framebuffer.addr, fb_len, ResvKind::Framebuffer);
    }

    let _ = reserve_range(0, boot.low32_pool_paddr, ResvKind::Firmware(0));
    let _ = reserve_range(
        boot.early_heap_paddr + boot.early_heap_len,
        0x10_0000,
        ResvKind::Firmware(0),
    );

    const TRAMP_PHYS: u64 = 0x0000_8000;
    let _ = reserve_range(TRAMP_PHYS, 0x1000, ResvKind::Trampoline);

    let _ = reserve_range(0xFEE0_0000, 0x1000, ResvKind::Mmio);
    let _ = reserve_range(0xFEC0_0000, 0x1000, ResvKind::Mmio);

    unsafe extern "C" {
        unsafe static __kernel_start: u8;
        unsafe static __kernel_end: u8;
    }

    let k_size =
        unsafe { (&__kernel_end as *const u8 as u64) - (&__kernel_start as *const u8 as u64) };

    // Reserve at the known physical base from BootInfo
    let k_phys = boot.kernel_phys_base;
    let _ = reserve_range(k_phys, k_size, ResvKind::Kernel);
}
