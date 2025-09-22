// src/arch/x86_64/smp.rs
// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::{
    ptr,
    sync::atomic::{Ordering, compiler_fence},
};

use x86_64::instructions::{hlt, interrupts::without_interrupts};

use crate::{
    acpi::{cpuid::CpuId, madt},
    arch::x86_64::{
        apic::{self, lapic_id},
        tables::{
            self, access_mut
        },
    },
    bootinfo::BootInfo,
    kprintln, mem,
};

use crate::arch::x86_64::ap_trampoline;

static mut HHDM_BASE: u64 = 0;

#[derive(Debug, Clone, Copy)]
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

/// Bring all enabled APs online (one-by-one to avoid sharing the same trampoline page)
/// Requires:
///   - paging/GDT/IDT are ready on BSP
///   - the trampoline has been assembled and findable via `ap_trampoline::blob()`
///   - low identity map for `TRAMP_PHYS` page exists
pub fn boot_all_aps(boot: &BootInfo) {
    unsafe { HHDM_BASE = boot.hhdm_base };
    let Some(m) = madt::discover(boot) else {
        kprintln!("[SMP] No MADT; cannot boot APs.");
        return;
    };

    // Initialize fault logging for SMP: determine CPU count and install a CPU
    // index function that returns the current APIC ID. This preserves any
    // records captured before SMP init.
    {
        let cpu_count = m.cpus.iter().filter(|c| c.enabled).count();
        fn cpu_index_from_lapic() -> usize {
            // Use the LAPIC ID as the CPU index. This works because APIC IDs
            // are unique. If APIC IDs are sparse, adapt this function to map
            // IDs to contiguous indices.
            lapic_id() as usize
        }
        unsafe { crate::faultsvc::init_smp(core::cmp::max(cpu_count, 1), cpu_index_from_lapic) };
    }

    // --- 1) Trampoline: copy once to low physical page (e.g., 0x8000) ---
    const TRAMP_PHYS: u64 = 0x1000; // 32KiB, <1MiB, 4KiB aligned
    let (blob, p32_off, p64_off) = ap_trampoline::blob();
    if blob.len() > 4096 {
        kprintln!("[SMP] Trampoline too large: {} bytes", blob.len());
        return;
    }
    mem::map_identity_4k(0x8000);
    mem::map_identity_4k(0x9000);
    unsafe {
        let dst = (boot.hhdm_base + TRAMP_PHYS) as *mut u8;
        core::ptr::copy_nonoverlapping(blob.as_ptr(), dst, blob.len());
    }
    let tramp_virt = boot.hhdm_base + TRAMP_PHYS;
    let vector: u8 = ((TRAMP_PHYS >> 12) & 0xFF) as u8;

    // --- 2) Warm-reset vector (some firmware requires it) ---
    fn program_warm_reset(tramp_phys: u64, hhdm: u64) {
        use x86_64::instructions::port::Port;
        unsafe {
            // CMOS shutdown code 0x0A
            Port::<u8>::new(0x70).write(0x0F);
            Port::<u8>::new(0x71).write(0x0A);
            // BDA warm reset vector at phys 0x467 (segment:offset)
            let wrv_seg = (hhdm + 0x467) as *mut u16;
            let wrv_off = (hhdm + 0x469) as *mut u16;
            wrv_seg.write((tramp_phys >> 4) as u16);
            wrv_off.write(0);
        }
    }
    program_warm_reset(TRAMP_PHYS, boot.hhdm_base);

    // --- 3) Share BSP's CR3 so APs see the same page tables ---
    let (cr3_frame, _) = x86_64::registers::control::Cr3::read();
    let cr3 = cr3_frame.start_address().as_u64();

    // --- 4) Entry for APs (kernel 64-bit entry) ---
    let entry64 = ap_entry as usize as u64;

    // --- 5) Bring up each enabled AP ---
    let bsp_id = apic::lapic_id();

    let (ab_va, ab_pa) = mem::alloc_one_phys_page_hhdm();
    let ab_ref: &mut ApBoot = unsafe { &mut *(ab_va as *mut ApBoot) };
    mem::map_identity_4k(ab_pa & !0xfff); // ApBoot page

    let (cr3_frame, _) = x86_64::registers::control::Cr3::read();
    let pml4_pa = cr3_frame.start_address().as_u64();
    if pml4_pa >= (1u64 << 32) {
        kprintln!(
            "[SMP] FATAL: PML4 frame >= 4 GiB (0x{:x}) — 32-bit CR3 write will truncate",
            pml4_pa
        );
        loop {}
    }

    for c in m.cpus.iter().filter(|c| c.enabled) {
        if c.apic_id == bsp_id {
            continue;
        }

        // (b) Per-AP stack: 32 KiB VMAP (guaranteed mapped)
        const AP_STACK_PAGES: usize = 128; // 8 * 4KiB = 32KiB
        let stk =
            crate::mem::vmap_alloc_pages(AP_STACK_PAGES).expect("[SMP] vmap stack alloc failed");
        let stk_va = stk as u64;
        let stk_top = stk_va + (AP_STACK_PAGES as u64) * 4096 - 0x08;
        if stk_va == 0 {
            continue;
        }

        // (c) Fill ApBoot (BSP writes VA, AP will read PA we pass to trampoline)
        *ab_ref = ApBoot {
            ready_flag: 0,
            _pad: 0,
            cr3,
            gdt_ptr: 0,
            idt_ptr: 0,
            stack_top: stk_top, // <-- VA, valid under CR3
            entry64,
            hhdm: boot.hhdm_base, // for HHDM conversions on AP if needed
        };

        let frame = (stk_top) as *mut u64; // space for [arg][entry]
        unsafe { core::ptr::write(frame.add(0), &raw mut *ab_ref as u64) };

        // (d) Patch trampoline with **physical** address of ApBoot
        unsafe {
            ((tramp_virt + p32_off as u64) as *mut u32).write(ab_pa as u32);
            ((tramp_virt + p64_off as u64) as *mut u64).write(ab_pa);
            compiler_fence(Ordering::SeqCst);
        }

        // (e) Kick the AP: INIT → SIPI → SIPI
        without_interrupts(|| {
            apic::send_init(c.apic_id);
            spin_delay_us(10_000);
            apic::send_startup(c.apic_id, vector);
            spin_delay_us(200);
            apic::send_startup(c.apic_id, vector);
        });

        // (f) Wait for trampoline to set ready_flag = 1
        if !wait_ready(&ab_ref.ready_flag as *const u32, 4_000) {
            kprintln!("[SMP] apic_id {} did not signal ready in time", c.apic_id);
        }
    }
}

/// Very dumb spin delay until you wire your calibrated TSC helper.

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
        hlt();
    }
    false
}

/// What each AP runs after the trampoline puts us in 64-bit mode.
#[unsafe(no_mangle)]
pub extern "C" fn ap_entry(apboot: &mut ApBoot) -> ! {
    without_interrupts(|| {
        let boot: ApBoot = *apboot;
        apic::ap_init(unsafe { HHDM_BASE });
        kprintln!("Hello from {}", lapic_id());
        apboot.ready_flag = 1;
        tables::ap_init();
        kprintln!("Loaded GDT and IDT");
        let mut stk_va = 0;
        let mut stk_top = 0;
        access_mut(|e| {
            if !matches!(e.stub, None) {
                if !matches!(e.vector, None) {
                    if let Some(stack) = &e.stack {
                        let me = CpuId::me();
                        stk_va = &raw const stack.me(me).unwrap().dump[0] as u64;
                        stk_top = (stk_va + stack.me(me).unwrap().dump.len() as u64 - 0x10)
                            & !0xF;
                    }
                }
            }
        });
    });

    loop {
        x86_64::instructions::hlt();
    }
}
