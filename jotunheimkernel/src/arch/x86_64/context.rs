#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct CpuContext {
    pub rax: u64, pub rcx: u64, pub rdx: u64, pub rbx: u64,
    pub rbp: u64, pub rsi: u64, pub rdi: u64,
    pub r8:  u64, pub r9:  u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub cs:  u64, pub rflags: u64,
}

unsafe extern "C" {
    fn __ctx_switch(prev: *mut CpuContext, next: *const CpuContext);
}

pub fn switch(prev: *mut CpuContext, next: *const CpuContext) {
    unsafe {
        __ctx_switch(prev, next);
    }
}
