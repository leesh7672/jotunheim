#![allow(dead_code)]

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Framebuffer {
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: u32, // 0 = RGB, 1 = BGR
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MemoryKind {
    Usable = 1,
    Reserved = 2,
    AcpiReclaimable = 3,
    AcpiNvs = 4,
    Mmio = 5,
    Bootloader = 6,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BootInfo {
    pub rsdp_addr: u64,
    pub memory_map: *const MemoryRegion,
    pub memory_map_len: usize,
    pub framebuffer: Framebuffer,
    pub kernel_phys_base: u64,
    pub kernel_virt_base: u64,
    pub early_heap_paddr: u64,
    pub early_heap_len: u64,
}
