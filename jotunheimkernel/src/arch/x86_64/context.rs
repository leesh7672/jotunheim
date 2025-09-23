// src/arch/x86_64/context.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#[repr(C)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vec: u64,
    pub err: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[derive(Copy, Clone, Default, Debug)]
#[repr(C)]
pub struct CpuContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
}

unsafe extern "C" {
    unsafe fn __ctx_switch(prev: *mut CpuContext, next: *const CpuContext);
    unsafe fn __first_switch(next: *const CpuContext) -> !;
}

pub fn switch(prev: *mut CpuContext, next: *const CpuContext) {
    unsafe { __ctx_switch(prev, next) }
}

pub fn first_switch(next: *const CpuContext){
    unsafe {__first_switch(next)}
}