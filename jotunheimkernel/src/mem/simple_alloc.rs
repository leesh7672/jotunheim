// src/mem/simple_alloc.rs
use x86_64::{
    PhysAddr,
    structures::paging::{FrameAllocator, PhysFrame, Size4KiB},
};

pub struct TinyBump {
    pub next: u64,
    pub end:  u64,
}

impl TinyBump {
    pub const fn new(start: u64, end: u64) -> Self {
        Self { next: start, end }
    }

    /// Optional helpers
    pub fn remaining_pages(&self) -> usize {
        if self.end <= self.next { 0 } else { ((self.end - self.next) / 0x1000) as usize }
    }
}

unsafe impl FrameAllocator<Size4KiB> for TinyBump {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        while self.next + 0x1000 <= self.end {
            let cand = self.next;
            self.next = self.next.saturating_add(0x1000);

            // NEW: skip reserved pages
            if crate::mem::reserved::is_reserved_page(cand) {
                continue;
            }

            let frame = PhysFrame::containing_address(PhysAddr::new(cand));
            return Some(frame);
        }
        None
    }
}
