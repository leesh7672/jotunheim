pub mod reserved;
pub mod simple_alloc;

extern crate alloc;
use core::sync::atomic::{fence, AtomicU64, Ordering};
use core::{
    alloc::{GlobalAlloc, Layout},
    sync::atomic::AtomicBool,
};
use heapless::Vec as HVec;
use linked_list_allocator::Heap as LlHeap;
use spin::{Mutex, MutexGuard};
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::registers::control::Cr0Flags;
use x86_64::structures::paging::{PageTableFlags as F, Translate};
use x86_64::{
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags,
        PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

static PT_LOCK: Mutex<()> = Mutex::new(());

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
pub const KHEAP_SIZE: usize = 32 * 1024 * 1024;

// ── MMIO window (separate VA space; 4 KiB mappings with NO_CACHE) ──────────
const MMIO_BASE: u64 = 0xffff_d000_0000_0000;
static NEXT_MMIO_VA: AtomicU64 = AtomicU64::new(MMIO_BASE);

fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + (a - 1)) & !(a - 1)
}

fn pt_locked<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    without_interrupts(|| {
        let g = PT_LOCK.lock();
        let r: R = f();
        drop(g);
        r
    })
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

    if boot.low32_pool_len >= 0x1000 {
        let lstart = align_down(boot.low32_pool_paddr, 0x1000);
        let lend = align_up(boot.low32_pool_paddr + boot.low32_pool_len, 0x1000);
        *LOW32_ALLOC.lock() = Some(simple_alloc::TinyBump::new(lstart, lend));
    }
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
    flags: F,
    fa: &mut impl FrameAllocator<Size4KiB>,
) {
    pt_locked(|| {
        use x86_64::{structures::paging::*, PhysAddr, VirtAddr};
        let pa_aligned = (pa_mask_52(pa)) & !0xFFF;
        let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(pa_aligned));
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
        unsafe {
            mapper.map_to(page, frame, flags, fa).unwrap().flush();
        }
    })
}

const fn pa_mask_52(x: u64) -> u64 {
    // Keep only bits 0..=51 (52-bit physical address space)
    x & 0x000F_FFFF_FFFF_FFFF
}

/// Map a physical MMIO region at a dedicated VA (not inside HHDM), 4 KiB pages, NO_CACHE.
/// Returns the VA base address.
pub fn map_mmio(pa: u64, len: usize) -> u64 {
    pt_locked(|| {
        let pa0 = pa_mask_52(pa) & !0xFFF;
        let pend = pa_mask_52(pa + len as u64 + 0xFFF) & !0xFFF;
        let size = pend - pa0;
        let off = pa - pa0;

        let va0 = NEXT_MMIO_VA.fetch_add(size, Ordering::SeqCst);

        let mut mapper = active_mapper();
        let mut fa = TinyAllocGuard::new().expect("map_mmio: no frames");
        let flags = F::PRESENT | F::WRITABLE | F::NO_CACHE | F::NO_EXECUTE;

        let mut pa_cur = pa0;
        let mut va_cur = va0;
        while pa_cur < pend {
            // SAFETY: pa_cur is masked to 52 bits and 4K aligned
            let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(pa_cur));
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va_cur));
            unsafe {
                mapper.map_to(page, frame, flags, &mut fa).unwrap().flush();
            }
            pa_cur += 0x1000;
            va_cur += 0x1000;
        }
        va0 + off
    })
}

pub fn map_identity_4k(phys: u64) {
    pt_locked(|| {
        let mut mapper = active_mapper();
        let mut fa = TinyAllocGuard::new().expect("idmap4k: no frames");
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(phys));
        let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys));
        unsafe {
            match mapper.map_to(page, frame, F::PRESENT | F::WRITABLE | F::GLOBAL, &mut fa) {
                Ok(flush) => flush.flush(),
                Err(x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_)) => {}
                Err(e) => panic!("idmap4k({:#x}) failed: {:?}", phys, e),
            }
        }
    })
}

pub fn alloc_one_phys_page_hhdm() -> (u64, u64) {
    let mut guard = LOW32_ALLOC.lock();
    let bump = guard.as_mut().expect("low32 allocator not seeded");
    let pf = bump.allocate_frame().expect("no low32 frame available");
    let pa = pf.start_address().as_u64();
    let va = pa + unsafe { PHYS_TO_VIRT_OFFSET };
    unsafe { core::ptr::write_bytes(va as *mut u8, 0, 4096) };
    (va, pa)
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
    }
    unsafe {
        GLOBAL_ALLOC.init(KHEAP_START as *mut u8, KHEAP_SIZE);
    }
    HEAP_READY.store(true, Ordering::SeqCst);
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
        if let Some(a) = self.lock.as_mut() {
            if let Some(pf) = a.allocate_frame() {
                return Some(pf);
            }
        }
        fallback_take_frame()
    }
}

struct MutexHeap{
    inner: Mutex<PagingHeap>
}

