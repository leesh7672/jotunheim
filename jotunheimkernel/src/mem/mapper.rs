use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

/// # Safety
/// - `hhdm` must be the physical→virtual offset for a valid direct map
///   (i.e., VA = PA + hhdm), 4 KiB aligned, and already mapped.
/// - CR3 must point to a valid L4 table.
pub unsafe fn active_offset_mapper(hhdm: u64) -> Result<OffsetPageTable<'static>, &'static str> {
    if hhdm == 0 || (hhdm & 0xfff) != 0 {
        return Err("hhdm must be nonzero and 4KiB-aligned");
    }

    let hhdm_va = VirtAddr::new(hhdm);
    let (l4_frame, _flags) = Cr3::read();
    let l4_phys = l4_frame.start_address().as_u64();

    // L4 virtual address = HHDM offset + physical L4 address
    let l4_virt = hhdm_va + l4_phys;

    if (l4_virt.as_u64() & 0xfff) != 0 {
        return Err("computed L4 VA not 4KiB-aligned");
    }

    // SAFETY: caller guarantees HHDM is mapped and CR3 points to a valid L4.
    let l4_ptr: *mut PageTable = l4_virt.as_mut_ptr();
    let l4_ref: &mut PageTable = unsafe { &mut *l4_ptr };

    Ok(unsafe { OffsetPageTable::new(l4_ref, hhdm_va) })
}

/// # Safety
/// - `hhdm` must be the PA→VA offset (direct map), 4KiB aligned, mapped.
/// - `pml4_phys` is the physical address of the current L4 table.
/// - The L4 page must be visible through the HHDM.
pub unsafe fn mapper_from_boot(
    hhdm: u64,
    pml4_phys: u64,
) -> Result<OffsetPageTable<'static>, &'static str> {
    if hhdm == 0 || (hhdm & 0xfff) != 0 {
        return Err("bad hhdm");
    }
    if (pml4_phys & 0xfff) != 0 {
        return Err("pml4 not 4K aligned");
    }

    let l4_virt = VirtAddr::new(hhdm + pml4_phys);
    let l4_ptr: *mut PageTable = l4_virt.as_mut_ptr();
    Ok(unsafe { OffsetPageTable::new(&mut *l4_ptr, VirtAddr::new(hhdm)) })
}
