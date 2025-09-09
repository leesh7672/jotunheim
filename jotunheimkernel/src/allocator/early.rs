// allocator/early.rs
use core::sync::atomic::{AtomicU64, Ordering::*};

#[derive(Clone, Copy)]
pub struct PhysFrame(pub u64); // physical address, 4 KiB aligned

pub struct TinyBump {
    next: AtomicU64,
    end: u64,
}

impl TinyBump {
    pub const fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
            end: 0,
        }
    }

    pub unsafe fn seed(&mut self, start_phys: u64, end_phys: u64) {
        debug_assert!(start_phys % 0x1000 == 0 && end_phys % 0x1000 == 0);
        self.next.store(start_phys, SeqCst);
        self.end = end_phys;
    }

    pub fn alloc(&self, frames: usize) -> Option<PhysFrame> {
        let bytes = (frames as u64) * 0x1000;
        loop {
            let cur = self.next.load(Relaxed);
            if cur == 0 || cur + bytes > self.end {
                return None;
            }
            if self
                .next
                .compare_exchange(cur, cur + bytes, AcqRel, Relaxed)
                .is_ok()
            {
                return Some(PhysFrame(cur));
            }
        }
    }
}

static mut EARLY: TinyBump = TinyBump::new();

// public wrappers
pub unsafe fn early_seed(start_phys: u64, end_phys: u64) {
    EARLY.seed(start_phys, end_phys)
}
pub fn early_alloc_frame() -> Option<PhysFrame> {
    unsafe { EARLY.alloc(1) }
}
pub fn early_alloc_frames(n: usize) -> Option<PhysFrame> {
    unsafe { EARLY.alloc(n) }
}
