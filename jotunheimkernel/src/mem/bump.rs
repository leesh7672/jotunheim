use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use spin::Mutex;

struct Bump {
    start: usize,
    end: usize,
    next: usize,
}

static ALLOC: Mutex<Option<Bump>> = Mutex::new(None);

pub fn init(paddr: usize, len: usize) {
    *ALLOC.lock() = Some(Bump {
        start: paddr,
        end: paddr + len,
        next: paddr,
    });
}

unsafe impl GlobalAlloc for LockedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(ref mut bump) = *ALLOC.lock() {
            let align = layout.align();
            let size = layout.size();
            let next = (bump.next + (align - 1)) & !(align - 1);
            let end = next.saturating_add(size);
            if end <= bump.end {
                bump.next = end;
                return next as *mut u8;
            }
        }
        null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) { /* no-op */
    }
}

pub struct LockedAlloc;
#[global_allocator]
static GLOBAL: LockedAlloc = LockedAlloc;
