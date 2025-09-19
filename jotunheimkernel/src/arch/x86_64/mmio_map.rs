// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
use x86_64::{
    VirtAddr,
    structures::paging::{Mapper, Page, PageTableFlags as F, Size4KiB},
};

fn enforce_mmio_flags<M: Mapper<Size4KiB>>(mapper4k: &mut M, va: u64) {
    let page4k = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
    let want = F::PRESENT | F::WRITABLE | F::NO_EXECUTE | F::WRITE_THROUGH | F::NO_CACHE;
    if let Ok(flush) = unsafe { mapper4k.update_flags(page4k, want) } {
        flush.flush();
    }
}

pub fn enforce_apic_mmio_flags() {
    let mut mapper = crate::mem::active_mapper(); // call only after mem::init()
    enforce_mmio_flags(&mut mapper, 0xFEC0_0000);
    enforce_mmio_flags(&mut mapper, 0xFEE0_0000);
}
