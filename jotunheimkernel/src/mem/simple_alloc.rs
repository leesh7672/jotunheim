// src/mem/simple_alloc.rs
use x86_64::{
    PhysAddr,
    structures::paging::{FrameAllocator, PhysFrame, Size4KiB},
};

pub struct TinyBump {
    next: u64,
    end: u64,
}

impl TinyBump {
    /// Give it a small scratch region of free physical memory (page aligned).
    pub const fn new(start: u64, end: u64) -> Self {
        Self { next: start, end }
    }
}

unsafe impl FrameAllocator<Size4KiB> for TinyBump {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        if self.next + 0x1000 > self.end {
            return None;
        }
        let frame = PhysFrame::containing_address(PhysAddr::new(self.next));
        self.next += 0x1000;
        Some(frame)
    }
}
