// faultsvc.rs — ISR-safe fault logging, early-boot friendly
#![allow(dead_code)]
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::sync::atomic::Ordering::{Acquire, Release, Relaxed};

/// Maximum number of CPUs supported by the fault logger.
pub const MAX_CPUS: usize = 64;
/// Number of entries per per-CPU ring buffer.
pub const RING_LEN: usize = 128;

/// A fault record captured from an ISR. All fields are plain integers so
/// writing from an exception handler never allocates or takes locks.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct FaultRecord {
    pub cpu: u32,
    pub vector: u8,
    pub has_err: u8,
    pub _pad0: u16,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
    pub cr2: u64,
    pub tsc: u64,
    pub seq: u64,
}

/// A lightweight view of the trap frame used when logging from an ISR.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct TrapFrameView {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// One slot in a per-CPU ring buffer. The `seq` field is used to track
/// whether the slot is empty, being written, or committed.
#[repr(C)]
struct Slot {
    seq: AtomicU64,
    rec: MaybeUninit<FaultRecord>,
}

struct Ring {
    head: AtomicUsize,
    slots: [Slot; RING_LEN],
}

struct State {
    max_cpus: AtomicUsize,
    seq: [AtomicU64; MAX_CPUS],
    rings: [Ring; MAX_CPUS],
    cpu_index_fn: fn() -> usize,
}

const EMPTY_SLOT: Slot = Slot { seq: AtomicU64::new(0), rec: MaybeUninit::uninit() };
const EMPTY_RING: Ring = Ring { head: AtomicUsize::new(0), slots: [EMPTY_SLOT; RING_LEN] };
const AU64: AtomicU64 = AtomicU64::new(0);

#[unsafe(link_section = ".bss")]
static mut STATE: State = State {
    max_cpus: AtomicUsize::new(1),
    seq: [AU64; MAX_CPUS],
    rings: [EMPTY_RING; MAX_CPUS],
    cpu_index_fn: || 0,
};

/// Initialize SMP support for the fault logger. Keeps any early-boot logs intact.
pub fn init_smp(max_cpus: usize, cpu_index_fn: fn() -> usize) {
    unsafe {
        STATE.max_cpus.store(max_cpus.min(MAX_CPUS), Relaxed);
        STATE.cpu_index_fn = cpu_index_fn;
    }
}

/// Log a fault from an ISR. Never allocates or locks.
pub fn log_from_isr(
    vector: u8,
    error_code: u64,
    has_err: bool,
    tf: &TrapFrameView,
    cr2: u64,
    tsc: u64,
) {
    unsafe {
        let max = STATE.max_cpus.load(Relaxed);
        let cpu_ix = (STATE.cpu_index_fn)().min(max.saturating_sub(1));
        push(cpu_ix, FaultRecord {
            cpu: cpu_ix as u32,
            vector,
            has_err: has_err as u8,
            _pad0: 0,
            error_code,
            rip: tf.rip,
            cs: tf.cs,
            rflags: tf.rflags,
            rsp: tf.rsp,
            ss: tf.ss,
            cr2,
            tsc,
            seq: 0,
        });
    }
}

/// Iterate over all committed fault logs and invoke the callback.
pub fn drain_and_print(mut print: impl FnMut(&FaultRecord)) {
    unsafe { drain_impl(&mut print) }
}

/// Snapshot the recent fault logs for a CPU without consuming them.
pub fn snapshot_cpu(cpu_ix: usize, out: &mut [FaultRecord]) -> usize {
    unsafe { snapshot_impl(cpu_ix, out) }
}

unsafe fn push(cpu_ix: usize, mut rec: FaultRecord) {
    let ring = &STATE.rings[cpu_ix];
    let head = ring.head.fetch_add(1, Relaxed);
    let idx = head % RING_LEN;
    let seq = STATE.seq[cpu_ix].fetch_add(1, Relaxed) + 1;
    rec.seq = seq;
    let slot = &ring.slots[idx];
    slot.seq.store((seq << 1) | 1, Relaxed);
    slot.rec.as_mut_ptr().write(rec);
    slot.seq.store(seq << 1, Release);
}

unsafe fn drain_impl(print: &mut impl FnMut(&FaultRecord)) {
    let max = STATE.max_cpus.load(Relaxed);
    for cpu_ix in 0..max {
        let produced = STATE.seq[cpu_ix].load(Relaxed);
        let from = produced.saturating_sub(RING_LEN as u64) + 1;
        let ring = &STATE.rings[cpu_ix];
        for seq in from..=produced {
            let idx = ((seq - 1) as usize) % RING_LEN;
            let slot = &ring.slots[idx];
            let s = slot.seq.load(Acquire);
            if s != (seq << 1) { continue; }
            let rec = slot.rec.assume_init_ref();
            if rec.seq != seq { continue; }
            print(rec);
        }
    }
}

unsafe fn snapshot_impl(cpu_ix: usize, out: &mut [FaultRecord]) -> usize {
    let max = STATE.max_cpus.load(Relaxed);
    if cpu_ix >= max || out.is_empty() { return 0; }
    let produced = STATE.seq[cpu_ix].load(Relaxed);
    let want = core::cmp::min(out.len() as u64, RING_LEN as u64);
    let start = produced.saturating_sub(want) + 1;
    let ring = &STATE.rings[cpu_ix];
    let mut n = 0usize;
    for seq in start..=produced {
        let idx = ((seq - 1) as usize) % RING_LEN;
        let slot = &ring.slots[idx];
        let s = slot.seq.load(Acquire);
        if s != (seq << 1) { continue; }
        let rec = slot.rec.assume_init_ref();
        if rec.seq != seq { continue; }
        out[n] = *rec;
        n += 1;
    }
    n
}