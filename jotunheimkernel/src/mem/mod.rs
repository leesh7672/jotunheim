pub mod mapper;
pub mod simple_alloc;

use core::sync::atomic::{AtomicU64, Ordering};
use spin::{Mutex, MutexGuard};
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator,
        Mapper,
        OffsetPageTable,
        Page,
        PageSize,
        PageTable,
        PageTableFlags,
        PhysFrame,
        Size4KiB,
        mapper::MapperFlush, // correct path
    },
};

use crate::bootinfo::BootInfo;
use crate::println;

pub const PAGE_SIZE: usize = 4096;

static mut PHYS_TO_VIRT_OFFSET: u64 = 0;

static FRAME_ALLOC: Mutex<Option<simple_alloc::TinyBump>> = Mutex::new(None);

// ── Heap window (separate from HHDM!) ────────────────────────────────────────
pub const KHEAP_START: u64 = 0xffff_c000_0000_0000; // moved out of HHDM
pub const KHEAP_SIZE: usize = 16 * 1024 * 1024;

// ── MMIO window (separate VA space; 4 KiB mappings with NO_CACHE) ──────────
const MMIO_BASE: u64 = 0xffff_d000_0000_0000;
static NEXT_MMIO_VA: AtomicU64 = AtomicU64::new(MMIO_BASE);

// Single global allocator
#[global_allocator]
static GLOBAL_ALLOC: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

#[inline]
unsafe fn read_phys_u8_slice<'a>(phys: u64, len: usize, off: u64) -> &'a [u8] {
    let va = phys.wrapping_add(off) as *const u8;
    unsafe { core::slice::from_raw_parts(va, len) }
}

unsafe fn validate_hhdm_with_rsdp(candidate_off: u64, rsdp_phys: u64) -> bool {
    if candidate_off & 0xfff != 0 {
        return false;
    }

    let s = unsafe { read_phys_u8_slice(rsdp_phys, 8, candidate_off) };
    if s != b"RSD PTR " {
        return false;
    }

    let v1 = unsafe { read_phys_u8_slice(rsdp_phys, 20, candidate_off) };
    let sum_v1: u8 = v1.iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
    if sum_v1 == 0 {
        return true;
    }

    let len_bytes = unsafe { read_phys_u8_slice(rsdp_phys + 20, 1, candidate_off) };
    let len = len_bytes[0] as usize;
    if len >= 20 && len <= 36 {
        let v2 = unsafe { read_phys_u8_slice(rsdp_phys, len, candidate_off) };
        let sum_v2: u8 = v2.iter().copied().fold(0u8, |a, b| a.wrapping_add(b));
        return sum_v2 == 0;
    }
    false
}

pub fn init(boot: &BootInfo) {
    let off = boot.hhdm_base;
    if (off & 0xfff) != 0 {
        println!("[mem] BUG: hhdm_base not 4K aligned: {:#x}", off);
        loop {}
    }

    match unsafe { crate::mem::mapper::active_offset_mapper(off) } {
        Ok(_) => { /* good */ }
        Err(e) => {
            println!("[mem] active_offset_mapper failed: {}", e);
            let rsdp = (boot.rsdp_addr.wrapping_add(off)) as *const u8;
            let ok = unsafe { core::slice::from_raw_parts(rsdp, 8).eq(b"RSD PTR ") };
            println!("[mem] RSDP via HHDM: sig_ok={}", ok);
            loop {}
        }
    }

    unsafe {
        PHYS_TO_VIRT_OFFSET = off;
    }

    // Seed TinyBump
    let start = boot.early_heap_paddr & !0xfffu64;
    let end = (boot.early_heap_paddr + boot.early_heap_len) & !0xfffu64;
    *FRAME_ALLOC.lock() = Some(simple_alloc::TinyBump::new(start, end));

    let _ = active_mapper();

    println!("[mem] HHDM offset = {:#x}", unsafe { PHYS_TO_VIRT_OFFSET });
}

fn active_mapper() -> OffsetPageTable<'static> {
    unsafe {
        let l4 = active_level4_table_virt();
        OffsetPageTable::new(l4, VirtAddr::new(PHYS_TO_VIRT_OFFSET))
    }
}

