use core::ptr::addr_of;
use crate::mem::{KHEAP_START, KHEAP_SIZE};

pub trait Memory {
    fn can_read(&self, addr: usize, len: usize) -> bool;
    fn can_write(&self, addr: usize, len: usize) -> bool;
}

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

fn in_any_range(addr: usize, len: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(s, e)| in_range(addr, len, s, e))
}

impl Memory for SectionMemory {
    fn can_read(&self, addr: usize, len: usize) -> bool {
        unsafe {
            use core::ptr::addr_of;
            let text = (
                addr_of!(__text_start) as usize,
                addr_of!(__text_end) as usize,
            );
            let rod = (
                addr_of!(__rodata_start) as usize,
                addr_of!(__rodata_end) as usize,
            );
            let data = (
                addr_of!(__data_start) as usize,
                addr_of!(__data_end) as usize,
            );
            let bss = (addr_of!(__bss_start) as usize, addr_of!(__bss_end) as usize);
            let heap = (
                KHEAP_START as usize,
                (KHEAP_START + KHEAP_SIZE as u64) as usize,
            );

            in_any_range(addr, len, &[text, rod, data, bss, heap])
        }
    }

    fn can_write(&self, addr: usize, len: usize) -> bool {
        unsafe {
            use core::ptr::addr_of;
            let data = (
                addr_of!(__data_start) as usize,
                addr_of!(__data_end) as usize,
            );
            let bss = (addr_of!(__bss_start) as usize, addr_of!(__bss_end) as usize);
            let heap = (
                KHEAP_START as usize,
                (KHEAP_START + KHEAP_SIZE as u64) as usize,
            );

            in_any_range(addr, len, &[data, bss, heap])
        }
    }
}
