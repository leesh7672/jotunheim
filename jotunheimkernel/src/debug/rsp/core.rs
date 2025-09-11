#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::identity_op)]

use core::ptr::addr_of_mut;
use spin::Mutex;

use crate::debug::BKPT;
use crate::debug::breakpoint;
use crate::debug::{Outcome, TrapFrame, clear_tf, set_tf};

use super::arch_x86_64 as arch;
use super::memory::Memory;
use super::transport::Transport;

// ---- Buffers (no &/&mut to statics; use raw ptrs) ---------------------------

const INBUF_LEN: usize = 8192;
const OUTBUF_LEN: usize = 8192;
const TMP_LEN: usize = 512;

#[unsafe(link_section = ".bss")]
static mut INBUF: [u8; INBUF_LEN] = [0; INBUF_LEN];
#[unsafe(link_section = ".bss")]
static mut OUTBUF: [u8; OUTBUF_LEN] = [0; OUTBUF_LEN];
#[unsafe(link_section = ".bss")]
static mut TMP: [u8; TMP_LEN] = [0; TMP_LEN];

/// RSP "no-ack" mode flag (QStartNoAckMode). No atomics (toolchain friendly).
static NO_ACK: Mutex<bool> = Mutex::new(false);

// ---- Small helpers ----------------------------------------------------------

#[inline]
fn hex4(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}
#[inline]
fn from_hex(h: u8) -> Option<u8> {
    match h {
        b'0'..=b'9' => Some(h - b'0'),
        b'a'..=b'f' => Some(10 + h - b'a'),
        b'A'..=b'F' => Some(10 + h - b'A'),
        _ => None,
    }
}

// Parse hex usize starting at off, up to `total` bytes in INBUF. Returns (val, used_len).
#[inline]
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

// "addr,len" pair; returns (addr, len, bytes_consumed)
#[inline]
fn parse_addr_len(off: usize, total: usize) -> Option<(usize, usize, usize)> {
    let (addr, ua) = parse_hex_usize(off, total)?;
    if off + ua >= total || unsafe { INBUF[off + ua] } != b',' {
        return None;
    }
    let (len, ul) = parse_hex_usize(off + ua + 1, total)?;
    Some((addr, len, ua + 1 + ul))
}

#[inline]
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

// ---- Packet I/O -------------------------------------------------------------

#[inline]
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
}

#[inline]
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
}

/// Receive a full packet into INBUF, returns payload len (no '$', no '#xx').
/// Handles ack/nack according to NO_ACK. CTRL-C (0x03) returns len=1 with INBUF[0]=0x03.
fn recv_pkt_len<T: Transport>(tx: &T) -> usize {
    loop {
        let mut c = tx.getc_block();

        // Ignore stray + / - from the other side (harmless)
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
            if len + 1 < INBUF_LEN {
                unsafe {
                    INBUF[len] = c;
                }
                len += 1;
                cks = cks.wrapping_add(c);
            }
        }

        let h1 = tx.getc_block();
        let h2 = tx.getc_block();

        if let (Some(a), Some(b)) = (from_hex(h1), from_hex(h2)) {
            let ok = ((a << 4) | b) == cks;
            let no_ack = *NO_ACK.lock();
            if !no_ack {
                tx.putc(if ok { b'+' } else { b'-' });
            }
            if ok {
                return len;
            }
        } else {
            if !*NO_ACK.lock() {
                tx.putc(b'-');
            }
        }
    }
}

