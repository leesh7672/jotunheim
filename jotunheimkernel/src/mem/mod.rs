pub mod mapper;
pub mod reserved;
pub mod simple_alloc;

extern crate alloc;
use alloc::alloc::alloc_zeroed;
use core::sync::atomic::{AtomicU64, Ordering};
use core::{
    alloc::{GlobalAlloc, Layout},
    sync::atomic::AtomicBool,
};
use linked_list_allocator::Heap as LlHeap;
use spin::{Mutex, MutexGuard};
use x86_64::registers::control::Cr0Flags;
use x86_64::structures::paging::{FrameDeallocator, PageTableFlags as F, Translate};
use x86_64::{
    structures::paging::{
        mapper::MapperFlush, FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable,
        PageTableFlags, PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

use crate::bootinfo::BootInfo;
use crate::kprintln;

const PAGE_SIZE: usize = 4096;
const VMAP_BASE: u64 = 0xffff_e000_0000_0000;

static NEXT_VMAP: AtomicU64 = AtomicU64::new(VMAP_BASE);
static mut PHYS_TO_VIRT_OFFSET: u64 = 0;
static HEAP_READY: AtomicBool = AtomicBool::new(false);
static FRAME_ALLOC: Mutex<Option<simple_alloc::TinyBump>> = Mutex::new(None);

// ── Heap window (separate from HHDM!) ────────────────────────────────────────
pub const KHEAP_START: u64 = 0xffff_c000_0000_0000; // moved out of HHDM
pub const KHEAP_SIZE: usize = 16 * 1024 * 1024;

// ── MMIO window (separate VA space; 4 KiB mappings with NO_CACHE) ──────────
const MMIO_BASE: u64 = 0xffff_d000_0000_0000;
static NEXT_MMIO_VA: AtomicU64 = AtomicU64::new(MMIO_BASE);

fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + (a - 1)) & !(a - 1)
}

pub fn virt_to_phys_pt(va: u64) -> Option<u64> {
    let mut mapper = active_mapper();
    mapper
        .translate_addr(VirtAddr::new(va))
        .map(|pa| pa.as_u64())
}

pub fn init(boot: &BootInfo) {
    let off = boot.hhdm_base;
    if (off & 0xfff) != 0 {
        kprintln!("[mem] BUG: hhdm_base not 4K aligned: {:#x}", off);
        loop {}
    }
    unsafe {
        PHYS_TO_VIRT_OFFSET = off;
    }

    let start = align_down(boot.early_heap_paddr, 0x1000);
    let end = align_up(boot.early_heap_paddr + boot.early_heap_len, 0x1000);
    *FRAME_ALLOC.lock() = Some(simple_alloc::TinyBump::new(start, end));

    use x86_64::registers::control::Cr0;
    unsafe { Cr0::write(Cr0::read() | Cr0Flags::WRITE_PROTECT) }
}

pub fn active_mapper() -> OffsetPageTable<'static> {
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

/// Identity-map a single 4 KiB physical page at the same VA (PA==VA).
/// Useful for pages the AP will access by physical pointer (trampoline, ApBoot).
pub fn map_identity_4k(phys: u64, flags: PageTableFlags) {
    let mut mapper = active_mapper();
    let mut fa = TinyAllocGuard::new().expect("map_identity_4k: no early frames");
    let pa = phys & !0xfff;
    let va = pa;
    map_4k(&mut mapper, va, pa, flags, &mut fa);
}

pub fn init_heap() {
    let bytes = KHEAP_SIZE;
    let mut mapper = active_mapper(); // safe here: call init_heap() only after mem::init()
    let mut fa = TinyAllocGuard::new().expect("premap_kheap_head: TinyBump not ready");

    let pages = ((bytes + 4095) / 4096).max(1);
    for i in 0..pages {
        let va = KHEAP_START + (i as u64) * 4096;
        let pf = fa
            .allocate_frame()
            .expect("premap_kheap_head: out of frames");
        map_4k(
            &mut mapper,
            va,
            pf.start_address().as_u64(),
            F::PRESENT | F::WRITABLE | F::GLOBAL | F::NO_EXECUTE,
            &mut fa,
        );
        unsafe {
            GLOBAL_ALLOC.init(KHEAP_START as *mut u8, KHEAP_SIZE);
        }
        HEAP_READY.store(true, Ordering::SeqCst);
    }
}

