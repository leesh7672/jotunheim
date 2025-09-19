// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use x86_64::instructions::interrupts;

use crate::{acpi::cpuid::CpuId, arch::x86_64::tables::{gdt::{self, generate}, idt::{self, load_bsp_idt}}, kprintln};

pub mod timer;
pub mod debug;
pub mod fault;
pub mod misc;

pub fn init(){
    timer::init();
    debug::init();
    fault::init();
    misc::init();
}
