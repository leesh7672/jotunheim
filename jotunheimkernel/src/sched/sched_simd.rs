// src/sched/simd.rs

pub const SIZE: usize = 832; // default FXSAVE size; will be bumped by init

#[derive(Clone, Debug)]
pub struct SimdArea {
    pub dump: [u8; SIZE],
}

impl Copy for SimdArea {}

impl SimdArea {
    pub fn as_mut_ptr(mut self) -> *mut u8 {
        self.dump.as_mut_ptr()
    }
}

impl Default for SimdArea {
    fn default() -> Self {
        Self { dump: [0u8; SIZE] }
    }
}

unsafe impl Send for SimdArea {}
unsafe impl Sync for SimdArea {}
