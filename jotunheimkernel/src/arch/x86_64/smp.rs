// src/arch/x86_64/smp.rs
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::{
    ptr,
    sync::atomic::{Ordering, compiler_fence},
};
use spin::Once;

use crate::{
    acpi::{CpuEntry, madt},
    bootinfo::BootInfo,
    kprintln,
};

use crate::arch::x86_64::ap_trampoline;

#[repr(C, align(16))]
pub struct ApBoot {
    pub ready_flag: u32, // set to 1 by the trampoline just before jumping to ap_entry()
    pub _pad: u32,
    pub cr3: u64,
    pub gdt_ptr: u64,
    pub idt_ptr: u64,
    pub stack_top: u64,
    pub entry64: u64,
    pub hhdm: u64,
}

pub struct Topology {
    pub bsp_lapic_id: u32,
    pub total_cpus: usize,
    pub enabled_cpus: usize,
}

static TOPO: Once<Topology> = Once::new();

pub fn enumerate(boot: &BootInfo) {
    if let Some(m) = madt::discover(boot) {
        let bsp_id = super::apic::lapic_id();
        let total = m.cpus.len();
        let enabled = m.cpus.iter().filter(|c| c.enabled).count();

        kprintln!("[SMP] MADT LAPIC mmio = {:#x}", m.lapic_phys);
        for (i, c) in m.cpus.iter().enumerate() {
            kprintln!(
                "[SMP] CPU#{:02} apic_id={} enabled={} x2apic={}",
                i,
                c.apic_id,
                c.enabled,
                c.is_x2apic
            );
        }

        let _ = TOPO.call_once(|| Topology {
            bsp_lapic_id: bsp_id,
            total_cpus: total,
            enabled_cpus: enabled,
        });
        kprintln!(
            "[SMP] BSP LAPIC ID={}, total={}, enabled={}",
            bsp_id,
            total,
            enabled
        );
    } else {
        kprintln!("[SMP] MADT not found.");
    }
}

pub fn topology() -> Option<&'static Topology> {
    TOPO.get()
}

/// Bring all enabled APs online (one-by-one to avoid sharing the same trampoline page)
/// Requires:
///   - paging/GDT/IDT are ready on BSP
///   - the trampoline has been assembled and findable via `ap_trampoline::blob()`
///   - low identity map for `TRAMP_PHYS` page exists
pub fn boot_all_aps(boot: &BootInfo) {
    let Some(m) = madt::discover(boot) else {
        kprintln!("[SMP] No MADT; cannot boot APs.");
        return;
    };

    // 1) Copy the trampoline to a fixed low 4KiB page
    const TRAMP_PHYS: u64 = 0x8000; // 32 KiB, 4 KiB aligned, <1MiB
    let (blob, p32_off, p64_off) = ap_trampoline::blob();
    if blob.len() > 4096 {
        kprintln!("[SMP] Trampoline too large: {} bytes", blob.len());
        return;
    }
    unsafe {
        let dst = (boot.hhdm_base + TRAMP_PHYS) as *mut u8;
        ptr::copy_nonoverlapping(blob.as_ptr(), dst, blob.len());
    }
    let tramp_virt = boot.hhdm_base + TRAMP_PHYS;
    let vector: u8 = ((TRAMP_PHYS >> 12) & 0xFF) as u8;

    // 2) Read BSP's CR3 so APs can use the same page tables
    let (cr3_frame, _) = x86_64::registers::control::Cr3::read();
    let cr3 = cr3_frame.start_address().as_u64();

    // 3) Entry for APs
    let entry_fn = ap_entry as extern "C" fn() -> !;
    let entry64 = entry_fn as usize as u64;

    // 4) Boot APs one-by-one; after SIPIs we wait until ready_flag==1
    let bsp_id = super::apic::lapic_id();
    for c in m.cpus.iter().filter(|c| c.enabled) {
        if c.apic_id == bsp_id {
            continue;
        }

        // Per-AP stack: allocate 8KiB and leak
        const AP_STACK_SIZE: usize = 8 * 1024;
        let stk_box: Box<[u8]> = vec_with_len(AP_STACK_SIZE).into_boxed_slice();
        let stk_top = (stk_box.as_ptr() as usize + AP_STACK_SIZE) as u64;
        let _leak_stack: &'static mut [u8] = Box::leak(stk_box); // keep forever

        // Per-AP ApBoot struct (leaked)
        let ab = ApBoot {
            ready_flag: 0,
            _pad: 0,
            cr3,
            gdt_ptr: 0, // you can point to your kernel GDT later; the tramp loads a tiny flat GDT already
            idt_ptr: 0, // reload in ap_entry() if you want
            stack_top: stk_top,
            entry64,
            hhdm: boot.hhdm_base,
        };
        let ab_ref: &'static mut ApBoot = Box::leak(Box::new(ab));

        // Trampoline expects PHYSICAL address of ApBoot
        let apboot_phys = (ab_ref as *mut ApBoot as usize as u64).wrapping_sub(boot.hhdm_base);

        // Patch the trampoline page with this AP's ApBoot physical pointer
        unsafe {
            let p32_ptr = (tramp_virt + p32_off as u64) as *mut u32;
            let p64_ptr = (tramp_virt + p64_off as u64) as *mut u64;
            p32_ptr.write(apboot_phys as u32);
            p64_ptr.write(apboot_phys);
            compiler_fence(Ordering::SeqCst);
        }

        // INIT → SIPI → SIPI
        unsafe { super::apic::send_init(c.apic_id) }
        spin_delay_us(10_000);
        unsafe { super::apic::send_startup(c.apic_id, vector) }
        spin_delay_us(200);
        unsafe { super::apic::send_startup(c.apic_id, vector) }

        // Wait for this AP to set ready_flag (the trampoline writes it to 1)
        if !wait_ready(&ab_ref.ready_flag as *const u32, 100_000) {
            // 100ms-ish spin
            kprintln!("[SMP] apic_id {} did not signal ready in time", c.apic_id);
        }
    }
    kprintln!("[SMP] All APs attempted.");
}

/// Very dumb spin delay until you wire your calibrated TSC helper.
#[inline(always)]
fn spin_delay_us(us: u64) {
    let iters = us.saturating_mul(200);
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}

/// Spin on a volatile u32 until it becomes non-zero, with a simple timeout loop.
fn wait_ready(flag_ptr: *const u32, max_spins: u64) -> bool {
    for _ in 0..max_spins {
        let v = unsafe { ptr::read_volatile(flag_ptr) };
        if v != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Helper to allocate a zeroed Vec of given length without referencing static mut.
fn vec_with_len(len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    // SAFETY: we immediately write zeros to the uninit region; then set_len.
    unsafe {
        ptr::write_bytes(v.as_mut_ptr(), 0, len);
        v.set_len(len);
    }
    v
}

/// What each AP runs after the trampoline puts us in 64-bit mode.
#[unsafe(no_mangle)]
pub extern "C" fn ap_entry() -> ! {
    // You can load your kernel GDT/IDT here if needed, enable interrupts, etc.
    let id = super::apic::lapic_id();
    kprintln!("[SMP] hello from AP lapic_id={}", id);
    loop {
        x86_64::instructions::hlt();
    }
}

/// Optional: legacy “kick” if you still call it elsewhere; now just wraps boot_all_aps().
pub fn start_aps(boot: &BootInfo, tramp_phys: u64, _unused: &ApBoot, cpus: &[CpuEntry]) {
    // Ignore the parameters; use the safer boot_all_aps() path that handles stacks & ApBoot
    let _ = (tramp_phys, _unused, cpus);
    boot_all_aps(boot);
}
