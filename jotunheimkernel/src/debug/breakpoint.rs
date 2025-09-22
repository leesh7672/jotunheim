// src/debug/breakpoint.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(unsafe_op_in_unsafe_fn)]
use spin::Mutex;
use x86_64::registers::control::{Cr0, Cr0Flags};

#[derive(Copy, Clone)]
struct Bp {
    addr: u64,
    orig: u8,
    armed: bool,
}

const MAX_BP: usize = 64;
static BP_TABLE: Mutex<[Option<Bp>; MAX_BP]> = Mutex::new([None; MAX_BP]);

// Reinsert after single-step?
static REPLANT_AFTER_STEP: Mutex<Option<u64>> = Mutex::new(None);

unsafe fn write_byte(addr: u64, val: u8) {
    (addr as *mut u8).write_volatile(val);
}

unsafe fn read_byte(addr: u64) -> u8 {
    (addr as *const u8).read_volatile()
}

// Temporarily clear CR0.WP so supervisor can patch RO text safely.
fn with_wp_disabled<F: FnOnce()>(f: F) {
    let old = Cr0::read();
    // If WP is already clear, just run f().
    if !old.contains(Cr0Flags::WRITE_PROTECT) {
        f();
        return;
    }
    unsafe {
        Cr0::write(old - Cr0Flags::WRITE_PROTECT);
    }
    f();
    unsafe {
        Cr0::write(old);
    }
}

fn find_slot(addr: u64, tbl: &mut [Option<Bp>; MAX_BP]) -> Option<usize> {
    let mut free: Option<usize> = None;
    for (i, e) in tbl.iter().enumerate() {
        match e {
            Some(bp) if bp.addr == addr => return Some(i),
            None if free.is_none() => free = Some(i),
            _ => {}
        }
    }
    free
}

pub fn insert(addr: u64) -> bool {
    let mut tbl = BP_TABLE.lock();
    let idx = match find_slot(addr, &mut *tbl) {
        Some(i) => i,
        None => return false,
    };
    // Already exists?
    if let Some(bp) = tbl[idx] {
        if bp.armed {
            return true;
        }
    }
    // Patch: read original byte, write 0xCC
    let (orig, ok) = unsafe {
        let o = read_byte(addr);
        let mut good = true;
        with_wp_disabled(|| write_byte(addr, 0xCC));
        if read_byte(addr) != 0xCC {
            good = false;
        }
        (o, good)
    };
    if !ok {
        return false;
    }
    tbl[idx] = Some(Bp {
        addr,
        orig,
        armed: true,
    });
    true
}

pub fn remove(addr: u64) -> bool {
    let mut tbl = BP_TABLE.lock();
    for e in tbl.iter_mut() {
        if let Some(bp) = *e {
            if bp.addr == addr {
                if bp.armed {
                    unsafe {
                        with_wp_disabled(|| write_byte(addr, bp.orig));
                    }
                }
                *e = None;
                return true;
            }
        }
    }
    false
}

// Called right as you enter the debugger on #BP (INT3).
// If RIP==addr+1 for a planted bp, unpatch + rewind, and mark for replant-on-resume/step.
pub fn on_breakpoint_enter(rip: &mut u64) -> Option<u64> {
    let hit_addr = rip.wrapping_sub(1);
    let mut tbl = BP_TABLE.lock();
    for e in tbl.iter_mut() {
        if let Some(bp) = *e {
            if bp.addr == hit_addr && bp.armed {
                // restore original now, and rewind IP
                unsafe {
                    with_wp_disabled(|| write_byte(hit_addr, bp.orig));
                }
                *rip = hit_addr;
                // Mark this bp as temporarily disarmed; we’ll re-plant on continue,
                // or after the single-step completes.
                *e = Some(Bp { armed: false, ..bp });
                return Some(hit_addr);
            }
        }
    }
    None
}

// When user chose "continue": re-arm the most recently hit bp (if any).
pub fn on_resume_continue(last_hit: Option<u64>) {
    if let Some(addr) = last_hit {
        let _ = insert(addr);
    }
}

// When user chose "step": defer replant until the #DB single-step trap.
pub fn on_resume_step(last_hit: Option<u64>) {
    *REPLANT_AFTER_STEP.lock() = last_hit;
}
