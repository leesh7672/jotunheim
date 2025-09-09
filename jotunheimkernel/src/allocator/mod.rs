use crate::bootinfo::BootInfo;
use core::sync::atomic::{AtomicU64, Ordering::*};

#[derive(Clone, Copy)]
pub struct PhysFrame(pub u64); // 4 KiB aligned physical address

// No static mut! Two atomics are enough for a bump allocator.
static EARLY_NEXT: AtomicU64 = AtomicU64::new(0);
static EARLY_END: AtomicU64 = AtomicU64::new(0);

#[inline]
fn is_page_aligned(x: u64) -> bool {
    (x & 0xfff) == 0
}

/// Seed the early frame pool from BootInfo (or a conservative fallback).
pub fn early_init_from_bootinfo(boot: &BootInfo) {
    let (start, end) = choose_early_pool_from_bootinfo(boot);
    debug_assert!(is_page_aligned(start) && is_page_aligned(end) && start < end);
    EARLY_NEXT.store(start, SeqCst);
    EARLY_END.store(end, SeqCst);
    crate::println!("[alloc] early pool = {:#x}..{:#x}", start, end);
}

// TODO: replace with real selection from boot.memory_map
fn choose_early_pool_from_bootinfo(_boot: &BootInfo) -> (u64, u64) {
    let start = 0x0030_0000u64; // 48 MiB
    let end = 0x0038_0000u64; // 56 MiB
    (start, end)
}

#[inline]
pub fn alloc_frame() -> Option<PhysFrame> {
    alloc_frames(1)
}

pub fn alloc_frames(n: usize) -> Option<PhysFrame> {
    let bytes = (n as u64) * 0x1000;
    loop {
        let cur = EARLY_NEXT.load(Relaxed);
        let end = EARLY_END.load(Relaxed);
        if cur == 0 || cur.checked_add(bytes)? > end {
            return None;
        }
        if EARLY_NEXT
            .compare_exchange(cur, cur + bytes, AcqRel, Relaxed)
            .is_ok()
        {
            return Some(PhysFrame(cur));
        }
    }
}

// Keep this as a no-op if you already have a #[global_allocator] elsewhere.
#[inline]
pub fn heap_init() {}
