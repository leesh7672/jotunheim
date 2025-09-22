// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64 as native;
