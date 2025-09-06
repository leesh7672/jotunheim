// src/mem/mapper.rs
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

/// If your paging is identity, use offset=0. If you use a phys->virt offset, pass it.
pub unsafe fn active_offset_mapper(phys_to_virt_offset: u64) -> OffsetPageTable<'static> {
    let (level_4_frame, _) = Cr3::read();
    let phys = level_4_frame.start_address().as_u64();
    let virt = VirtAddr::new(phys + phys_to_virt_offset);
    let l4_table: &mut PageTable = unsafe { &mut *virt.as_mut_ptr() };
    unsafe { OffsetPageTable::new(l4_table, VirtAddr::new(phys_to_virt_offset))
}
