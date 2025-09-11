use crate::arch::x86_64::serial;

pub trait Transport {
    fn getc_block(&self) -> u8;
    fn putc(&self, b: u8);
    fn write(&self, bytes: &[u8]) {
        for &b in bytes {
            self.putc(b);
        }
    }
}

/// COM2 backend; keep COM1 for human logs.
pub struct Com2Transport;

impl Transport for Com2Transport {
    #[inline]
    fn getc_block(&self) -> u8 {
        serial::com2_getc_block()
    }
    #[inline]
    fn putc(&self, b: u8) {
        serial::com2_write(&[b]);
    }
}

pub struct Com2Raw;

impl Transport for Com2Raw {
    fn putc(&self, b: u8) {
        unsafe {
            use x86_64::instructions::port::Port;
            let mut lsr: Port<u8> = Port::new(0x2F8 + 5);
            let mut thr: Port<u8> = Port::new(0x2F8 + 0);
            while lsr.read() & 0x20 == 0 {} // THRE
            thr.write(b);
        }
    }
    fn write(&self, buf: &[u8]) {
        for &b in buf {
            self.putc(b);
        }
    }
    fn getc_block(&self) -> u8 {
        unsafe {
            use x86_64::instructions::port::Port;
            let mut lsr: Port<u8> = Port::new(0x2F8 + 5);
            let mut rbr: Port<u8> = Port::new(0x2F8 + 0);
            loop {
                if lsr.read() & 0x01 != 0 {
                    return rbr.read();
                } // DR
                core::hint::spin_loop();
            }
        }
    }
}
