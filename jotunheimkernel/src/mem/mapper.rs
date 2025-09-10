use crate::println;

use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

#[inline(always)]
fn is_page_aligned(x: u64) -> bool {
    (x & 0xfff) == 0
}

/// Build a mapper using a validated HHDM offset, without panicking.
pub fn active_offset_mapper(hhdm: u64) -> Result<OffsetPageTable<'static>, &'static str> {
    if !is_page_aligned(hhdm) {
        println!("[mapper] HHDM not 4K aligned: {:#x}", hhdm);
        return Err("misaligned hhdm");
    }

    // 1) HHDM base must itself be canonical
    let hhdm_va = VirtAddr::try_new(hhdm).map_err(|_| {
        println!("[mapper] HHDM not canonical: {:#x}", hhdm);
        "non-canonical hhdm"
    })?;

    // 2) Compute L4 virtual address through HHDM and validate
    let (l4_frame, _) = Cr3::read();
    let l4_phys = l4_frame.start_address().as_u64();
    if !is_page_aligned(l4_phys) {
        println!("[mapper] L4 phys not page-aligned: {:#x}", l4_phys);
        return Err("l4 phys misaligned");
    }

    let l4_virt_u = l4_phys.wrapping_add(hhdm);
    let l4_virt_va = VirtAddr::try_new(l4_virt_u).map_err(|_| {
        println!(
            "[mapper] L4 virt not canonical: phys={:#x} hhdm={:#x} sum={:#x}",
            l4_phys, hhdm, l4_virt_u
        );
        "non-canonical l4 virt"
    })?;
    if !is_page_aligned(l4_virt_va.as_u64()) {
        println!(
            "[mapper] L4 virt not 4K aligned: {:#x}",
            l4_virt_va.as_u64()
        );
        return Err("l4 virt misaligned");
    }

    // 3) Touch the page to prove the mapping exists (avoids creating a &mut first)
    let peek = unsafe { core::slice::from_raw_parts(l4_virt_va.as_ptr(), 8) };
    let _: u8 = peek[0];

    // 4) Now create the reference and mapper
    let l4: &mut PageTable = unsafe { &mut *(l4_virt_va.as_mut_ptr()) };
    let mapper = unsafe { OffsetPageTable::new(l4, hhdm_va) };
    Ok(mapper)
}
