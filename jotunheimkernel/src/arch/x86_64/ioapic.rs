// src/arch/x86_64/ioapic.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
const IOAPIC_BASE: usize = 0xFEC0_0000;
const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;


unsafe fn ioregsel() -> *mut u32 {
    (IOAPIC_BASE + IOREGSEL) as *mut u32
}

unsafe fn iowin() -> *mut u32 {
    (IOAPIC_BASE + IOWIN) as *mut u32
}

unsafe fn mmio_write(reg: u32, val: u32) {
    unsafe { core::ptr::write_volatile(ioregsel(), reg) };
    unsafe { core::ptr::write_volatile(iowin(), val) };
}
unsafe fn mmio_read(reg: u32) -> u32 {
    unsafe { core::ptr::write_volatile(ioregsel(), reg) };
    unsafe { core::ptr::read_volatile(iowin()) }
}

pub unsafe fn mask_all() {
    // Discover how many redirection entries the IOAPIC has
    // IOAPICVER: bits 23:16 hold (MaxRedirEntry)
    let ver = unsafe { mmio_read(0x01) };
    let max_redir = ((ver >> 16) & 0xFF) as u32; // usually 0x17 on Q35 (== 24 entries - 1)

    for i in 0..=max_redir {
        let redir_lo = 0x10 + i * 2;
        // Read, set mask bit (16), write back
        let mut lo = unsafe { mmio_read(redir_lo) };
        lo |= 1 << 16;
        unsafe { mmio_write(redir_lo, lo) };
        // (no need to touch high dword to just mask)
    }
}