/// TRUE once GLOBAL_ALLOC is initialized and the heap VA range is mapped.
pub fn heap_ready() -> bool {
    HEAP_READY.load(Ordering::SeqCst)
}

/// Heap-backed bytes inside KHEAP. No page-table mapping here.
/// Returns a zeroed, page-aligned block from the global allocator.
pub fn heap_alloc_bytes(bytes: usize, align: usize) -> Option<*mut u8> {
    if !heap_ready() {
        return None;
    }
    let layout = Layout::from_size_align(bytes, align.max(1)).ok()?;
    let p = unsafe { alloc_zeroed(layout) } as *mut u8;
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

/// VMAP-backed anonymous pages outside KHEAP. Does its own VA reservation + PFN mapping.
/// Never calls the heap allocator.
pub fn vmap_alloc_pages(pages: usize) -> Option<*mut u8> {
    let bytes = pages.checked_mul(PAGE_SIZE)? as u64;
    let base = NEXT_VMAP.fetch_add(bytes, Ordering::SeqCst);

    let mut mapper = active_mapper();
    let mut fa = TinyAllocGuard::new()?;

    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL;

    let mut off = 0u64;
    while off < bytes {
        let pf = fa.allocate_frame()?;
        map_4k(
            &mut mapper,
            base + off,
            pf.start_address().as_u64(),
            flags,
            &mut fa,
        );
        off += Size4KiB::SIZE as u64;
    }
    Some(base as *mut u8)
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

struct NoopDealloc;
impl FrameDeallocator<Size4KiB> for NoopDealloc {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {}
}
pub struct PagingHeap {
    inner: Mutex<LlHeap>,
    mapped_end: AtomicU64, // [KHEAP_START .. mapped_end) is backed by frames
}

impl PagingHeap {
    pub const fn empty() -> Self {
        Self {
            inner: Mutex::new(LlHeap::empty()),
            mapped_end: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        unsafe { self.inner.lock().init(start, size) };
        self.mapped_end.store(KHEAP_START, Ordering::SeqCst);
    }

    fn ensure_mapped(&self, need_end: u64) {
        let cur = self.mapped_end.load(Ordering::Acquire);
        if cur >= need_end {
            return;
        }

        let mut mapper = active_mapper();
        let mut fa = TinyAllocGuard::new().expect("heap map: TinyBump not ready");

        // map 4KiB pages from current watermark up to need_end (rounded up)
        let mut va = cur & !0xfff;
        if va < KHEAP_START {
            va = KHEAP_START;
        }
        let end = (need_end + 0xfff) & !0xfff;

        while va < end {
            // skip if already mapped (safe if you pre-mapped some pages)
            if mapper.translate_addr(VirtAddr::new(va)).is_none() {
                let pf = fa.allocate_frame().expect("heap map: out of frames");
                unsafe {
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
                    let flush = mapper
                        .map_to(
                            page,
                            pf,
                            F::PRESENT | F::WRITABLE | F::GLOBAL | F::NO_EXECUTE,
                            &mut fa,
                        )
                        .expect("heap map_to failed");
                    flush.flush();
                }
            }
            va += 4096;
        }

        self.mapped_end.store(need_end, Ordering::Release);
    }
}

unsafe impl GlobalAlloc for PagingHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Map *ahead* so the inner heap can safely write headers anywhere it chooses.
        let size = layout.size().max(core::mem::size_of::<usize>());
        let headroom = 4096; // space for allocator metadata/splits
        let need = (size + headroom + 0xfff) & !0xfff;

        let cur = self.mapped_end.load(Ordering::Acquire);
        self.ensure_mapped(cur.saturating_add(need as u64));

        // Now hand out memory from the inner heap
        let mut heap = self.inner.lock();
        match heap.allocate_first_fit(layout) {
            Ok(nn) => nn.as_ptr(),
            Err(_) => {
                // try to grow more and retry once (useful under fragmentation)
                self.ensure_mapped(cur.saturating_add(need as u64 + (1 << 20))); // +1 MiB
                heap.allocate_first_fit(layout)
                    .ok()
                    .map_or(core::ptr::null_mut(), |nn| nn.as_ptr())
            }
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.inner
            .lock()
            .deallocate(core::ptr::NonNull::new_unchecked(ptr), layout)
    }
}

#[global_allocator]
static GLOBAL_ALLOC: PagingHeap = PagingHeap::empty();
