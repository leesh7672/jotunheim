use core::ptr::addr_of;

pub trait Memory {
    fn can_read(&self, addr: usize, len: usize) -> bool;
    fn can_write(&self, addr: usize, len: usize) -> bool;
}

#[inline]
fn in_range(addr: usize, len: usize, s: usize, e: usize) -> bool {
    addr >= s && addr.checked_add(len).map(|a| a <= e).unwrap_or(false)
}

/// Allow only .text/.rodata/.data/.bss until you add a page-walk/HHDM.
pub struct SectionMemory;

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __bss_start: u8;
    static __bss_end: u8;
}

impl Memory for SectionMemory {
    fn can_read(&self, addr: usize, len: usize) -> bool {
        let (t0, t1) = (
            addr_of!(__text_start) as usize,
            addr_of!(__text_end) as usize,
        );
        let (r0, r1) = (
            addr_of!(__rodata_start) as usize,
            addr_of!(__rodata_end) as usize,
        );
        let (d0, d1) = (
            addr_of!(__data_start) as usize,
            addr_of!(__data_end) as usize,
        );
        let (b0, b1) = (addr_of!(__bss_start) as usize, addr_of!(__bss_end) as usize);
        in_range(addr, len, t0, t1)
            || in_range(addr, len, r0, r1)
            || in_range(addr, len, d0, d1)
            || in_range(addr, len, b0, b1)
    }
    fn can_write(&self, addr: usize, len: usize) -> bool {
        let (d0, d1) = (
            addr_of!(__data_start) as usize,
            addr_of!(__data_end) as usize,
        );
        let (b0, b1) = (addr_of!(__bss_start) as usize, addr_of!(__bss_end) as usize);
        in_range(addr, len, d0, d1) || in_range(addr, len, b0, b1)
    }
}
