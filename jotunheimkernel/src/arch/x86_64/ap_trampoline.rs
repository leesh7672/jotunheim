// src/arch/x86_64/ap_trampoline.rs
#[allow(improper_ctypes)]
unsafe extern "C" {
    unsafe static _ap_tramp_start: u8;
    unsafe static _ap_tramp_end: u8;
    unsafe static _ap_tramp_apboot_ptr32: u8;
    unsafe static _ap_tramp_apboot_ptr64: u8;
}

pub fn blob() -> (&'static [u8], usize, usize) {
    unsafe {
        let start = &_ap_tramp_start as *const u8 as usize;
        let end = &_ap_tramp_end as *const u8 as usize;
        let p32 = &_ap_tramp_apboot_ptr32 as *const u8 as usize - start;
        let p64 = &_ap_tramp_apboot_ptr64 as *const u8 as usize - start;
        let len = end - start;
        (
            core::slice::from_raw_parts(start as *const u8, len),
            p32,
            p64,
        )
    }
}
