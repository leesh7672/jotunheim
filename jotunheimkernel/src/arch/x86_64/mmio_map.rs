use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags as F,
    PhysFrame, Size2MiB, Size4KiB,
    mapper::{MapToError, MapperFlush},
};
use x86_64::{PhysAddr, VirtAddr};

/// Map a single 4KiB page identity with UC (PCD|PWT), RW, NX.
pub fn map_identity_uc<M, A>(
    mapper: &mut M,
    alloc: &mut A,
    phys: u64,
) -> Result<(), MapToError<Size4KiB>>
where
    M: Mapper<Size4KiB>,
    A: FrameAllocator<Size4KiB>,
{
    let pa = PhysAddr::new(phys & !0xfffu64);
    let va = VirtAddr::new(pa.as_u64()); // identity
    let page = Page::<Size4KiB>::containing_address(va);
    let frame = PhysFrame::<Size4KiB>::containing_address(pa);

    // UC via PCD|PWT; also mark GLOBAL to avoid unnecessary flushes
    let flags =
        F::PRESENT | F::WRITABLE | F::NO_EXECUTE | F::WRITE_THROUGH | F::NO_CACHE | F::GLOBAL;
    unsafe { mapper.map_to(page, frame, flags, alloc)? }.flush();
    Ok(())
}
