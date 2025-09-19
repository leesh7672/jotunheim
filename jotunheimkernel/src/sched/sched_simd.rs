// src/sched/simd.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub const SIZE: usize = 4096;

#[derive(Clone, Debug)]
#[repr(C, align(64))]
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