// ---- Server core ------------------------------------------------------------

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
        let tx = _tx; // move into closure scope (no &mut self)

        let (tid, pc) = (1u64, unsafe { (*tf).rip });
        send_t_stop(&tx, 0x05, tid, pc, /*swbreak=*/ true);

        loop {
            let len = recv_pkt_len(&tx);
            if len == 0 {
                send_pkt(&tx, b"");
                continue;
            }

            let b0 = unsafe { INBUF[0] };

            match b0 {
                // "Why did you stop?" — say SIGTRAP (05)
                b'?' => send_pkt(&tx, b"S05"),

                // Set thread (we only expose one)
                b'H' => send_pkt(&tx, b"OK"),

                // Queries
                b'q' => {
                    if starts_with(0, len, b"qSupported") {
                        // keep it minimal; DO NOT advertise qXfer if you don't serve it
                        send_pkt(&tx, b"PacketSize=1800;QStartNoAckMode+");
                    } else if starts_with(0, len, b"qAttached") {
                        send_pkt(&tx, b"1"); // we’re attached to a live target
                    } else if starts_with(0, len, b"qfThreadInfo") {
                        // first chunk of thread ids: "m<hex-id>[,<hex-id>...]"
                        // single-thread model => just one
                        send_pkt(&tx, b"m1");
                    } else if starts_with(0, len, b"qsThreadInfo") {
                        // end of list
                        send_pkt(&tx, b"l");
                    } else if starts_with(0, len, b"qC") {
                        // current thread id
                        send_pkt(&tx, b"QC1");
                    } else if starts_with(0, len, b"qTStatus") {
                        // not tracing; empty = "unsupported"
                        send_pkt(&tx, b"");
                    } else if starts_with(0, len, b"vCont?") {
                        // advertise continue/single-step
                        send_pkt(&tx, b"vCont;c;s");
                    } else {
                        send_pkt(&tx, b"");
                    }
                }

                // Set options
                b'Q' => {
                    if starts_with(0, len, b"QStartNoAckMode") {
                        *NO_ACK.lock() = true;
                        send_pkt(&tx, b"OK");
                    } else {
                        send_pkt(&tx, b"");
                    }
                }

                // Read all registers
                b'g' => unsafe {
                    let out = core::ptr::addr_of_mut!(OUTBUF) as *mut u8;
                    let _written = arch::write_g(out, tf as *const TrapFrame);
                    // Always send exactly the amount we advertised in target.xml
                    send_pkt_raw(&tx, out as *const u8, arch::G_HEX_LEN);
                },

                // Write all registers
                b'G' => {
                    // length of hex payload after the 'G'
                    let pay_len = len.saturating_sub(1);

                    // must exactly match what target.xml advertises
                    if pay_len != arch::G_HEX_LEN {
                        send_pkt(&tx, b"E00");
                        continue;
                    }

                    // stack buffer to avoid taking a reference to static mut INBUF
                    let mut local: [u8; arch::G_HEX_LEN] = [0; arch::G_HEX_LEN];

                    unsafe {
                        // raw pointer to INBUF (no & to static mut)
                        let src = (core::ptr::addr_of_mut!(INBUF) as *const u8).add(1);
                        core::ptr::copy_nonoverlapping(src, local.as_mut_ptr(), pay_len);
                    }

                    // pass a slice to arch::read_G
                    let ok = unsafe { arch::read_g(tf, &local[..pay_len]) };
                    if ok {
                        send_pkt(&tx, b"OK");
                    } else {
                        send_pkt(&tx, b"E00");
                    }
                }

                // Read memory: mADDR,LEN
                b'm' => {
                    if let Some((addr, rlen, _used)) = parse_addr_len(1, len) {
                        let max_len = OUTBUF_LEN / 2;
                        let mut allowed = rlen != 0 && rlen <= max_len && m.can_read(addr, rlen);

                        // Also allow a window around the *current* stack pointer so GDB can backtrace.
                        // (saturating math avoids underflow on very small addresses)
                        if !allowed {
                            let tf_ref = unsafe { &*tf };
                            let rsp = tf_ref.rsp as usize;
                            let win_lo = rsp.saturating_sub(128 * 1024); // 128 KiB below RSP
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
                            let out = core::ptr::addr_of_mut!(OUTBUF) as *mut u8;
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
                // Z0 = software breakpoint; z0 = remove
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

                // Write memory: MADDR,LEN:HEX...
                b'M' => {
                    if let Some((addr, wlen, used)) = parse_addr_len(1, len) {
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
                            core::ptr::copy_nonoverlapping(tmp as *const u8, addr as *mut u8, wlen);
                        }
                        send_pkt(&tx, b"OK");
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
                    return Outcome::Continue;
                }
                b'v' if starts_with(0, len, b"vCont;s") => {
                    unsafe {
                        set_tf(&mut *tf);
                    }
                    return Outcome::SingleStep;
                }

                // legacy c/s
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
                    return Outcome::Continue;
                }
                b's' => {
                    unsafe {
                        set_tf(&mut *tf);
                    }
                    return Outcome::SingleStep;
                }

                // kill
                b'k' => return Outcome::KillTask,

                // async break (Ctrl-C)
                0x03 => {
                    send_pkt(&tx, b"S02");
                }

                // default: empty response
                _ => send_pkt(&tx, b""),
            }
        }
    }
}

fn send_t_stop<T: Transport>(tx: &T, sig: u8, tid: u64, pc: u64, swbreak: bool) {
    // small stack buffer; no refs to statics
    let mut buf = [0u8; 96];
    let mut w = 0usize;

    // "Txx"
    buf[w] = b'T';
    w += 1;
    buf[w] = hex4((sig >> 4) & 0xF);
    w += 1;
    buf[w] = hex4(sig & 0xF);
    w += 1;

    // "thread:<hex>;"
    buf[w..w + 7].copy_from_slice(b"thread:");
    w += 7;
    // write hex(tid)
    let mut tmp = [0u8; 16];
    let mut i = 0;
    let mut t = tid;
    if t == 0 {
        tmp[i] = b'0';
        i += 1;
    }
    while t != 0 {
        tmp[i] = hex4((t & 0xF) as u8);
        i += 1;
        t >>= 4;
    }
    // reverse
    for k in (0..i).rev() {
        buf[w] = tmp[k];
        w += 1;
    }
    buf[w] = b';';
    w += 1;

    // "pc:<hex>;"
    buf[w..w + 3].copy_from_slice(b"pc:");
    w += 3;
    let mut tmp2 = [0u8; 16];
    let mut j = 0;
    let mut p = pc;
    if p == 0 {
        tmp2[j] = b'0';
        j += 1;
    }
    while p != 0 {
        tmp2[j] = hex4((p & 0xF) as u8);
        j += 1;
        p >>= 4;
    }
    for k in (0..j).rev() {
        buf[w] = tmp2[k];
        w += 1;
    }
    buf[w] = b';';
    w += 1;

    send_pkt(tx, &buf[..w]);
}
