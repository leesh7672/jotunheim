// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project

pub mod debug;
pub mod fault;
pub mod misc;
pub mod timer;

pub fn init() {
    timer::init();
    debug::init();
    fault::init();
    misc::init();
}
