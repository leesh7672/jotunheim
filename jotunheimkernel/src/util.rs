unsafe extern "C" {
    unsafe static __bss_start: u8;
    unsafe static __bss_end: u8;
}
#[allow(dead_code)]

pub unsafe fn zero_bss() {
    let start = unsafe { &__bss_start as *const u8 as usize };
    let end = unsafe { &__bss_end as *const u8 as usize };
    unsafe {
        core::ptr::write_bytes(start as *mut u8, 0, end - start);
    }
}
