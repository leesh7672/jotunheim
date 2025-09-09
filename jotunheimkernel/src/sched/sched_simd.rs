// src/sched/simd.rs
use core::arch::asm;
use core::ptr::NonNull;

static mut XSAVE_SIZE: u32 = 512; // default FXSAVE size; will be bumped by init

pub fn set_xsave_size(n: u32) {
    unsafe {
        XSAVE_SIZE = n.max(512);
    }
}
pub fn get_xsave_size() -> u32 {
    unsafe { XSAVE_SIZE }
}

#[repr(align(64))]
pub struct SimdArea {
    // opaque byte area sized to XSAVE size
    ptr: NonNull<u8>,
    len: usize,
}
#[inline]
const fn align_up(x: usize, a: usize) -> usize {
    debug_assert!(a.is_power_of_two());
    (x + (a - 1)) & !(a - 1)
}

impl SimdArea {
    pub fn alloc() -> Option<Self> {
        // use your kernel page allocator; must be 64B aligned and zeroed once
        let len = align_up(get_xsave_size() as usize, 64);
        let base;
        let pages;
        unsafe {
            pages = (len + 4095) / 4096;
            base = crate::mem::alloc_pages(pages); // already mapped RW
            core::ptr::write_bytes(base?, 0, pages * 4096);
        }
        // Zero first time to ensure INIT state
        Some(Self {
            ptr: NonNull::new(base?)?,
            len,
        })
    }
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
    pub fn len(&self) -> usize {
        self.len
    }
}

impl Drop for SimdArea {
    fn drop(&mut self) {
        unsafe {
            let pages = (self.len + 4095) / 4096;
            let _ = crate::mem::unmap_pages(self.ptr.as_ptr(), pages);
            crate::mem::free_pages(self.ptr.as_ptr(), pages);
        }
    }
}

// XSAVE/XRSTOR wrappers
#[inline]
pub fn xsave(save_area: *mut u8) {
    let eax: u32 = u32::MAX;
    let edx: u32 = u32::MAX;
    unsafe {
        asm!("xsave [{0}]", in(reg) save_area, in("eax") eax, in("edx") edx, options(nostack));
    }
}
#[inline]
pub fn xrstor(save_area: *const u8) {
    let eax: u32 = u32::MAX;
    let edx: u32 = u32::MAX;
    unsafe {
        asm!("xrstor [{0}]", in(reg) save_area, in("eax") eax, in("edx") edx, options(nostack));
    }
}

unsafe impl Send for SimdArea {}
unsafe impl Sync for SimdArea {}
