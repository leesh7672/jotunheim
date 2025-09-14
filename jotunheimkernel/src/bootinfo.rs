
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Framebuffer {
    pub addr: u64, // physical address of linear framebuffer
    pub width: u32,
    pub height: u32,
    pub pitch: u32,        // bytes per scanline
    pub bpp: u32,          // bits per pixel (commonly 32)
    pub pixel_format: u32, // kernel enum/discriminant
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MemoryRegion {
    pub phys_start: u64,
    pub virt_start: u64, // 0 at boot (or phys+offset if you prefer)
    pub len: u64,
    pub typ: u32,  // kernel enum/discriminant
    pub attr: u64, // attribute bits
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
    pub hhdm_base: u64,
}