impl MutexHeap {
    fn init(&self, start: *mut u8, size: usize){
        unsafe { self.inner.lock().init(start, size) };
    }
    const fn new() -> Self{
        Self{inner: Mutex::new(PagingHeap::empty())}
    }
}

unsafe impl GlobalAlloc for MutexHeap {
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { self.inner.lock().alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {self.inner.lock().realloc(ptr, layout, new_size)}
    }
    
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {self.inner.lock().alloc(layout)}
    }
    
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {self.inner.lock().dealloc(ptr, layout)}
    }
}

struct PagingHeap {
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
    fn ensure_mapped_span(&self, start: u64, end: u64) {
        pt_locked(|| {
            let mut mapper = active_mapper();
            let mut fa = TinyAllocGuard::new().expect("heap map: TinyBump not ready");

            let mut va = start & !0xfff;
            let end_al = (end + 0xfff) & !0xfff;
            while va < end_al {
                if mapper.translate_addr(VirtAddr::new(va)).is_none() {
                    let pf = fa.allocate_frame().expect("heap map: out of frames");
                    unsafe {
                        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
                        match mapper.map_to_with_table_flags(
                            page,
                            pf,
                            F::PRESENT | F::WRITABLE | F::GLOBAL | F::NO_EXECUTE,
                            F::PRESENT | F::WRITABLE,
                            &mut fa,
                        ) {
                            Ok(flush) => {
                                flush.flush();
                                fence(Ordering::SeqCst);
                            }
                            Err(
                                x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(
                                    _,
                                ),
                            ) => {
                                // Another thread mapped it since our translate; just ensure flags.
                                mapper
                                    .update_flags(
                                        page,
                                        F::PRESENT | F::WRITABLE | F::GLOBAL | F::NO_EXECUTE,
                                    )
                                    .unwrap()
                                    .flush();
                            }
                            Err(e) => panic!("heap map_to failed @va={:#x}: {:?}", va, e),
                        }
                    }
                }
                va += 4096;
            }
        })
    }

    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        unsafe { self.inner.lock().init(start, size) };
        self.mapped_end.store(KHEAP_START, Ordering::SeqCst);
    }
}

unsafe impl GlobalAlloc for PagingHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        without_interrupts(|| {
            let mut heap = self.inner.lock();
            if let Ok(nn) = heap.allocate_first_fit(layout) {
                let p = nn.as_ptr();
                let size = layout.size().max(1);
                // map exactly what the caller will touch: [p, p+size)
                self.ensure_mapped_span(p as u64, (p as u64).saturating_add(size as u64));
                return p;
            }
            drop(heap);

            let cur = self.mapped_end.load(Ordering::Acquire);
            let grow = 1u64 << 20;
            let end = cur.saturating_add(grow);
            self.ensure_mapped_span(cur, end);
            self.mapped_end.store(end, Ordering::Release);

            let mut heap = self.inner.lock();
            match heap.allocate_first_fit(layout) {
                Ok(nn) => {
                    let p = nn.as_ptr();
                    let size = layout.size().max(1);
                    self.ensure_mapped_span(p as u64, (p as u64).saturating_add(size as u64));
                    p
                }
                Err(_) => core::ptr::null_mut(),
            }
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        without_interrupts(|| unsafe {
            self.inner
                .lock()
                .deallocate(core::ptr::NonNull::new_unchecked(ptr), layout)
        })
    }
}

#[global_allocator]
static GLOBAL_ALLOC: MutexHeap = MutexHeap::new();
static LOW32_ALLOC: spin::Mutex<Option<simple_alloc::TinyBump>> = Mutex::new(None);

const MAX_USABLE: usize = 256;
static USABLE: Mutex<HVec<(u64, u64), MAX_USABLE>> = Mutex::new(HVec::new()); // [(start,end))

pub fn seed_usable_from_mmap(boot: &BootInfo) {
    let mm = unsafe { core::slice::from_raw_parts(boot.memory_map, boot.memory_map_len) };
    let mut v = USABLE.lock();
    *v = HVec::new();
    for mr in mm {
        if mr.typ != 1 {
            continue;
        } // only usable RAM
        let s = (mr.phys_start + 0xfff) & !0xfff;
        let e = (mr.phys_start + mr.len) & !0xfff;
        if e <= s {
            continue;
        }
        // skip reserved holes inside
        // we’ll clip simple overlaps out by stepping 4KiB at allocation time
        v.push((s, e)).ok();
    }
}

// Take one 4KiB frame from the USABLE list, skipping reserved pages.
fn fallback_take_frame() -> Option<PhysFrame<Size4KiB>> {
    let mut v = USABLE.lock();
    while let Some((mut s, e)) = v.pop() {
        while s + 0x1000 <= e {
            let cand = s;
            s += 0x1000;
            if !crate::mem::reserved::is_reserved_page(cand) {
                // put back remainder
                if s < e {
                    let _ = v.push((s, e));
                }
                return Some(PhysFrame::containing_address(PhysAddr::new(cand)));
            }
        }
        // exhausted this range; continue to next
    }
    None
}