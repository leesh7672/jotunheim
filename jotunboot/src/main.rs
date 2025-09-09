#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;

use alloc::vec::Vec;
use core::{arch::asm, mem::transmute, ptr};
use log::{error, info};
use uefi::prelude::*;
use uefi::{
    boot,
    boot::{AllocateType, MemoryType},
    fs::{FileSystem, Path},
};
use xmas_elf::ElfFile;
use xmas_elf::header::{Class, Data, Machine, Type as ElfType};
use xmas_elf::program::Type as PhType;

#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe {
        loop {
            asm!("hlt");
        }
    }
}

#[repr(C)]
pub struct BootInfo {
    pub load_base: u64,
    pub load_size: u64,
    pub load_bias: u64,
    pub entry: u64, // kernel entry VA
    pub mmap_ptr: *const u8,
    pub mmap_len: usize,
}

/* ================== Serial (QEMU `-serial stdio`) ================== */
#[inline(always)]
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
#[inline(always)]
unsafe fn serial_putc(c: u8) {
    const COM1: u16 = 0x3F8;
    loop {
        let mut lsr: u8;
        asm!("in al, dx", out("al") lsr, in("dx") COM1 + 5);
        if (lsr & 0x20) != 0 {
            break;
        } // THR empty
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

/* =================================== Entry =================================== */
#[entry]
fn main() -> Status {
    unsafe { serial_init() }
    serial_line(">>> JotunBoot entry");

    if uefi::helpers::init().is_ok() {
        serial_line("[serial] helpers::init OK");
    } else {
        serial_line("[serial][FATAL] helpers::init failed");
        unsafe {
            loop {
                asm!("hlt");
            }
        }
    }
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

    // ---- Unified handoff (PIE or EXEC) ----
    let entry_va = elf.header.pt2.entry_point();
    if !(min_vaddr..max_vaddr).contains(&entry_va) {
        slog!(
            "[serial][WARN] entry VA 0x{:x} not in [0x{:x}, 0x{:x})",
            entry_va,
            min_vaddr,
            max_vaddr
        );
        // You can `die(...)` here if you prefer a hard stop.
    }
    slog!("[serial] entry_va = 0x{:x}", entry_va);

    let bi_page = must_alloc_page(MemoryType::LOADER_DATA, "BootInfo");
    let tramp_page = must_alloc_page(MemoryType::LOADER_CODE, "trampoline");

    // Kernel stack
    let stack_pages = 8; // 32 KiB
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

    // Identity coverage must include trampoline/bootinfo/stack and the whole loaded image phys span.
    let tramp_end = tramp_page.as_ptr() as u64 + 0x1000;
    let bi_end = bi_page.as_ptr() as u64 + 0x1000;
    let image_end = load_base + total_size as u64;
    let mut ident_hi = *[tramp_end, bi_end, stack_top_aligned, image_end]
        .iter()
        .max()
        .unwrap();
    // Defensive: keep at least the first 1 GiB identity-mapped.
    let one_gib = 1u64 << 30;
    if ident_hi < one_gib {
        ident_hi = one_gib;
    }

    ident_hi = ident_hi.max(0xFEC0_0000 + 0x1000).max(0xFEE0_0000 + 0x1000);

    slog!("[serial] ident_hi = 0x{:x}", ident_hi);

    slog!("[serial] building page tables …");
    let pml4_phys = build_pagetables_exec(load_base, min_vaddr, max_vaddr, ident_hi)
        .unwrap_or_else(|_| die(Status::OUT_OF_RESOURCES, &format_args!("paging failed")));

    slog!("[serial] pml4_phys = 0x{:x}", pml4_phys);
    log_step("paging ready");

    let bi = BootInfo {
        load_base,
        load_size: total_size as u64,
        load_bias: load_base - min_vaddr, // useful for PIE
        entry: entry_va,
        mmap_ptr: core::ptr::null(),
        mmap_len: 0,
    };

    serial_line("[serial] ExitBootServices …");
    let _ = unsafe { boot::exit_boot_services(None) };

    unsafe {
        enter_kernel(
            pml4_phys,
            stack_top_sysv,
            entry_va,
            bi_page.as_ptr() as *const BootInfo,
        );
    }
}
#[inline(never)]
unsafe extern "sysv64" fn enter_kernel(
    pml4_phys: u64,
    stack_top_sysv: u64,
    entry_va: u64,
    bi_ptr: *const BootInfo,
) -> ! {
    unsafe {
        core::arch::asm!(
            "cli",
            // load CR3 = pml4_phys (in rdi)
            "mov rax, rdi",
            "mov cr3, rax",
            // switch to kernel stack (rsi)
            "mov rsp, rsi",
            // first arg to kernel = &BootInfo (rcx -> rdi under SysV)
            "mov rdi, rcx",
            // jump to entry (rdx)
            "jmp rdx",
            options(noreturn)
        );
    }
}

/* ================== Logging & helpers ================== */
#[inline(always)]
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
#[inline]
fn align_up(x: u64, a: u64) -> u64 {
    let m = a.max(1);
    (x + m - 1) & !(m - 1)
}
#[inline]
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

/* ================== Paging (EXEC+PIE path) ================== */
const PTE_P: u64 = 1 << 0;
const PTE_RW: u64 = 1 << 1;
// const PTE_US: u64 = 1 << 2;
const PTE_PS: u64 = 1 << 7; // 2 MiB large page
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

#[inline]
fn pml4_index(va: u64) -> usize {
    ((va >> 39) & 0x1ff) as usize
}
#[inline]
fn pdpt_index(va: u64) -> usize {
    ((va >> 30) & 0x1ff) as usize
}
#[inline]
fn pd_index(va: u64) -> usize {
    ((va >> 21) & 0x1ff) as usize
}
#[inline]
fn pt_index(va: u64) -> usize {
    ((va >> 12) & 0x1ff) as usize
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
// Identity 4K helper
fn map_4k_ident(pml4: *mut u64, start_va: u64, end_va: u64) -> Result<(), ()> {
    map_4k_offset(pml4, start_va, end_va, 0)
}

// Map a single 2 MiB page at VA → phys
unsafe fn map_2mib_page(pml4: *mut u64, va: u64, phys: u64) -> Result<(), ()> {
    let pdpt = ensure_pdpt(pml4, pml4_index(va))?;
    let pd = ensure_pd(pdpt, pdpt_index(va))?;
    if *pd.add(pd_index(va)) & PTE_P == 0 {
        *pd.add(pd_index(va)) = align_down(phys, 2 * 1024 * 1024) | PTE_P | PTE_RW | PTE_PS;
    }
    Ok(())
}

// Map the kernel VA range [min_vaddr..max_vaddr) to phys starting at `load_base`.
// Identity-map 0..ident_bytes (defensive: 4K for 0..2MiB, 2MiB pages thereafter) BUT
// skip any overlap with the kernel VA span to avoid PTE conflicts.
fn build_pagetables_exec(
    load_base: u64,
    min_vaddr: u64,
    max_vaddr: u64,
    ident_bytes: u64,
) -> Result<u64, ()> {
    let (pml4, pml4_phys) = alloc_zero_page(MemoryType::LOADER_DATA).ok_or(())?;
    let first_2mib_end = 2 * 1024 * 1024;

    // Map kernel range by constant offset
    let delta = load_base as i128 - min_vaddr as i128;

    // Kernel low slice (<2MiB) via 4K pages
    if min_vaddr < first_2mib_end {
        let low_end = core::cmp::min(max_vaddr, first_2mib_end);
        map_4k_offset(pml4, min_vaddr, low_end, delta)?;
    }
    // Kernel remainder via 2MiB pages
    let mut big_va = core::cmp::max(first_2mib_end, align_up(min_vaddr, 2 * 1024 * 1024));
    let big_end = align_up(max_vaddr, 2 * 1024 * 1024);
    while big_va < big_end {
        let phys = ((big_va as i128) + delta) as u64;
        unsafe {
            map_2mib_page(pml4, big_va, phys)?;
        }
        big_va += 2 * 1024 * 1024;
    }

    // Identity-map 0..2MiB except the kernel's low slice
    let id0_start = 0u64;
    let id0_end = first_2mib_end;
    if id0_start < core::cmp::min(min_vaddr, id0_end) {
        map_4k_ident(pml4, id0_start, core::cmp::min(min_vaddr, id0_end))?;
    }
    if max_vaddr < id0_end {
        map_4k_ident(pml4, max_vaddr, id0_end)?;
    }

    // Identity-map [2MiB .. ident_bytes) with 2MiB pages, skipping any overlap with kernel VA span
    let mut va = core::cmp::max(first_2mib_end, align_down(0, 2 * 1024 * 1024));
    let ident_end = align_up(ident_bytes, 2 * 1024 * 1024);
    while va < ident_end {
        let overlap_kernel = !(va + 2 * 1024 * 1024 <= min_vaddr || va >= max_vaddr);
        if !overlap_kernel {
            unsafe {
                map_2mib_page(pml4, va, va)?;
            }
        }
        va += 2 * 1024 * 1024;
    }
    Ok(pml4_phys)
}
