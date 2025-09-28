// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::identity_op)]

use core::ptr::{addr_of_mut, copy_nonoverlapping};
use core::sync::atomic::{AtomicBool, Ordering};

use super::arch_x86_64 as arch;
use super::memory::Memory;
use super::transport::Transport;

use crate::debug::{BKPT, Outcome, TrapFrame, breakpoint, clear_tf, set_tf};
use crate::kprintln;

// ─────────────────────────── Buffers (all in .bss) ───────────────────────────

const INBUF_LEN: usize = 0x2000;
const OUTBUF_LEN: usize = 0x2000;
const TMP_LEN: usize = 0x200;

#[unsafe(link_section = ".bss")]
static mut INBUF: [u8; INBUF_LEN] = [0; INBUF_LEN];
#[unsafe(link_section = ".bss")]
static mut OUTBUF: [u8; OUTBUF_LEN] = [0; OUTBUF_LEN];
#[unsafe(link_section = ".bss")]
static mut TMP: [u8; TMP_LEN] = [0; TMP_LEN];

/// RSP "no-ack" mode flag (QStartNoAckMode). Atomic so it’s irq-friendly.
static NO_ACK: AtomicBool = AtomicBool::new(false);
static EVER_RESUMED: AtomicBool = AtomicBool::new(false);

// ───────────────────────────── Small helpers ─────────────────────────────────

fn hex4(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}

fn from_hex(h: u8) -> Option<u8> {
    match h {
        b'0'..=b'9' => Some(h - b'0'),
        b'a'..=b'f' => Some(10 + h - b'a'),
        b'A'..=b'F' => Some(10 + h - b'A'),
        _ => None,
    }
}

/// Parse hex usize starting at `off`, up to `total` bytes in INBUF.
/// Returns (val, used_len).

fn parse_hex_usize(off: usize, total: usize) -> Option<(usize, usize)> {
    let mut n = 0usize;
    let mut i = 0usize;
    while off + i < total {
        let b = unsafe { INBUF[off + i] };
        if let Some(v) = from_hex(b) {
            n = (n << 4) | v as usize;
            i += 1;
        } else {
            break;
        }
    }
    if i == 0 { None } else { Some((n, i)) }
}

/// Parse "addr,len" pair; returns (addr, len, bytes_consumed)

fn parse_addr_len(off: usize, total: usize) -> Option<(usize, usize, usize)> {
    let (addr, ua) = parse_hex_usize(off, total)?;
    if off + ua >= total || unsafe { INBUF[off + ua] } != b',' {
        return None;
    }
    let (len, ul) = parse_hex_usize(off + ua + 1, total)?;
    Some((addr, len, ua + 1 + ul))
}

fn starts_with(off: usize, total: usize, pat: &[u8]) -> bool {
    if pat.len() > total.saturating_sub(off) {
        return false;
    }
    for i in 0..pat.len() {
        unsafe {
            if INBUF[off + i] != pat[i] {
                return false;
            }
        }
    }
    true
}

// ─────────────────────────── Packet I/O helpers ──────────────────────────────

fn send_pkt<T: Transport>(tx: &T, payload: &[u8]) {
    tx.putc(b'$');
    let mut cks: u8 = 0;
    for &b in payload {
        tx.putc(b);
        cks = cks.wrapping_add(b);
    }
    tx.putc(b'#');
    tx.putc(hex4((cks >> 4) & 0xF));
    tx.putc(hex4(cks & 0xF));

    // wait for '+', and resend once if we saw '-'
    if !NO_ACK.load(core::sync::atomic::Ordering::Relaxed) {
        loop {
            let b = tx.getc_block();
            match b {
                b'+' => break,
                b'-' => {
                    // resend exactly once; if you want, loop until '+'
                    tx.putc(b'$');
                    let mut cks2: u8 = 0;
                    for &b in payload {
                        tx.putc(b);
                        cks2 = cks2.wrapping_add(b);
                    }
                    tx.putc(b'#');
                    tx.putc(hex4((cks2 >> 4) & 0xF));
                    tx.putc(hex4(cks2 & 0xF));
                }
                _ => continue,
            }
        }
    }
}

