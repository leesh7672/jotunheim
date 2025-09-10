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
