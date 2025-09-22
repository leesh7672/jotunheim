// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
// src/sched/exec.rs

use heapless::Deque;
use spin::{Mutex, Once};
use x86_64::instructions::hlt;

// Tune as needed
const QUEUE_CAPACITY: usize = 64; // max pending closures (early AP)
const SLOT_SIZE: usize = 128; // max capture size (bytes) for early-boot closures

/// One queued closure, “erased” into a fixed buffer.
/// No heap and no raw-pointer fields that break Send/Sync.

unsafe fn slot_call<F: FnOnce() + 'static>(p: *mut u8) {
    let f: F = unsafe { core::ptr::read_unaligned(p.cast()) };
    f();
}

/// Drops an in-place F stored at p (if you ever need it).
unsafe fn slot_drop_in_place<F: 'static>(p: *mut u8) {
    unsafe { core::ptr::drop_in_place::<F>(p.cast()) };
}

// --- your Slot type ---

struct Slot {
    call: unsafe fn(*mut u8),
    drop_in_place: unsafe fn(*mut u8),
    buf: [u8; SLOT_SIZE],
}

impl Slot {
    fn invoke_and_forget(self) {
        // `call` expects the exact type we wrote into `buf` in `into_slot`.
        unsafe { (self.call)(self.buf.as_ptr() as *mut u8) };
    }
}

// --- into_slot implementation ---

fn into_slot<F>(f: F) -> Result<Slot, ()>
where
    F: FnOnce() + Send + 'static,
{
    if core::mem::size_of::<F>() > SLOT_SIZE {
        return Err(());
    }

    let mut buf = [0u8; SLOT_SIZE];

    // Move closure bytes into fixed buffer.
    unsafe {
        let mut tmp: core::mem::MaybeUninit<F> = core::mem::MaybeUninit::uninit();
        core::ptr::write(tmp.as_mut_ptr(), f);
        core::ptr::copy_nonoverlapping(
            tmp.as_ptr().cast::<u8>(),
            buf.as_mut_ptr(),
            core::mem::size_of::<F>(),
        );
        // `tmp` is intentionally not dropped (bytes moved into `buf`)
    }

    // NOTE: named fields, and the function items are monomorphised here:
    Ok(Slot {
        call: slot_call::<F>,
        drop_in_place: slot_drop_in_place::<F>,
        buf,
    })
}

// ===== Global queue + single serving thread =====

static QUEUE: Mutex<Deque<Slot, QUEUE_CAPACITY>> = Mutex::new(Deque::new());
static STARTED: Once<()> = Once::new();

/// Call once when the scheduler is up (e.g., end of `sched::init()`).
/// Spawns one server thread that turns queued slots into `sched::spawn(closure)`d threads.
pub fn init() {
    STARTED.call_once(|| {
        // Your public scheduler API takes closures — perfect.
        crate::sched::spawn(|| server_main());
    });
}

/// Early-AP safe: capture closure into a fixed-size slot and enqueue it.
/// No `spawn()` here; the server thread will call `spawn()` as soon as it runs.
/// Returns `Err(())` if the closure is too large or the queue is full.
pub fn submit<F>(f: F) -> Result<(), ()>
where
    F: FnOnce() + Send + 'static,
{
    let slot = into_slot(f)?;
    let mut q = QUEUE.lock();
    if q.push_back(slot).is_err() {
        return Err(()); // queue full; caller can retry or drop
    }
    Ok(())
}

fn server_main() -> ! {
    loop {
        // Drain everything available; for each slot, spawn a *new* thread.
        while let Some(slot) = QUEUE.lock().pop_front() {
            crate::sched::spawn(move || {
                slot.invoke_and_forget();
            });
        }
        // Timer-driven scheduler will wake us soon.
        hlt();
    }
}