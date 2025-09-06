use core::fmt::Write;
use spin::Mutex;
use uart_16550::SerialPort;

static COM1: Mutex<Option<SerialPort>> = Mutex::new(None);

pub unsafe fn init_com1(_baud: u32) {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    *COM1.lock() = Some(port);
}

fn _write_str(s: &str) {
    if let Some(ref mut port) = *COM1.lock() {
        for &b in s.as_bytes() {
            let _ = port.send(b);
        }
    }
}

pub struct Serial;
impl Write for Serial {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        _write_str(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!(&mut $crate::arch::x86_64::serial::Serial, $($arg)*);
    }};
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($fmt:literal $(, $($arg:tt)+)?) => {{
        $crate::print!(concat!($fmt, "\n") $(, $($arg)+)?);
    }};
}
