// SPDX-License-Identifier: JOSSL-1.0
// Copyright (C) 2025 The Jotunheim Project
#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod simd;

use alloc::vec::Vec;
use core::{arch::asm, ptr};

use log::{error, info};
use uefi::boot::{AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::{
    boot,
    fs::{FileSystem, Path},
};

use xmas_elf::ElfFile;
use xmas_elf::header::{Class, Data, Machine, Type as ElfType};
use xmas_elf::program::Type as PhType;

const HHDM_BASE: u64 = 0xffff_8880_0000_0000;

/* ============================ Global allocator ============================ */

#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

/* ================================ Panic ================================== */

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe {
        loop {
            asm!("hlt");
        }
    }
}

/* =========================== Kernel-facing ABI =========================== */

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Framebuffer {
    pub addr: u64, // physical address of linear framebuffer
    pub width: u32,
    pub height: u32,
    pub pitch: u32,        // bytes per scanline
    pub bpp: u32,          // bits per pixel (commonly 32)
    pub pixel_format: u32, // kernel enum/discriminant
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MemoryRegion {
    pub phys_start: u64,
    pub virt_start: u64, // 0 at boot (or phys+offset if you prefer)
    pub len: u64,
    pub typ: u32,  // kernel enum/discriminant
    pub attr: u64, // attribute bits
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BootInfo {
    pub rsdp_addr: u64,
    pub memory_map: *const MemoryRegion,
    pub memory_map_len: usize,
    pub framebuffer: Framebuffer,
    pub kernel_phys_base: u64,
    pub kernel_virt_base: u64,
    pub early_heap_paddr: u64,
    pub early_heap_len: u64,
    pub hhdm_base: u64,
    pub low32_pool_paddr: u64,
    pub low32_pool_len: u64,
}

/* ========================== Serial (QEMU stdio) ========================== */


unsafe fn serial_init() {
    const COM1: u16 = 0x3F8;
    asm!("out dx, al", in("dx") COM1 + 1, in("al") 0u8);
    asm!("out dx, al", in("dx") COM1 + 3, in("al") 0x80u8);
    asm!("out dx, al", in("dx") COM1 + 0, in("al") 0x01u8);
    asm!("out dx, al", in("dx") COM1 + 1, in("al") 0x00u8);
    asm!("out dx, al", in("dx") COM1 + 3, in("al") 0x03u8);
    asm!("out dx, al", in("dx") COM1 + 2, in("al") 0xC7u8);
    asm!("out dx, al", in("dx") COM1 + 4, in("al") 0x0Bu8);
}

unsafe fn serial_putc(c: u8) {
    const COM1: u16 = 0x3F8;
    loop {
        let mut lsr: u8;
        asm!("in al, dx", out("al") lsr, in("dx") COM1 + 5);
        if (lsr & 0x20) != 0 {
            break;
        }
    }
    asm!("out dx, al", in("dx") COM1, in("al") c);
}
fn serial_line(s: &str) {
    unsafe {
        for b in s.bytes() {
            serial_putc(b);
        }
        serial_putc(b'\r');
        serial_putc(b'\n');
    }
}
macro_rules! slog {
    ($($t:tt)*) => {{
        let s = alloc::format!($($t)*);
        serial_line(&s);
    }};
}

/* ============================ Small utilities ============================ */


fn log_step(msg: &str) {
    info!("[step] {msg}");
    boot::stall(80_000);
}
#[cold]
fn die(_: Status, msg: &core::fmt::Arguments) -> ! {
    error!("[fatal] {}", msg);
    serial_line("[serial][FATAL] abort");
    boot::stall(1_000_000);
    unsafe {
        loop {
            asm!("hlt");
        }
    }
}

fn align_up(x: u64, a: u64) -> u64 {
    let m = a.max(1);
    (x + m - 1) & !(m - 1)
}

fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}
fn must_alloc_page(kind: MemoryType, name: &str) -> core::ptr::NonNull<u8> {
    boot::allocate_pages(AllocateType::AnyPages, kind, 1).unwrap_or_else(|e| {
        die(
            Status::OUT_OF_RESOURCES,
            &format_args!("alloc {name} {:?}", e),
        )
    })
}

/* =========================== ACPI/GOP/MemMap ============================ */

use core::cell::Cell;

fn find_rsdp() -> u64 {
    use uefi::{system, table::cfg};
    let rsdp = Cell::new(0u64);
    system::with_config_table(|ct| {
        for e in ct {
            if e.guid == cfg::ACPI2_GUID || e.guid == cfg::ACPI_GUID {
                rsdp.set(e.address as u64); // interior mutability; OK in Fn
                break;
            }
        }
    });
    rsdp.get()
}

fn get_framebuffer() -> Framebuffer {
    use uefi::proto::console::gop::GraphicsOutput;

    // Find & open GOP
    let h = boot::get_handle_for_protocol::<GraphicsOutput>().expect("No GOP handle found");
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(h).expect("Open GOP failed");

    let info = gop.current_mode_info();
    let (w, h) = info.resolution();
    let mut fb = gop.frame_buffer();

    // Map PixelFormat to your kernel enum if you need; here 0=RGB,1=BGR,2=Bitmask,3=BltOnly
    let pf = match info.pixel_format() {
        uefi::proto::console::gop::PixelFormat::Rgb => 0,
        uefi::proto::console::gop::PixelFormat::Bgr => 1,
        uefi::proto::console::gop::PixelFormat::Bitmask => 2,
        uefi::proto::console::gop::PixelFormat::BltOnly => 3,
    };

    Framebuffer {
        addr: fb.as_mut_ptr() as u64,
        width: w as u32,
        height: h as u32,
        pitch: (info.stride() as u32) * 4,
        bpp: 32,
        pixel_format: pf,
    }
}

fn uefi_type_to_kernel(t: boot::MemoryType) -> u32 {
    use boot::MemoryType as U;
    match t {
        U::CONVENTIONAL => 1,
        U::LOADER_CODE => 2,
        U::LOADER_DATA => 3,
        U::BOOT_SERVICES_CODE => 4,
        U::BOOT_SERVICES_DATA => 5,
        U::RUNTIME_SERVICES_CODE => 6,
        U::RUNTIME_SERVICES_DATA => 7,
        U::ACPI_RECLAIM => 8,
        _ => 0,
    }
}

fn build_memory_regions_vec() -> Vec<MemoryRegion> {
    // Newer uefi crate API: pass a MemoryType; returns an owned map you can iterate.
    let mm = boot::memory_map(MemoryType::LOADER_DATA).expect("memory_map");
    let mut out = Vec::new();
    for d in mm.entries() {
        let len = (d.page_count as u64) * 4096;
        out.push(MemoryRegion {
            phys_start: d.phys_start as u64,
            virt_start: 0,
            len,
            typ: uefi_type_to_kernel(d.ty),
            attr: d.att.bits() as u64,
        });
    }
    out
}

/* ================================ Paging ================================= */

const PTE_P: u64 = 1 << 0;
const PTE_RW: u64 = 1 << 1;
const PTE_PS: u64 = 1 << 7; // 2 MiB page
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

fn is_aligned(x: u64, a: u64) -> bool {
    (x & (a - 1)) == 0
}

fn pml4_index(va: u64) -> usize {
    ((va >> 39) & 0x1ff) as usize
}

fn pdpt_index(va: u64) -> usize {
    ((va >> 30) & 0x1ff) as usize
}

fn pd_index(va: u64) -> usize {
    ((va >> 21) & 0x1ff) as usize
}

fn pt_index(va: u64) -> usize {
    ((va >> 12) & 0x1ff) as usize
}

fn alloc_zero_page_low(kind: MemoryType) -> Option<(*mut u64, u64)> {
    let p = boot::allocate_pages(AllocateType::MaxAddress(0x0000_FFFF_FFFF_F000), kind, 1).ok()?;
    let phys = p.as_ptr() as u64;
    unsafe { core::ptr::write_bytes(p.as_ptr(), 0, 4096) };
    Some((p.as_ptr() as *mut u64, phys))
}

fn alloc_zero_page(kind: MemoryType) -> Option<(*mut u64, u64)> {
    let p = boot::allocate_pages(AllocateType::AnyPages, kind, 1).ok()?;
    let phys = p.as_ptr() as usize as u64;
    unsafe { core::ptr::write_bytes(p.as_ptr(), 0, 4096) };
    Some((p.as_ptr() as *mut u64, phys))
}

unsafe fn ensure_pdpt(pml4: *mut u64, pml4i: usize) -> Result<*mut u64, ()> {
    let e = *pml4.add(pml4i);
    if e & PTE_P == 0 {
        let (pdpt, phys) = alloc_zero_page(MemoryType::LOADER_DATA).ok_or(())?;
        *pml4.add(pml4i) = phys | PTE_P | PTE_RW;
        Ok(pdpt)
    } else {
        Ok((e & ADDR_MASK) as *mut u64)
    }
}
unsafe fn ensure_pd(pdpt: *mut u64, pdpti: usize) -> Result<*mut u64, ()> {
    let e = *pdpt.add(pdpti);
    if e & PTE_P == 0 {
        let (pd, phys) = alloc_zero_page(MemoryType::LOADER_DATA).ok_or(())?;
        *pdpt.add(pdpti) = phys | PTE_P | PTE_RW;
        Ok(pd)
    } else {
        if e & PTE_PS != 0 {
            return Err(());
        }
        Ok((e & ADDR_MASK) as *mut u64)
    }
}
unsafe fn ensure_pt(pd: *mut u64, pdi: usize) -> Result<*mut u64, ()> {
    let e = *pd.add(pdi);
    if e & PTE_P == 0 {
        let (pt, phys) = alloc_zero_page(MemoryType::LOADER_DATA).ok_or(())?;
        *pd.add(pdi) = phys | PTE_P | PTE_RW;
        Ok(pt)
    } else {
        if e & PTE_PS != 0 {
            return Err(());
        }
        Ok((e & ADDR_MASK) as *mut u64)
    }
}

// Map [start_va, end_va) with 4 KiB pages, phys = va + delta
fn map_4k_offset(pml4: *mut u64, start_va: u64, end_va: u64, delta: i128) -> Result<(), ()> {
    let mut va = align_down(start_va, 0x1000);
    let end = align_up(end_va, 0x1000);
    while va < end {
        unsafe {
            let pdpt = ensure_pdpt(pml4, pml4_index(va))?;
            let pd = ensure_pd(pdpt, pdpt_index(va))?;
            let pt = ensure_pt(pd, pd_index(va))?;
            let phys = ((va as i128) + delta) as u64 & ADDR_MASK;
            *pt.add(pt_index(va)) = phys | PTE_P | PTE_RW;
        }
        va += 0x1000;
    }
    Ok(())
}
fn map_4k_ident(pml4: *mut u64, start_va: u64, end_va: u64) -> Result<(), ()> {
    map_4k_offset(pml4, start_va, end_va, 0)
}
// Replace your map_4kib_page with a real 4KiB PTE writer.
unsafe fn map_4kib_page(pml4: *mut u64, va: u64, phys: u64) -> Result<(), ()> {
    let pdpt = ensure_pdpt(pml4, pml4_index(va))?;
    let pd = ensure_pd(pdpt, pdpt_index(va))?;
    let pt = ensure_pt(pd, pd_index(va))?;

    let pte = pt.add(pt_index(va));
    if (*pte & PTE_P) == 0 {
        *pte = (phys & ADDR_MASK) | PTE_P | PTE_RW; // ← PTE, NO PS bit
    }
    Ok(())
}

unsafe fn map_hhdm_huge(pml4: *mut u64, phys_max: u64) -> Result<(), ()> {
    let mut phys = 0u64;

    // 1 GiB chunks
    while phys < phys_max {
        if phys_max - phys >= (1 << 30)
            && is_aligned(phys, 1 << 30)
            && is_aligned(HHDM_BASE + phys, 1 << 30)
        {
            let va = HHDM_BASE + phys;
            let l4 = pml4_index(va);
            let l3 = pdpt_index(va);
            let pdpt = ensure_pdpt(pml4, l4)?;
            // install a HUGE 1GiB PDE at PDPT level:
            let e = pdpt.add(l3);
            if (*e & PTE_P) == 0 {
                *e = (phys & ADDR_MASK) | PTE_P | PTE_RW | PTE_PS; // 1GiB page
            }
            phys += 1 << 30;
        } else {
            break;
        }
    }

    // 2 MiB chunks
    while phys < phys_max {
        if phys_max - phys >= (2 << 20)
            && is_aligned(phys, 2 << 20)
            && is_aligned(HHDM_BASE + phys, 2 << 20)
        {
            let va = HHDM_BASE + phys;
            let pdpt = ensure_pdpt(pml4, pml4_index(va))?;
            let pd = ensure_pd(pdpt, pdpt_index(va))?;
            let e = pd.add(pd_index(va));
            if (*e & PTE_P) == 0 {
                *e = (phys & ADDR_MASK) | PTE_P | PTE_RW | PTE_PS; // 2MiB page
            }
            phys += 2 << 20;
        } else {
            break;
        }
    }

    // 4 KiB tail
    while phys < phys_max {
        let va = HHDM_BASE + phys;
        map_4kib_page(pml4, va, phys)?;
        phys += 4096;
    }

    Ok(())
}

fn build_pagetables_exec(
    load_base: u64,
    min_vaddr: u64,
    max_vaddr: u64,
    ident_bytes: u64,
    phys_max: u64,
) -> Result<u64, ()> {
    let (pml4, pml4_phys) = alloc_zero_page_low(MemoryType::LOADER_DATA).ok_or(())?;
    let two_mib = 2 * 1024 * 1024u64;
    let first_2mib_end = two_mib;

    // constant offset VA->PA
    let delta = load_base as i128 - min_vaddr as i128;

    // Low slice: 4 KiB
    if min_vaddr < first_2mib_end {
        let low_end = core::cmp::min(max_vaddr, first_2mib_end);
        map_4k_offset(pml4, min_vaddr, low_end, delta)?;
    }

    // Remainder: use 2 MiB only if delta is 2 MiB aligned; else 4 KiB.
    let rem_start = core::cmp::max(first_2mib_end, min_vaddr);

    map_4k_offset(pml4, rem_start, max_vaddr, delta)?;

    // Identity low [0..2MiB) around the kernel’s low slice
    let id0_end = first_2mib_end;
    if 0 < core::cmp::min(min_vaddr, id0_end) {
        map_4k_ident(pml4, 0x1000, core::cmp::min(min_vaddr, id0_end))?; // (optional) leave VA 0 unmapped
    }
    if max_vaddr < id0_end {
        map_4k_ident(pml4, max_vaddr, id0_end)?;
    }

    let mut va = core::cmp::max(first_2mib_end, 0);
    let ident_end = align_up(ident_bytes, 0x1000);
    while va < ident_end {
        let overlap_kernel = !(va + 0x1000 <= min_vaddr || va >= max_vaddr);
        if !overlap_kernel {
            unsafe {
                map_4kib_page(pml4, va, va)?;
            }
        }
        va += 0x1000;
    }

    unsafe {
        map_hhdm_huge(pml4, align_up(phys_max, 0x1000))?;
    }
    Ok(pml4_phys)
}

/* ========================= Low trampoline (blob) ========================= */


unsafe fn enter_kernel_via_trampoline(
    tramp_page: core::ptr::NonNull<u8>,
    pml4_phys: u64,
    stack_top_sysv: u64,
    entry_va: u64,
    bi_ptr: *const BootInfo,
) -> ! {
    // cli; mov cr3, rdi; mov rsp, rsi; mov rdi, rcx; jmp rdx
    const CODE: [u8; 12] = [
        0xFA, // cli
        0x0F, 0x22, 0xDF, // mov cr3, rdi
        0x48, 0x89, 0xF4, // mov rsp, rsi
        0x48, 0x89, 0xCF, // mov rdi, rcx
        0xFF, 0xE2, // jmp rdx
    ];

    core::ptr::copy_nonoverlapping(CODE.as_ptr(), tramp_page.as_ptr(), CODE.len());

    let tramp: extern "sysv64" fn(u64, u64, u64, *const BootInfo) -> ! =
        core::mem::transmute(tramp_page.as_ptr());
    tramp(pml4_phys, stack_top_sysv, entry_va, bi_ptr);
}

/* ================================= Entry ================================= */

#[entry]
fn main() -> Status {
    unsafe { serial_init() }
    serial_line(">>> JotunBoot entry");

    if uefi::helpers::init().is_err() {
        serial_line("[serial][FATAL] helpers::init failed");
        unsafe {
            loop {
                asm!("hlt");
            }
        }
    }
    simd::enable_sse_avx_boot();
    log_step("loader start.");

    // ---- FS & read kernel ----
    serial_line("[serial] acquiring FileSystem.");
    let image = boot::image_handle();
    let mut fs: FileSystem = match boot::get_image_file_system(image) {
        Ok(p) => {
            serial_line("[serial] FileSystem OK");
            p.into()
        }
        Err(e) => die(
            Status::LOAD_ERROR,
            &format_args!("get_image_file_system failed: {:?}", e),
        ),
    };
    log_step("fs ok");

    let elf_path = Path::new(cstr16!(r"\JOTUNHEIM\KERNEL.ELF"));
    serial_line("[serial] reading \\JOTUNHEIM\\KERNEL.ELF.");
    let elf_bytes: Vec<u8> = match fs.read(elf_path) {
        Ok(v) => {
            slog!("[serial] kernel bytes = {}", v.len());
            v
        }
        Err(e) => die(
            Status::NOT_FOUND,
            &format_args!("read KERNEL.ELF failed: {:?}", e),
        ),
    };
    info!("kernel bytes = {}", elf_bytes.len());

    // ---- Parse ELF ----
    serial_line("[serial] parsing ELF …");
    let elf = ElfFile::new(&elf_bytes)
        .unwrap_or_else(|_| die(Status::LOAD_ERROR, &format_args!("ELF parse error")));
    if elf.header.pt1.class() != Class::SixtyFour
        || elf.header.pt1.data() != Data::LittleEndian
        || elf.header.pt2.machine().as_machine() != Machine::X86_64
    {
        die(Status::LOAD_ERROR, &format_args!("bad ELF header"));
    }
    let elf_ty = elf.header.pt2.type_().as_type();
    serial_line(match elf_ty {
        ElfType::Executable => "[serial] ELF type = EXEC",
        ElfType::SharedObject => "[serial] ELF type = PIE",
        _ => "[serial] ELF type = OTHER",
    });
    log_step("ELF header ok");

    // ---- Layout PT_LOADs ----
    let (min_vaddr, max_vaddr, max_align) = {
        let mut min = u64::MAX;
        let mut max = 0;
        let mut align = 0;
        for ph in elf.program_iter() {
            if ph.get_type().ok() == Some(PhType::Load) {
                let v = ph.virtual_addr();
                let m = ph.mem_size();
                if m == 0 {
                    continue;
                }
                min = min.min(v);
                max = max.max(align_up(v + m, 0x1000));
                align = align.max(ph.align().max(0x1000));
            }
        }
        if min == u64::MAX {
            die(Status::LOAD_ERROR, &format_args!("no PT_LOAD"));
        }
        (min, max, align)
    };
    slog!(
        "[serial] layout: min=0x{:x} max=0x{:x} align=0x{:x}",
        min_vaddr,
        max_vaddr,
        max_align
    );
    info!(
        "layout: min=0x{:x} max=0x{:x} size={} align=0x{:x}",
        min_vaddr,
        max_vaddr,
        (max_vaddr - min_vaddr) as usize,
        max_align
    );

    // ---- Allocate contiguous phys & copy segments ----
    let total_size = (max_vaddr - min_vaddr) as usize;
    let reserve = total_size + (max_align as usize) + 0x1000;
    let pages = (reserve + 0xFFF) / 0x1000;
    slog!("[serial] allocate {} pages for image", pages);
    let raw_base = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .unwrap_or_else(|e| {
            die(
                Status::OUT_OF_RESOURCES,
                &format_args!("alloc image {:?}", e),
            )
        });
    let load_base = align_up(raw_base.as_ptr() as u64, max_align);
    unsafe { ptr::write_bytes(load_base as *mut u8, 0, total_size) };

    for ph in elf.program_iter() {
        if ph.get_type().ok() != Some(PhType::Load) {
            continue;
        }
        let fsz = ph.file_size() as usize;
        let msz = ph.mem_size() as usize;
        if msz == 0 {
            continue;
        }
        let off = ph.offset() as usize;
        let rel = ph.virtual_addr() - min_vaddr;
        let dst = (load_base + rel) as *mut u8;
        unsafe {
            if fsz > 0 {
                let src = &elf_bytes[off..off + fsz];
                ptr::copy_nonoverlapping(src.as_ptr(), dst, fsz);
            }
            if msz > fsz {
                ptr::write_bytes(dst.add(fsz), 0, msz - fsz);
            }
        }
    }
    serial_line("[serial] segments copied");
    log_step("segments copied");

    // ---- Handoff preparation ----
    let entry_va = elf.header.pt2.entry_point();
    if !(min_vaddr..max_vaddr).contains(&entry_va) {
        slog!(
            "[serial][WARN] entry VA 0x{:x} not in [0x{:x}, 0x{:x})",
            entry_va,
            min_vaddr,
            max_vaddr
        );
    }
    slog!("[serial] entry_va = 0x{:x}", entry_va);

    let low32_pages = 512usize; // 2 MiB pool; adjust as you like
    let low32_block = boot::allocate_pages(
        AllocateType::MaxAddress(0xFFFF_FFFF),
        MemoryType::LOADER_DATA,
        low32_pages,
    )
    .unwrap_or_else(|e| {
        die(
            Status::OUT_OF_RESOURCES,
            &format_args!("low32 pool {:?}", e),
        )
    });

    let low32_pool_paddr = low32_block.as_ptr() as u64;
    let low32_pool_len = (low32_pages as u64) * 4096;
    slog!("[serial] low32_pool_paddr: {}", low32_pool_paddr);
    slog!("[serial] low32_pool_len: {}", low32_pool_len);

    let bi_page = must_alloc_page(MemoryType::LOADER_DATA, "BootInfo");
    let tramp_page = must_alloc_page(MemoryType::LOADER_CODE, "trampoline");

    let stack_pages = 16usize;
    let stack_base =
        boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, stack_pages)
            .unwrap_or_else(|e| {
                die(
                    Status::OUT_OF_RESOURCES,
                    &format_args!("alloc stack {:?}", e),
                )
            });
    let stack_top = stack_base.as_ptr() as u64 + (stack_pages as u64) * 4096;
    let stack_top_aligned = stack_top & !0xFu64;
    let stack_top_sysv = stack_top_aligned.wrapping_sub(8);
    slog!("[serial] stack_top_sysv  = 0x{:x}", stack_top_sysv);
    slog!("[serial] tramp_page = 0x{:x}", tramp_page.as_ptr() as u64);
    slog!("[serial] bootinfo   = 0x{:x}", bi_page.as_ptr() as u64);
    slog!("[serial] stack_top  = 0x{:x}", stack_top_aligned);

    const EARLY_HEAP_PAGES: usize = 0x4000;
    let early_heap = boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        EARLY_HEAP_PAGES,
    )
    .unwrap_or_else(|e| {
        die(
            Status::OUT_OF_RESOURCES,
            &format_args!("early heap {:?}", e),
        )
    });
    let early_heap_paddr = early_heap.as_ptr() as u64;
    let early_heap_len = (EARLY_HEAP_PAGES * 4096) as u64;

    // Copy UEFI memory map into our own buffer
    let regions = build_memory_regions_vec();

    let phys_max = regions
        .iter()
        .map(|r| r.phys_start.saturating_add(r.len))
        .max()
        .unwrap_or(0);

    let map_bytes = core::mem::size_of::<MemoryRegion>() * regions.len();
    let map_pages = (map_bytes + 0xFFF) / 0x1000;
    let memmap_pages =
        boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, map_pages)
            .unwrap_or_else(|e| {
                die(
                    Status::OUT_OF_RESOURCES,
                    &format_args!("memmap pages {:?}", e),
                )
            });
    unsafe {
        core::ptr::copy_nonoverlapping(
            regions.as_ptr() as *const u8,
            memmap_pages.as_ptr(),
            map_bytes,
        );
    }
    let memory_map_ptr = memmap_pages.as_ptr() as *const MemoryRegion;
    let memory_map_len = regions.len();

    // GOP framebuffer & ACPI RSDP
    let fb = get_framebuffer();
    let rsdp_addr = find_rsdp();

    // Identity coverage must include trampoline/bootinfo/stack/image span/early heap/memmap/fb.
    let tramp_end = tramp_page.as_ptr() as u64 + 0x1000;
    let bi_end = bi_page.as_ptr() as u64 + 0x1000;
    let stack_end = stack_top_aligned;
    let image_end = load_base + (max_vaddr - min_vaddr);
    let early_heap_end = early_heap_paddr + early_heap_len;
    let memmap_end = memmap_pages.as_ptr() as u64 + (map_pages as u64) * 4096;
    let fb_end = fb.addr + (fb.pitch as u64) * (fb.height as u64);

    let mut ident_hi = *[
        tramp_end,
        bi_end,
        stack_end,
        image_end,
        early_heap_end,
        memmap_end,
        fb_end,
    ]
    .iter()
    .max()
    .unwrap();

    // Defensive floor (1 GiB) and APIC MMIO
    let one_gib = 1u64 << 30;
    if ident_hi < one_gib {
        ident_hi = one_gib;
    }
    ident_hi = ident_hi.max(0xFEC0_0000 + 0x1000).max(0xFEE0_0000 + 0x1000);

    slog!("[serial] ident_hi = 0x{:x}", ident_hi);

    slog!("[serial] building page tables …");
    let pml4_phys = build_pagetables_exec(load_base, min_vaddr, max_vaddr, ident_hi, phys_max)
        .unwrap_or_else(|_| die(Status::OUT_OF_RESOURCES, &format_args!("paging failed")));
    slog!("[serial] pml4_phys = 0x{:x}", pml4_phys);
    log_step("paging ready");

    // Persist BootInfo
    let bi_val = BootInfo {
        rsdp_addr,
        memory_map: memory_map_ptr,
        memory_map_len,
        framebuffer: fb,
        kernel_phys_base: load_base,
        kernel_virt_base: min_vaddr,
        early_heap_paddr: early_heap_paddr,
        early_heap_len: early_heap_len,
        hhdm_base: HHDM_BASE,
        low32_pool_len,
        low32_pool_paddr,
    };
    unsafe {
        (bi_page.as_ptr() as *mut BootInfo).write(bi_val);
    }

    // ExitBootServices and jump via low trampoline (identity mapped in both CR3s)
    serial_line("[serial] ExitBootServices …");
    let _ = unsafe { boot::exit_boot_services(None) };

    unsafe {
        enter_kernel_via_trampoline(
            tramp_page,
            pml4_phys,
            stack_top_sysv,
            entry_va,
            bi_page.as_ptr() as *const BootInfo,
        );
    }
}
