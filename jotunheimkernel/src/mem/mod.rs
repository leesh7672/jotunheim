pub mod bump;
pub mod mapper;
pub mod simple_alloc;

use spin::Mutex;
use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB,
    },
};

pub const PAGE_SIZE: usize = 4096;

static mut PHYS_TO_VIRT_OFFSET: u64 = 0;
static FRAME_ALLOC: Mutex<Option<simple_alloc::TinyBump>> = Mutex::new(None);

pub fn init(phys_free_start: u64, phys_free_end: u64, phys_to_virt_offset: u64) {
    *FRAME_ALLOC.lock() = Some(simple_alloc::TinyBump::new(phys_free_start, phys_free_end));
    unsafe {
        PHYS_TO_VIRT_OFFSET = phys_to_virt_offset;
    }
}

fn active_mapper() -> OffsetPageTable<'static> {
    unsafe { mapper::active_offset_mapper(PHYS_TO_VIRT_OFFSET) }
}

// NOTE: early bring-up: contiguous PA via TinyBump, map at VA=PA+offset
pub unsafe fn alloc_pages(pages: usize) -> Option<*mut u8> {
    unsafe {
        let mut fa = FRAME_ALLOC.lock();
        let fa = fa.as_mut()?;
        let first = fa.allocate_frame()?;
        let mut map = active_mapper();
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;

        let mut frame = first;
        let mut va_u64 = frame.start_address().as_u64() + PHYS_TO_VIRT_OFFSET;
        let mut page = Page::<Size4KiB>::containing_address(VirtAddr::new(va_u64));
        map.map_to(page, frame, flags, &mut DummyAlloc)
            .ok()?
            .flush();

        for _ in 1..pages {
            let next = fa.allocate_frame()?;
            // require contiguity for simplicity
            if next.start_address().as_u64() != frame.start_address().as_u64() + 0x1000 {
                return None;
            }
            frame = next;
            va_u64 += 0x1000;
            page = Page::<Size4KiB>::containing_address(VirtAddr::new(va_u64));
            map.map_to(page, frame, flags, &mut DummyAlloc)
                .ok()?
                .flush();
        }

        Some((first.start_address().as_u64() + PHYS_TO_VIRT_OFFSET) as *mut u8)
    }
}

pub fn unmap_pages(base: *mut u8, pages: usize) -> Result<(), ()> {
    let mut map = active_mapper();
    for i in 0..pages {
        let va = (base as u64) + (i as u64) * 0x1000;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
        map.unmap(page).map_err(|_| ())?.1.flush();
    }
    Ok(())
}

pub unsafe fn free_pages(_base: *mut u8, _pages: usize) {
    // TinyBump has no free; okay for bring-up.
}

struct DummyAlloc;
unsafe impl FrameAllocator<Size4KiB> for DummyAlloc {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        None
    }
}