fn active_level4_table_virt() -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;
    let (l4_frame, _) = Cr3::read();
    let phys = l4_frame.start_address().as_u64();
    let virt = {
        let off = unsafe { PHYS_TO_VIRT_OFFSET };
        VirtAddr::new(phys + off)
    };
    unsafe { &mut *virt.as_mut_ptr::<PageTable>() }
}

fn map_4k(
    mapper: &mut OffsetPageTable<'static>,
    va: u64,
    pa: u64,
    flags: PageTableFlags,
    fa: &mut impl FrameAllocator<Size4KiB>,
) {
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
    let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(pa));
    unsafe {
        let flush: MapperFlush<Size4KiB> = mapper
            .map_to(page, frame, flags, fa)
            .expect("map_to(4K) failed");
        flush.flush();
    }
}

fn map_range_4k(
    mapper: &mut OffsetPageTable<'static>,
    va: u64,
    pa: u64,
    len: usize,
    flags: PageTableFlags,
    fa: &mut impl FrameAllocator<Size4KiB>,
) {
    let mut off = 0usize;
    while off < len {
        map_4k(mapper, va + off as u64, pa + off as u64, flags, fa);
        off += Size4KiB::SIZE as usize;
    }
}

/// Map a physical MMIO region at a dedicated VA (not inside HHDM), 4 KiB pages, NO_CACHE.
/// Returns the VA base.
pub fn map_mmio(pa: u64, len: usize) -> u64 {
    let size = ((len + (PAGE_SIZE - 1)) / PAGE_SIZE) * PAGE_SIZE;
    let va = NEXT_MMIO_VA.fetch_add(size as u64, Ordering::SeqCst);

    let mut mapper = active_mapper();
    let mut fa = TinyAllocGuard::new().expect("mmio: no early frame allocator");

    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE;

    map_range_4k(&mut mapper, va, pa, size, flags, &mut fa);
    va
}

pub fn init_heap() {
    let mut mapper = active_mapper();
    let mut fa = TinyAllocGuard::new().expect("heap: no early frame allocator");

    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL;

    let mut mapped = 0usize;
    while mapped < KHEAP_SIZE {
        let pf = fa.allocate_frame().expect("heap: out of early frames");
        map_4k(
            &mut mapper,
            KHEAP_START + mapped as u64,
            pf.start_address().as_u64(),
            flags,
            &mut fa,
        );
        mapped += Size4KiB::SIZE as usize;
    }

    unsafe {
        GLOBAL_ALLOC.lock().init(KHEAP_START as *mut u8, KHEAP_SIZE);
    }
}
pub fn alloc_pages(pages: usize) -> Option<*mut u8> {
    let bytes = (pages * PAGE_SIZE) as u64;
    let mut mapper = active_mapper();
    let mut fa = TinyAllocGuard::new()?;

    let mut out_va: u64 = 0;
    let mut off = 0u64;

    while off < bytes {
        let pf = fa.allocate_frame()?;
        let va_this = pf.start_address().as_u64() + unsafe { PHYS_TO_VIRT_OFFSET };
        if out_va == 0 {
            out_va = va_this;
        }
        map_4k(
            &mut mapper,
            va_this,
            pf.start_address().as_u64(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            &mut fa,
        );
        off += Size4KiB::SIZE;
    }

    Some(out_va as *mut u8)
}

pub unsafe fn free_pages(_base: *mut u8, _pages: usize) {}
pub unsafe fn unmap_pages(_base: *mut u8, _pages: usize) {}

#[inline]
pub fn phys_to_virt(pa: u64) -> u64 {
    pa + unsafe { PHYS_TO_VIRT_OFFSET }
}
#[inline]
pub fn virt_to_phys(va: u64) -> u64 {
    va - unsafe { PHYS_TO_VIRT_OFFSET }
}

struct TinyAllocGuard<'a> {
    lock: MutexGuard<'a, Option<simple_alloc::TinyBump>>,
}
impl<'a> TinyAllocGuard<'a> {
    fn new() -> Option<Self> {
        let lock = FRAME_ALLOC.lock();
        if lock.is_some() {
            Some(Self { lock })
        } else {
            None
        }
    }
}
unsafe impl<'a> FrameAllocator<Size4KiB> for TinyAllocGuard<'a> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        match self.lock.as_mut() {
            Some(a) => a.allocate_frame(),
            None => None,
        }
    }
}
