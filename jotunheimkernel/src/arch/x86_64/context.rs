// src/arch/x86_64/context.rs
#![allow(dead_code)]

#[repr(C)]
pub struct TrapFrame {
    // GP regs — order must match your asm SAVE_REGS:
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
    // software-pushed vector and (synthetic) error code
    pub vec: u64,
    pub err: u64,
    // hardware interrupt frame
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum DebugReason {
    Breakpoint,
    SingleStep,
    Exception(u8),
    Int3,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct CpuContext {
    // Callee-saved (SysV) + control state
    pub r15: u64,    // 0x00
    pub r14: u64,    // 0x08
    pub r13: u64,    // 0x10
    pub r12: u64,    // 0x18
    pub rbp: u64,    // 0x20
    pub rbx: u64,    // 0x28
    pub rsp: u64,    // 0x30
    pub rip: u64,    // 0x38
    pub rflags: u64, // 0x40 (bit 9 = IF)
}

unsafe extern "C" {
    fn __ctx_switch(prev: *mut CpuContext, next: *const CpuContext);
}

#[inline(always)]
pub fn switch(prev: *mut CpuContext, next: *const CpuContext) {
    unsafe { __ctx_switch(prev, next) }
}
