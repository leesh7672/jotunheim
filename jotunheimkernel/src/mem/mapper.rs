use crate::kprintln;

use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

#[inline(always)]
fn is_page_aligned(x: u64) -> bool {
    (x & 0xfff) == 0
}

// src/mem/mapper.rs
pub unsafe fn active_offset_mapper(hhdm: u64) -> Result<OffsetPageTable<'static>, &'static str> {
    use x86_64::{VirtAddr, registers::control::Cr3};

    // This must be canonical; you already asserted before calling.
    let hhdm_va = VirtAddr::new(hhdm);

    let (l4_frame, _flags) = Cr3::read();
    let phys = l4_frame.start_address();
    let l4_ptr = (hhdm_va.as_u64() + phys.as_u64()) as *mut PageTable;

    Ok(unsafe { OffsetPageTable::new(&mut *l4_ptr, hhdm_va) })
}
