#![allow(unused)]

use crate::arch::x86_64::split_huge::split_huge_2m;
use crate::mem::mapper::active_offset_mapper;
use crate::mem::simple_alloc::TinyBump;

use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags as F, PhysFrame, Size4KiB, mapper::MapToError,
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
/// Map LAPIC (0xFEE0_0000) + IOAPIC (0xFEC0_0000) as present, writable, UC.
/// Must be called BEFORE apic::init() when running without x2APIC.
pub fn early_map_mmio_for_apics() {
    // Small scratch region for page tables (adjust if you need more)
    let mut alloc = TinyBump::new(0x0030_0000, 0x0031_0000);

    // Get the active offset mapper; 0 if your phys=virt for low memory
    let mut mapper = unsafe { active_offset_mapper(0) };

    // Split any covering 2MiB huge pages so the 4KiB MMIO pages can be mapped
    let _ = split_huge_2m(&mut mapper, &mut alloc, 0xFEC0_0000);
    let _ = split_huge_2m(&mut mapper, &mut alloc, 0xFEE0_0000);

    // Identity map IOAPIC + LAPIC as uncacheable
    let _ = map_identity_uc(&mut mapper, &mut alloc, 0xFEC0_0000);
    let _ = map_identity_uc(&mut mapper, &mut alloc, 0xFEE0_0000);
}
