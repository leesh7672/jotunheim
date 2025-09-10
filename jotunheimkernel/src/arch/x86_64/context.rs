#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct CpuContext {
    pub r15: u64,    // 0x00
    pub r14: u64,    // 0x08
    pub r13: u64,    // 0x10
    pub r12: u64,    // 0x18
    pub r11: u64,    // 0x20
    pub r10: u64,    // 0x28
    pub r9: u64,     // 0x30
    pub r8: u64,     // 0x38
    pub rdi: u64,    // 0x40
    pub rsi: u64,    // 0x48
    pub rbp: u64,    // 0x50
    pub rbx: u64,    // 0x58
    pub rdx: u64,    // 0x60
    pub rcx: u64,    // 0x68
    pub rax: u64,    // 0x70
    pub rsp: u64,    // 0x78
    pub rip: u64,    // 0x80
    pub rflags: u64, // 0x88
}
unsafe extern "C" {
    fn __ctx_switch(prev: *mut CpuContext, next: *const CpuContext);
}

pub fn switch(prev: *mut CpuContext, next: *const CpuContext) {
    crate::println!("[SWITCH] rip: {:#x}", unsafe { (*next).rip });
    unsafe {
        __ctx_switch(prev, next);
    }
}
