// src/arch/x86_64/serial.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(dead_code)]

use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::SerialPort;

/// Global COM1 handle. It's inside a Mutex to serialize writers.
/// We store it as Option so the printing path can cheaply no-op if not inited.
static COM1: Mutex<Option<SerialPort>> = Mutex::new(None);
/// Dedicated COM2 for the debugger (RSP or secondary console).
static COM2: Mutex<Option<SerialPort>> = Mutex::new(None);

// init_com1 / init_com2: wrap SerialPort::new in an explicit unsafe block
pub unsafe fn init_com1(_baud: u32) {
    let mut p = unsafe { SerialPort::new(0x3F8) };
    p.init();
    *COM1.lock() = Some(p);
}

pub unsafe fn init_com2(_baud: u32) {
    let mut p = unsafe { SerialPort::new(0x2F8) };
    p.init();
    *COM2.lock() = Some(p);
}

/// Are the ports ready?
pub fn com1_ready() -> bool {
    COM1.lock().is_some()
}
pub fn com2_ready() -> bool {
    COM2.lock().is_some()
}

/// Lightweight writer that grabs the mutex and polls the UART.
/// IMPORTANT: We never print if COM1 isn't initialized.
struct Com1Writer;

impl Write for Com1Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(port) = &mut *COM1.lock() {
            for b in s.bytes() {
                // Convert '\n' to CRLF for nicer consoles
                if b == b'\n' {
                    port.send(b'\r');
                }
                port.send(b);
            }
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

/// COM2 writer (for debugger messages, optional banner)
struct Com2Writer;

impl Write for Com2Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(port) = &mut *COM2.lock() {
            for b in s.bytes() {
                if b == b'\n' {
                    port.send(b'\r');
                }
                port.send(b);
            }
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal printing entry points used by the macros below.
// We disable interrupts while holding the lock to prevent deadlocks if
// printing happens inside an ISR or if an IRQ would try to print concurrently.

#[doc(hidden)]
pub fn _kprint(args: fmt::Arguments) {
    // If COM1 isn't ready, silently drop—early boot should not crash on logs.
    if !com1_ready() {
        return;
    }
    let _ = Com1Writer.write_fmt(args);
}

#[doc(hidden)]
pub fn _kprint2(args: fmt::Arguments) {
    if !com2_ready() {
        return;
    }
    let _ = Com2Writer.write_fmt(args);
}

// ─────────────────────────────────────────────────────────────────────────────
// Public helpers for direct byte I/O (useful for debugger stubs)
// COM1 byte I/O — use try_* APIs
pub fn com1_putc(b: u8) {
    if let Some(p) = COM1.lock().as_mut() {
        let _ = p.try_send_raw(b);
    }
}
pub fn com1_write(bytes: &[u8]) {
    if let Some(p) = COM1.lock().as_mut() {
        for &b in bytes {
            // use raw to avoid double CRLF translation (SerialPort::send does its own)
            let _ = p.try_send_raw(b);
        }
    }
}
pub fn com1_getc_block() -> u8 {
    loop {
        if let Some(p) = COM1.lock().as_mut() {
            if let Ok(b) = p.try_receive() {
                return b;
            }
        }
        core::hint::spin_loop();
    }
}
pub fn com1_getc_nb() -> Option<u8> {
    if let Some(p) = COM1.lock().as_mut() {
        if let Ok(b) = p.try_receive() {
            return Some(b);
        }
    }
    None
}

// COM2 byte I/O — same idea
pub fn com2_putc(b: u8) {
    if let Some(p) = COM2.lock().as_mut() {
        let _ = p.try_send_raw(b);
    }
}
pub fn com2_write(bytes: &[u8]) {
    if let Some(p) = COM2.lock().as_mut() {
        for &b in bytes {
            let _ = p.try_send_raw(b);
        }
    }
}
pub fn com2_getc_block() -> u8 {
    loop {
        if let Some(p) = COM2.lock().as_mut() {
            if let Ok(b) = p.try_receive() {
                return b;
            }
        }
        core::hint::spin_loop();
    }
}
pub fn com2_getc_nb() -> Option<u8> {
    if let Some(p) = COM2.lock().as_mut() {
        if let Ok(b) = p.try_receive() {
            return Some(b);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Macros: kernel print to COM1 (logs) and to COM2 (debug link)

/// Print to COM1 with newline.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        $crate::arch::x86_64::serial::_kprint(core::format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => {{
        $crate::arch::x86_64::serial::_kprint(core::format_args!("\n"));
    }};
    ($($arg:tt)*) => {{
        // First print the formatted message, then a newline (both stable).
        $crate::arch::x86_64::serial::_kprint(core::format_args!($($arg)*));
        $crate::arch::x86_64::serial::_kprint(core::format_args!("\n"));
    }};
}

/// Print to COM2 (debugger wire) without newline.
#[macro_export]
macro_rules! dprint {
    ($($arg:tt)*) => ({
        $crate::arch::x86_64::serial::_kprint2(core::format_args!($($arg)*));
    })
}

#[macro_export]
macro_rules! dprintln {
    () => {{
        $crate::arch::x86_64::serial::_kprint2(core::format_args!("\n"));
    }};
    ($($arg:tt)*) => {{
        // First print the formatted message, then a newline (both stable).
        $crate::arch::x86_64::serial::_kprint2(core::format_args!($($arg)*));
        $crate::arch::x86_64::serial::_kprint2(core::format_args!("\n"));
    }};
}

// ─────────────────────────────────────────────────────────────────────────────
// Small convenience banner helpers (optional)

pub fn banner_com1(s: &str) {
    com1_write(b"\n---[ ");
    com1_write(s.as_bytes());
    com1_write(b" ]---\n");
}
pub fn banner_com2(s: &str) {
    com2_write(b"\n---[ ");
    com2_write(s.as_bytes());
    com2_write(b" ]---\n");
}
