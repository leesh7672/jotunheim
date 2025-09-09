// allocator/heap.rs
use core::ptr::NonNull;
use linked_list_allocator::LockedHeap;
use crate::paging::{PageMapper, MapFlags}; // your own types

// Choose a high, canonical kernel address window (example values):
pub const KHEAP_START: u64 = 0xffff_8880_0000_0000;
pub const KHEAP_SIZE:  usize = 16 * 1024 * 1024; // 16 MiB to start

#[global_allocator]
static GLOBAL: LockedHeap = LockedHeap::empty();

pub fn heap_init(mapper: &mut impl PageMapper) {
    // Map [KHEAP_START .. KHEAP_START+KHEAP_SIZE) RW, WB
    let mut mapped = 0;
    while mapped < KHEAP_SIZE {
        let va = KHEAP_START + mapped as u64;

        // one 4 KiB page at a time (could do huge pages if aligned)
        let pf = super::frames::alloc_frame()
            .expect("heap_init: out of early frames");
        mapper.map_page(va, pf.0, MapFlags::PRESENT | MapFlags::WRITABLE | MapFlags::GLOBAL | MapFlags::WB);

        mapped += 0x1000;
    }

    unsafe {
        GLOBAL.lock().init(KHEAP_START as usize, KHEAP_SIZE);
    }
}
