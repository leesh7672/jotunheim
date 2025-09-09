#![allow(clippy::missing_safety_doc)]

use crate::mem::mapper::active_offset_mapper;
use crate::mem::simple_alloc::TinyBump;
use crate::println;

use x86_64::VirtAddr;
use x86_64::structures::paging::{Mapper, Page, PageTableFlags as F, Size2MiB};

fn enforce_mmio_flags_2m<M: Mapper<Size2MiB>>(mapper: &mut M, va_2m: u64) {
    let page2m = Page::<Size2MiB>::containing_address(VirtAddr::new(va_2m));
    // MMIO via PDE (PS=1): P | RW | NX | PWT | PCD  (no GLOBAL)
    let want = F::PRESENT | F::WRITABLE | F::NO_EXECUTE | F::WRITE_THROUGH | F::NO_CACHE;
    unsafe {
        if let Ok(flush) = mapper.update_flags(page2m, want) {
            flush.flush();
        }
    }
}

pub fn early_map_mmio_for_apics() {
    let alloc = TinyBump::new(0x0030_0000, 0x0031_0000);
    let mut mapper = unsafe { active_offset_mapper(0) };

    // --- FAST PATH: keep the loader’s 2 MiB identity mapping ---
    // Attempt to update flags on the existing 2 MiB PDEs, then flush TLBs.
    enforce_mmio_flags_2m(&mut mapper, 0xFEC0_0000);
    enforce_mmio_flags_2m(&mut mapper, 0xFEE0_0000);

    // TLB shootdown (covers any 2 MiB entries, with global toggle too)
    use x86_64::registers::control::Cr3;
    unsafe {
        let (cr3, flags) = Cr3::read();
        Cr3::write(cr3, flags); // flush non-global
    }
}

/* -------------------- Debug helpers: dump current mapping -------------------- */

#[derive(Copy, Clone)]
struct Levels {
    pml4e: u64,
    pdpte: Option<u64>,
    pde: Option<u64>,
    pte: Option<u64>,
}

// Walk the current tables (CR3) and collect entries for a VA.
// `phys_offset` is the virtual base where physical memory is linearly mapped.
// While on the loader tables, this is 0 (identity).
fn dump_va_mapping(va: u64, phys_offset: u64) -> Levels {
    use x86_64::{VirtAddr, registers::control::Cr3};

    // Convert a physical page-table address to a virtual pointer using the window.
    unsafe fn phys_to_virt(phys: u64, phys_offset: u64) -> *const u64 {
        ((phys & !0xfff) + phys_offset) as *const u64
    }

    let (pml4_frame, _) = Cr3::read();
    let pml4_phys = pml4_frame.start_address().as_u64();

    let mut out = Levels {
        pml4e: 0,
        pdpte: None,
        pde: None,
        pte: None,
    };

    let v = VirtAddr::new(va);
    let pml4i = ((v.as_u64() >> 39) & 0x1ff) as usize;
    let pdpti = ((v.as_u64() >> 30) & 0x1ff) as usize;
    let pdi = ((v.as_u64() >> 21) & 0x1ff) as usize;
    let pti = ((v.as_u64() >> 12) & 0x1ff) as usize;

    unsafe {
        let pml4 = phys_to_virt(pml4_phys, phys_offset);
        let pml4e = *pml4.add(pml4i);
        out.pml4e = pml4e;

        if pml4e & 1 == 0 {
            return out;
        }
        let pdpt_phys = pml4e & 0x000F_FFFF_FFFF_F000;
        let pdpt = phys_to_virt(pdpt_phys, phys_offset);
        let pdpte = *pdpt.add(pdpti);
        out.pdpte = Some(pdpte);

        if pdpte & 1 == 0 {
            return out;
        }
        if pdpte & (1 << 7) != 0 {
            // 1GiB large page
            return out;
        }
        let pd_phys = pdpte & 0x000F_FFFF_FFFF_F000;
        let pd = phys_to_virt(pd_phys, phys_offset);
        let pde = *pd.add(pdi);
        out.pde = Some(pde);

        if pde & 1 == 0 {
            return out;
        }
        if pde & (1 << 7) != 0 {
            // 2MiB large page
            return out;
        }
        let pt_phys = pde & 0x000F_FFFF_FFFF_F000;
        let pt = phys_to_virt(pt_phys, phys_offset);
        let pte = *pt.add(pti);
        out.pte = Some(pte);
    }

    out
}

pub fn log_va_mapping(tag: &str, va: u64, phys_offset: u64) {
    let lev = dump_va_mapping(va, phys_offset);
    println!(
        "[PT] {tag} VA={:#016x} PML4E={:#018x} PDPTE={:?} PDE={:?} PTE={:?}",
        va, lev.pml4e, lev.pdpte, lev.pde, lev.pte
    );
}
