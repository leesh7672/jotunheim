// src/arch/x86_64/serial.rs
#![allow(dead_code)]

use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::SerialPort;

/// Global COM1 handle. It's inside a Mutex to serialize writers.
/// We store it as Option so the printing path can cheaply no-op if not inited.
static COM1: Mutex<Option<SerialPort>> = Mutex::new(None);
pub unsafe fn init_com1(_baud: u32) {
    let mut p = unsafe { SerialPort::new(0x3F8) };
    p.init();
    *COM1.lock() = Some(p);

    // If you keep your manual register setup, every .write() is unsafe:
    // let mut ier: Port<u8> = Port::new(0x3F9);
    // unsafe { ier.write(0u8); }
}

/// Lightweight writer that grabs the mutex and polls the UART.
/// IMPORTANT: We never print if COM1 isn't initialized.
struct Com1Writer;

impl Write for Com1Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Serialize access and avoid re-entrancy from ISRs:
        // If you're printing from an interrupt, the outer `without_interrupts`
        // still makes this path safe (no nested interrupts while the lock is held).
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
            // Not initialized yet — silently fail so early boot code isn't brittle.
            Err(fmt::Error)
        }
    }
}

/// Internal printing entry point used by the macros below.
/// We disable interrupts while holding the lock to prevent deadlocks if
/// printing happens inside an ISR or if an IRQ would try to print concurrently.
pub fn _print(args: fmt::Arguments) {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let _ = Com1Writer.write_fmt(args);
    });
}

/// `print!`/`println!` macros that target COM1.
/// These are tiny wrappers that keep your existing call sites unchanged.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::arch::x86_64::serial::_print(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::arch::x86_64::serial::_print(core::format_args!($($arg)*));
        $crate::print!("\n");
    }};
}
