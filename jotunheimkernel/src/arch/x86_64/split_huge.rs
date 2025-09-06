// at top
use x86_64::structures::paging::{
    FrameAllocator,
    Mapper,
    Page,
    PageSize, // <— for ::SIZE
    PageTable,
    PageTableFlags as F,
    PhysFrame,
    Size2MiB,
    Size4KiB,
    mapper::MapToError,
    mapper::MapperFlush, // <— moved
};

use x86_64::{PhysAddr, VirtAddr};

/// Replace a single 2MiB mapping that covers `addr` with a 4KiB page table
/// mapping the same 2MiB range (WB), so you can then override single pages.
pub fn split_huge_2m<M, A>(
    mapper: &mut M,
    alloc: &mut A,
    addr: u64,
) -> Result<(), MapToError<Size4KiB>>
where
    M: Mapper<Size4KiB> + Mapper<Size2MiB>,
    A: FrameAllocator<Size4KiB>,
{
    let va = VirtAddr::new(addr);
    let huge_page = Page::<Size2MiB>::containing_address(va);

    // Grab the current 2MiB mapping (if any)
    if let Ok(flush) = unsafe { mapper.unmap(huge_page) } {
        flush.1.flush();
        // Back the 2MiB range with a brand new 4KiB page table:
        let base = huge_page.start_address().as_u64();
        for i in 0..(Size2MiB::SIZE / Size4KiB::SIZE) {
            let off = i * Size4KiB::SIZE;
            let page4 = Page::<Size4KiB>::containing_address(VirtAddr::new(base + off));
            let frame4 = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(base + off));
            let flags = F::PRESENT | F::WRITABLE | F::GLOBAL; // WB default
            unsafe { mapper.map_to(page4, frame4, flags, alloc)? }.ignore();
        }
    }
    Ok(())
}
