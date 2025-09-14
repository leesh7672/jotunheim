use crate::debug::TrapFrame;

pub struct X86_64Core;

pub const G_HEX_LEN: usize = 572;


const fn hex4(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}


unsafe fn put8(out: *mut u8, w: &mut usize, v: u8) {
    out.add(*w).write(hex4((v >> 4) & 0xF));
    out.add(*w + 1).write(hex4(v & 0xF));
    *w += 2;
}


unsafe fn put16(out: *mut u8, w: &mut usize, v: u16) {
    for b in v.to_le_bytes() {
        put8(out, w, b);
    }
}


unsafe fn put32(out: *mut u8, w: &mut usize, v: u32) {
    for b in v.to_le_bytes() {
        put8(out, w, b);
    }
}


unsafe fn put64(out: *mut u8, w: &mut usize, v: u64) {
    for b in v.to_le_bytes() {
        put8(out, w, b);
    }
}

/// Returns number of hex bytes written (must be == G_HEX_LEN)
pub unsafe fn write_g(out: *mut u8, tf: *const TrapFrame) -> usize {
    let t = &*tf;
    let mut w = 0usize;

    macro_rules! r64 {
        ($e:expr) => {
            put64(out, &mut w, $e)
        };
    }
    macro_rules! r32 {
        ($e:expr) => {
            put32(out, &mut w, $e as u32)
        };
    }
    macro_rules! r16 {
        ($e:expr) => {
            put16(out, &mut w, $e as u16)
        };
    }

    // 16 GPRs + RIP
    r64!(t.rax);
    r64!(t.rbx);
    r64!(t.rcx);
    r64!(t.rdx);
    r64!(t.rsi);
    r64!(t.rdi);
    r64!(t.rbp);
    r64!(t.rsp);
    r64!(t.r8);
    r64!(t.r9);
    r64!(t.r10);
    r64!(t.r11);
    r64!(t.r12);
    r64!(t.r13);
    r64!(t.r14);
    r64!(t.r15);
    r64!(t.rip);

    // eflags (32), seg selectors (32 each)
    r32!(t.rflags as u32);
    r32!(t.cs);
    r32!(t.ss);
    r32!(0);
    r32!(0);
    r32!(0);
    r32!(0); // ds, es, fs, gs — zeroed

    // x87 st0..st7 (80-bit each) — write 10 zero bytes apiece
    for _ in 0..8 {
        for _ in 0..10 {
            put8(out, &mut w, 0);
        }
    }

    // x87 control/status words — all zeros
    r16!(0); // fctrl
    r16!(0); // fstat
    r16!(0); // ftag
    r16!(0); // fiseg
    r32!(0); // fioff
    r16!(0); // foseg
    r32!(0); // fooff
    r16!(0); // fop

    // fs_base / gs_base — zero if you don't track them yet
    r64!(0);
    r64!(0);

    w
}

// arch_x86_64.rs (or wherever read_G lives)
pub unsafe fn read_g(tf: *mut TrapFrame, payload: &[u8]) -> bool {
    // 26 regs @ 188 bytes => 376 hex chars
    if payload.len() != G_HEX_LEN {
        return false;
    }

    let mut i = 0usize;

    
    fn from_hex(h: u8) -> Option<u8> {
        match h {
            b'0'..=b'9' => Some(h - b'0'),
            b'a'..=b'f' => Some(10 + h - b'a'),
            b'A'..=b'F' => Some(10 + h - b'A'),
            _ => None,
        }
    }
    
    fn r64(p: &[u8], idx: &mut usize) -> Option<u64> {
        let mut le = [0u8; 8];
        for k in 0..8 {
            let hi = from_hex(*p.get(*idx + 2 * k)?)?;
            let lo = from_hex(*p.get(*idx + 2 * k + 1)?)?;
            le[k] = (hi << 4) | lo;
        }
        *idx += 16;
        Some(u64::from_le_bytes(le))
    }
    
    fn r32(p: &[u8], idx: &mut usize) -> Option<u32> {
        let mut le = [0u8; 4];
        for k in 0..4 {
            let hi = from_hex(*p.get(*idx + 2 * k)?)?;
            let lo = from_hex(*p.get(*idx + 2 * k + 1)?)?;
            le[k] = (hi << 4) | lo;
        }
        *idx += 8;
        Some(u32::from_le_bytes(le))
    }

    macro_rules! R64 {
        () => {
            match r64(payload, &mut i) {
                Some(v) => v,
                None => return false,
            }
        };
    }
    macro_rules! R32 {
        () => {
            match r32(payload, &mut i) {
                Some(v) => v,
                None => return false,
            }
        };
    }

    let t = &mut *tf;

    // 16 GPRs
    t.rax = R64!();
    t.rbx = R64!();
    t.rcx = R64!();
    t.rdx = R64!();
    t.rsi = R64!();
    t.rdi = R64!();
    t.rbp = R64!();
    t.rsp = R64!();
    t.r8 = R64!();
    t.r9 = R64!();
    t.r10 = R64!();
    t.r11 = R64!();
    t.r12 = R64!();
    t.r13 = R64!();
    t.r14 = R64!();
    t.r15 = R64!();

    // RIP, orig_rax (orig_rax is ignored for now)
    t.rip = R64!();
    let _orig_rax = R64!();

    // eflags (lower 32 bits)
    let ef = R32!();
    t.rflags = (t.rflags & !0xFFFF_FFFF) | (ef as u64);

    // cs/ss/ds/es/fs/gs — consume and ignore writes
    let _cs = R32!();
    let _ss = R32!();
    let _ds = R32!();
    let _es = R32!();
    let _fs = R32!();
    let _gs = R32!();

    // fs_base / gs_base — consume (ignored)
    let _fsb = R64!();
    let _gsb = R64!();

    true
}