unsafe fn send_pkt_raw<T: Transport>(tx: &T, ptr: *const u8, len: usize) {
    tx.putc(b'$');
    let mut cks: u8 = 0;
    for i in 0..len {
        let b = ptr.add(i).read();
        tx.putc(b);
        cks = cks.wrapping_add(b);
    }
    tx.putc(b'#');
    tx.putc(hex4((cks >> 4) & 0xF));
    tx.putc(hex4(cks & 0xF));

    if !NO_ACK.load(core::sync::atomic::Ordering::Relaxed) {
        loop {
            match tx.getc_block() {
                b'+' => break,
                b'-' => {
                    // resend once; you can loop until '+' if you prefer
                    tx.putc(b'$');
                    let mut c2: u8 = 0;
                    for i in 0..len {
                        let b = unsafe { ptr.add(i).read() };
                        tx.putc(b);
                        c2 = c2.wrapping_add(b);
                    }
                    tx.putc(b'#');
                    tx.putc(hex4((c2 >> 4) & 0xF));
                    tx.putc(hex4(c2 & 0xF));
                }
                _ => {}
            }
        }
    }
}

/// Receive a full packet into INBUF, return payload len (no '$' nor '#xx').
/// Handles ack/nack according to NO_ACK. CTRL-C (0x03) returns len=1 with INBUF[0]=0x03.
fn recv_pkt_len<T: Transport>(tx: &T) -> usize {
    loop {
        let mut c = tx.getc_block();

        // Ignore stray acks from peer
        if c == b'+' || c == b'-' {
            continue;
        }

        // Async ^C
        if c == 0x03 {
            unsafe {
                INBUF[0] = 0x03;
            }
            return 1;
        }

        // Expect a new packet
        if c != b'$' {
            continue;
        }

        let mut len = 0usize;
        let mut cks: u8 = 0;

        loop {
            c = tx.getc_block();
            if c == b'#' {
                break;
            }
            // keep one spare byte for safety; we never NUL-terminate, so <= ok too
            if len < INBUF_LEN {
                unsafe {
                    INBUF[len] = c;
                }
                len += 1;
                cks = cks.wrapping_add(c);
            }
        }

        let h1 = tx.getc_block();
        let h2 = tx.getc_block();

        let no_ack = NO_ACK.load(Ordering::Relaxed);
        if let (Some(a), Some(b)) = (from_hex(h1), from_hex(h2)) {
            let ok = ((a << 4) | b) == cks;
            if !no_ack {
                tx.putc(if ok { b'+' } else { b'-' });
            }
            if ok {
                return len;
            }
        } else {
            if !no_ack {
                tx.putc(b'-');
            }
        }
    }
}

// ───────────────────────────── Server core ───────────────────────────────────

pub struct RspServer;

