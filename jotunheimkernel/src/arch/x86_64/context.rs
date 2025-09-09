#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct CpuContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub rip: u64,
}

unsafe extern "C" {
    fn __ctx_switch(prev: *mut CpuContext, next: *const CpuContext);
}

pub fn switch(prev: *mut CpuContext, next: *const CpuContext) {
    unsafe {
        __ctx_switch(prev, next);
    }
}
