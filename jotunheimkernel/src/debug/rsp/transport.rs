// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
pub trait Transport {
    fn getc_block(&self) -> u8;
    fn putc(&self, b: u8);
}

/// COM2 backend; keep COM1 for human logs.
pub struct Com2Transport;

impl Transport for Com2Transport {
    fn putc(&self, b: u8) {
        unsafe {
            use x86_64::instructions::port::Port;
            let mut lsr: Port<u8> = Port::new(0x2F8 + 5);
            let mut thr: Port<u8> = Port::new(0x2F8 + 0);
            while lsr.read() & 0x20 == 0 {} // THRE
            thr.write(b);
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