impl RspServer {
    /// Run one RSP session on transport `tx`, using arch/memory policy `arch/mem`.
    /// Returns an `Outcome` directing the caller (resume/step/kill).
    #[inline(never)]
    pub fn run<T: Transport, M: Memory>(
        _tx: T,
        _arch: arch::X86_64Core,
        m: M,
        tf: *mut TrapFrame,
    ) -> Outcome {
        let tx = _tx; // by value, no &mut self

        if EVER_RESUMED.load(Ordering::Relaxed) {
            let (tid, pc) = (1u64, unsafe { (*tf).rip as u64 });
            send_t_stop(&tx, 0x05, tid, pc); // SIGTRAP; adjust if you classify causes
        }
        loop {
            let len = recv_pkt_len(&tx);
            if len == 0 {
                send_pkt(&tx, b"");
                continue;
            }

            let b0 = unsafe { INBUF[0] };

            match b0 {
                // "Why did you stop?"
                b'?' => send_pkt(&tx, b"S05"),

                // Set thread — single-thread model
                b'H' => send_pkt(&tx, b"OK"),

                // Queries
                b'q' => {
                    if starts_with(0, len, b"qSupported") {
                        // PacketSize is HEX per RSP (no 0x prefix). Keep features minimal.
                        send_pkt(&tx, b"PacketSize=2000;QStartNoAckMode+");
                    } else if starts_with(0, len, b"qAttached") {
                        send_pkt(&tx, b"1"); // attached to a live target
                    } else if starts_with(0, len, b"qfThreadInfo") {
                        send_pkt(&tx, b"m1"); // first chunk: one thread id (1)
                    } else if starts_with(0, len, b"qsThreadInfo") {
                        send_pkt(&tx, b"l"); // end of list
                    } else if starts_with(0, len, b"qC") {
                        send_pkt(&tx, b"QC1"); // current thread id
                    } else if starts_with(0, len, b"qTStatus") {
                        send_pkt(&tx, b""); // not tracing
                    } else if starts_with(0, len, b"vCont?") {
                        send_pkt(&tx, b"vCont;c;s");
                    } else {
                        send_pkt(&tx, b"");
                    }
                }

                // Set options
                b'Q' => {
                    if starts_with(0, len, b"QStartNoAckMode") {
                        NO_ACK.store(true, Ordering::Relaxed);
                        send_pkt(&tx, b"OK");
                    } else {
                        send_pkt(&tx, b"");
                    }
                }

                // Read all registers
                b'g' => unsafe {
                    let out = addr_of_mut!(OUTBUF) as *mut u8;
                    let _written = arch::write_g(out, tf as *const TrapFrame);
                    // Always send exactly what target.xml advertises
                    send_pkt_raw(&tx, out as *const u8, arch::G_HEX_LEN);
                },

                // Write all registers
                b'G' => {
                    let pay_len = len.saturating_sub(1);
                    if pay_len != arch::G_HEX_LEN {
                        send_pkt(&tx, b"E00");
                        continue;
                    }

                    let mut local: [u8; arch::G_HEX_LEN] = [0; arch::G_HEX_LEN];
                    unsafe {
                        // copy after 'G'
                        let src = (addr_of_mut!(INBUF) as *const u8).add(1);
                        copy_nonoverlapping(src, local.as_mut_ptr(), pay_len);
                    }

                    let ok = unsafe { arch::read_g(tf, &local[..pay_len]) };
                    send_pkt(&tx, if ok { b"OK" } else { b"E00" });
                }

                // Read memory: mADDR,LEN
                b'm' => {
                    if let Some((addr, rlen, _used)) = parse_addr_len(1, len) {
                        let max_len = OUTBUF_LEN / 2; // hex expansion
                        let mut allowed = rlen != 0 && rlen <= max_len && m.can_read(addr, rlen);

                        // Allow small window around current RSP to help backtraces even if not mapped in policy
                        if !allowed {
                            let tf_ref = unsafe { &*tf };
                            let rsp = tf_ref.rsp as usize;
                            let win_lo = rsp.saturating_sub(128 * 1024);
                            let win_hi = rsp.saturating_add(128 * 1024);
                            let end = addr.saturating_add(rlen);
                            if addr >= win_lo && end <= win_hi {
                                allowed = true;
                            }
                        }

                        if !allowed {
                            send_pkt(&tx, b"E01");
                            continue;
                        }

                        unsafe {
                            let src = addr as *const u8;
                            let out = addr_of_mut!(OUTBUF) as *mut u8;
                            let mut w = 0usize;
                            for i in 0..rlen {
                                let v = src.add(i).read(); // will fault if truly unmapped
                                out.add(w).write(hex4((v >> 4) & 0xF));
                                out.add(w + 1).write(hex4(v & 0xF));
                                w += 2;
                            }
                            send_pkt_raw(&tx, out as *const u8, w);
                        }
                    } else {
                        send_pkt(&tx, b"E00");
                    }
                }

                // Write memory: MADDR,LEN:HEX...
                b'M' => {
                    if let Some((addr, wlen, used)) = parse_addr_len(1, len) {
                        // Require colon
                        if 1 + used >= len || unsafe { INBUF[1 + used] } != b':' {
                            send_pkt(&tx, b"E00");
                            continue;
                        }
                        if wlen == 0 || wlen > TMP_LEN || !m.can_write(addr, wlen) {
                            send_pkt(&tx, b"E01");
                            continue;
                        }

                        let hex_off = 1 + used + 1;
                        let hex_len = len - hex_off;
                        if hex_len != wlen * 2 {
                            send_pkt(&tx, b"E00");
                            continue;
                        }

                        unsafe {
                            let tmp = addr_of_mut!(TMP) as *mut u8;
                            for i in 0..wlen {
                                let hi = from_hex(INBUF[hex_off + i * 2]);
                                let lo = from_hex(INBUF[hex_off + i * 2 + 1]);
                                match (hi, lo) {
                                    (Some(h), Some(l)) => tmp.add(i).write((h << 4) | l),
                                    _ => {
                                        send_pkt(&tx, b"E00");
                                        continue;
                                    }
                                }
                            }
                            copy_nonoverlapping(tmp as *const u8, addr as *mut u8, wlen);
                        }
                        send_pkt(&tx, b"OK");
                    } else {
                        send_pkt(&tx, b"E00");
                    }
                }

                // SW breakpoints: Z0/z0
                b'Z' if starts_with(0, len, b"Z0,") => {
                    if let Some((addr, _used)) = parse_hex_usize(3, len) {
                        let ok = breakpoint::insert(addr as u64);
                        send_pkt(&tx, if ok { b"OK" } else { b"E01" });
                    } else {
                        send_pkt(&tx, b"E00");
                    }
                }
                b'z' if starts_with(0, len, b"z0,") => {
                    if let Some((addr, _used)) = parse_hex_usize(3, len) {
                        let ok = breakpoint::remove(addr as u64);
                        send_pkt(&tx, if ok { b"OK" } else { b"E01" });
                    } else {
                        send_pkt(&tx, b"E00");
                    }
                }

                // vCont family
                b'v' if starts_with(0, len, b"vCont?") => {
                    send_pkt(&tx, b"vCont;c;s");
                }
                b'v' if starts_with(0, len, b"vCont;c") => {
                    unsafe {
                        clear_tf(&mut *tf);
                    }
                    if let Some((addr, orig)) = BKPT.lock().take() {
                        unsafe {
                            core::ptr::write_volatile(addr as *mut u8, orig);
                        }
                        unsafe {
                            if (*tf).rip == addr + 1 {
                                (*tf).rip = (*tf).rip.wrapping_sub(1);
                            }
                        }
                    }
                    EVER_RESUMED.store(true, Ordering::Relaxed);
                    return Outcome::Continue;
                }
                b'v' if starts_with(0, len, b"vCont;s") => {
                    unsafe {
                        set_tf(&mut *tf);
                    }
                    EVER_RESUMED.store(true, Ordering::Relaxed);
                    return Outcome::SingleStep;
                }

                // Legacy c/s
                b'c' => {
                    unsafe {
                        clear_tf(&mut *tf);
                    }
                    if let Some((addr, orig)) = BKPT.lock().take() {
                        unsafe {
                            core::ptr::write_volatile(addr as *mut u8, orig);
                        }
                        unsafe {
                            if (*tf).rip == addr + 1 {
                                (*tf).rip = (*tf).rip.wrapping_sub(1);
                            }
                        }
                    }
                    EVER_RESUMED.store(true, Ordering::Relaxed);
                    return Outcome::Continue;
                }
                b's' => {
                    unsafe {
                        set_tf(&mut *tf);
                    }
                    EVER_RESUMED.store(true, Ordering::Relaxed);
                    return Outcome::SingleStep;
                }

                // Kill
                b'k' => return Outcome::KillTask,

                // Async break while stopped
                0x03 => {
                    // Report SIGINT; remain in command loop (already stopped)
                    send_pkt(&tx, b"S02");
                }

                // Default: empty
                _ => send_pkt(&tx, b""),
            }
        }
    }
}

