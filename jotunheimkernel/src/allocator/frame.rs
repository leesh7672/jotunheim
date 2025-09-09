// allocator/frames.rs
use super::early::{early_alloc_frame, early_alloc_frames};
use super::early::PhysFrame;

pub fn alloc_frame() -> Option<PhysFrame> { early_alloc_frame() }
pub fn alloc_frames(n: usize) -> Option<PhysFrame> { early_alloc_frames(n) }