// ─────────────────────────── Stop-reply builder ──────────────────────────────

fn send_t_stop<T: Transport>(tx: &T, sig: u8, tid: u64, pc: u64) {
    // Stream the payload (no stack buffer, no memcpy) and compute checksum.
    // Payload: b"T" + hex(sig,2) + b";thread:" + hex(tid) + b";pc:" + hex(pc) + b";"
    let mut cks: u8 = 0;

    // open
    tx.putc(b'$');

    // "Txx"
    let write_byte = |tx: &T, cks: &mut u8, b: u8| {
        tx.putc(b);
        *cks = cks.wrapping_add(b);
    };
    write_byte(tx, &mut cks, b'T');
    write_byte(tx, &mut cks, hex4((sig >> 4) & 0xF));
    write_byte(tx, &mut cks, hex4(sig & 0xF));

    // ";thread:"
    write_byte(tx, &mut cks, b';');
    for &b in b"thread:" {
        write_byte(tx, &mut cks, b);
    }
    // hex(tid)
    write_hex_u64_stream(tx, &mut cks, tid);

    // ";pc:"
    write_byte(tx, &mut cks, b';');
    for &b in b"pc:" {
        write_byte(tx, &mut cks, b);
    }
    // hex(pc)
    write_hex_u64_stream(tx, &mut cks, pc);

    // ";"
    write_byte(tx, &mut cks, b';');

    // trailer
    tx.putc(b'#');
    tx.putc(hex4((cks >> 4) & 0xF));
    tx.putc(hex4(cks & 0xF));

    if !NO_ACK.load(core::sync::atomic::Ordering::Relaxed) {
        wait_for_ack(tx);
    }
}

fn wait_for_ack<T: Transport>(tx: &T) {
    if NO_ACK.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    loop {
        let b = tx.getc_block();
        match b {
            b'+' => break, // ack ok
            b'-' => break, // caller will resend
            _ => continue, // ignore noise (spurious bytes)
        }
    }
}

fn write_hex_u64_stream<T: Transport>(tx: &T, cks: &mut u8, mut v: u64) {
    // write without leading zeros; "0" if zero
    if v == 0 {
        tx.putc(b'0');
        *cks = cks.wrapping_add(b'0');
        return;
    }
    // collect nybbles reversed, then emit
    let mut tmp = [0u8; 16];
    let mut n = 0usize;
    while v != 0 {
        tmp[n] = hex4((v & 0xF) as u8);
        n += 1;
        v >>= 4;
    }
    for i in (0..n).rev() {
        tx.putc(tmp[i]);
        *cks = cks.wrapping_add(tmp[i]);
    }
}
