// src/acpi/madt.rs
#![allow(clippy::missing_safety_doc)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::mem::size_of;

use crate::acpi::{CpuEntry, IoApic, MadtInfo};
use crate::bootinfo::BootInfo;
use crate::kprintln;

// ───────────────────── RSDP/RSDT/XSDT headers ─────────────────────

#[repr(C, packed)]
struct Rsdp10 {
    sig: [u8; 8], // "RSD PTR "
    checksum: u8, // sum of first 20 bytes == 0
    oem_id: [u8; 6],
    rev: u8, // 0 for ACPI 1.0, >=2 means 2.0+
    rsdt_addr: u32,
}

#[repr(C, packed)]
struct Rsdp20 {
    // first 20 bytes are identical to RSDP 1.0
    sig: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    rev: u8,
    rsdt_addr: u32,
    // extended
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8, // checksum over entire length
    _reserved: [u8; 3],
}

#[repr(C, packed)]
struct SdtHeader {
    sig: [u8; 4],
    length: u32,
    _rev: u8,
    _checksum: u8,
    _oem_id: [u8; 6],
    _oem_table_id: [u8; 8],
    _oem_rev: u32,
    _creator_id: u32,
    _creator_rev: u32,
}

#[repr(C, packed)]
struct MadtHeader {
    header: SdtHeader, // "APIC"
    lapic_mmio: u32,   // legacy xAPIC MMIO (may be overridden by entry type 5)
    flags: u32,
}

// MADT entry common header
#[derive(Copy, Clone)]
#[repr(C, packed)]
struct MadtEntryHeader {
    typ: u8,
    len: u8,
}

// Entry types we care about
const PLAPIC: u8 = 0;
const IOAPIC: u8 = 1;
const LAPIC_ADDR_OVERRIDE: u8 = 5;
const PLX2APIC: u8 = 9;

// ─────────────────────────── helpers ───────────────────────────

fn checksum_ok(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)) == 0
}

fn read_phys_slice(hhdm: u64, phys: u64, len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts((hhdm + phys) as *const u8, len) }
}

fn sdt_valid(hhdm: u64, phys: u64) -> Option<SdtHeader> {
    let hdr_bytes = read_phys_slice(hhdm, phys, size_of::<SdtHeader>());
    // Copy the header into a local value (avoids aliasing packed ref pitfalls)
    let mut hdr = SdtHeader {
        sig: [0; 4],
        length: 0,
        _rev: 0,
        _checksum: 0,
        _oem_id: [0; 6],
        _oem_table_id: [0; 8],
        _oem_rev: 0,
        _creator_id: 0,
        _creator_rev: 0,
    };
    hdr.sig.copy_from_slice(&hdr_bytes[0..4]);
    hdr.length = u32::from_le_bytes(hdr_bytes[4..8].try_into().unwrap());
    hdr._rev = hdr_bytes[8];
    hdr._checksum = hdr_bytes[9];
    // We won’t need the rest to check length+checksum
    if hdr.length < size_of::<SdtHeader>() as u32 {
        return None;
    }
    if !checksum_ok(read_phys_slice(hhdm, phys, hdr.length as usize)) {
        return None;
    }
    Some(hdr)
}

// Search XSDT (64-bit entry array)
fn find_sdt_by_sig_xsdt(hhdm: u64, xsdt_phys: u64, want: &[u8; 4]) -> Option<(u64, u32)> {
    let xsdt = sdt_valid(hhdm, xsdt_phys)?;
    let entries = ((xsdt.length as usize) - size_of::<SdtHeader>()) / 8;
    for i in 0..entries {
        let ptr_bytes = read_phys_slice(
            hhdm,
            xsdt_phys + size_of::<SdtHeader>() as u64 + (i as u64) * 8,
            8,
        );
        let table_phys = u64::from_le_bytes(ptr_bytes.try_into().unwrap());
        if let Some(thdr) = sdt_valid(hhdm, table_phys) {
            if &thdr.sig == want {
                return Some((table_phys, thdr.length));
            }
        }
    }
    None
}

// Search RSDT (32-bit entry array)
fn find_sdt_by_sig_rsdt(hhdm: u64, rsdt_phys: u64, want: &[u8; 4]) -> Option<(u64, u32)> {
    let rsdt = sdt_valid(hhdm, rsdt_phys)?;
    let entries = ((rsdt.length as usize) - size_of::<SdtHeader>()) / 4;
    for i in 0..entries {
        let ptr_bytes = read_phys_slice(
            hhdm,
            rsdt_phys + size_of::<SdtHeader>() as u64 + (i as u64) * 4,
            4,
        );
        let table_phys = u32::from_le_bytes(ptr_bytes.try_into().unwrap()) as u64;
        if let Some(thdr) = sdt_valid(hhdm, table_phys) {
            if &thdr.sig == want {
                return Some((table_phys, thdr.length));
            }
        }
    }
    None
}

// ───────────────────────── MADT discovery ─────────────────────────

pub fn discover(boot: &BootInfo) -> Option<Box<MadtInfo>> {
    if boot.rsdp_addr == 0 {
        kprintln!("[acpi] RSDP address is 0");
        return None;
    }

    // Read first 20 bytes for ACPI 1.0 view
    let r1_bytes = read_phys_slice(boot.hhdm_base, boot.rsdp_addr, size_of::<Rsdp10>());
    if &r1_bytes[0..8] != b"RSD PTR " || !checksum_ok(r1_bytes) {
        kprintln!("[acpi] Bad RSDP signature or v1 checksum");
        return None;
    }
    // Safe to cast to Rsdp10 now
    let rsdp10: &Rsdp10 = unsafe { &*(r1_bytes.as_ptr() as *const Rsdp10) };
    let rev = rsdp10.rev;

    // If revision >= 2, read extended RSDP and validate ext checksum
    let mut xsdt_addr: u64 = 0;
    if rev >= 2 {
        let r2_bytes = read_phys_slice(boot.hhdm_base, boot.rsdp_addr, size_of::<Rsdp20>());
        let rsdp20: &Rsdp20 = unsafe { &*(r2_bytes.as_ptr() as *const Rsdp20) };
        // ACPI 2.0+: ext checksum over 'length' bytes
        let total_len = rsdp20.length as usize;
        if total_len >= size_of::<Rsdp20>()
            && checksum_ok(read_phys_slice(boot.hhdm_base, boot.rsdp_addr, total_len))
        {
            xsdt_addr = rsdp20.xsdt_addr;
        } else {
            // ext checksum failed; still can fall back to RSDT
            xsdt_addr = 0;
        }
    }

    // Prefer XSDT if present and valid; else use RSDT
    let madt = if xsdt_addr != 0 {
        if let Some((madt_phys, madt_len)) =
            find_sdt_by_sig_xsdt(boot.hhdm_base, xsdt_addr, b"APIC")
        {
            Some((madt_phys, madt_len))
        } else {
            // XSDT path failed; try RSDT as fallback
            if rsdp10.rsdt_addr != 0 {
                find_sdt_by_sig_rsdt(boot.hhdm_base, rsdp10.rsdt_addr as u64, b"APIC")
            } else {
                None
            }
        }
    } else {
        if rsdp10.rsdt_addr != 0 {
            find_sdt_by_sig_rsdt(boot.hhdm_base, rsdp10.rsdt_addr as u64, b"APIC")
        } else {
            None
        }
    };

    let (madt_phys, madt_len) = match madt {
        Some(v) => v,
        None => {
            kprintln!("[acpi] MADT not found via XSDT/RSDT");
            return None;
        }
    };

    let madt_bytes = read_phys_slice(boot.hhdm_base, madt_phys, madt_len as usize);
    let mh: &MadtHeader = unsafe { &*(madt_bytes.as_ptr() as *const MadtHeader) };

    let mut lapic_phys = mh.lapic_mmio as u64;
    let mut cpus: Vec<Box<CpuEntry>> = Vec::new();
    let mut ioapics: Vec<Box<IoApic>> = Vec::new();

    let mut p = size_of::<MadtHeader>() as usize;
    while p + size_of::<MadtEntryHeader>() <= madt_len as usize {
        let hdr: &MadtEntryHeader =
            unsafe { &*(madt_bytes[p..].as_ptr() as *const MadtEntryHeader) };
        if hdr.len as usize == 0 {
            break;
        }

        match hdr.typ {
            PLAPIC if hdr.len as usize >= 8 => {
                let apic_id = madt_bytes[p + 3];
                let flags = u32::from_le_bytes(madt_bytes[p + 4..p + 8].try_into().unwrap());
                let enabled = (flags & 1) != 0;
                cpus.push(Box::new(CpuEntry {
                    apic_id: apic_id as u32,
                    enabled,
                    _is_x2apic: false,
                }));
            }
            IOAPIC if hdr.len as usize >= 12 => {
                let id = madt_bytes[p + 2];
                let base = u32::from_le_bytes(madt_bytes[p + 4..p + 8].try_into().unwrap()) as u64;
                let gsi = u32::from_le_bytes(madt_bytes[p + 8..p + 12].try_into().unwrap());
                ioapics.push(Box::new(IoApic {
                    _id: id,
                    _mmio_base_phys: base,
                    _gsi_base: gsi,
                }));
            }
            LAPIC_ADDR_OVERRIDE if hdr.len as usize >= 12 => {
                lapic_phys = u64::from_le_bytes(madt_bytes[p + 4..p + 12].try_into().unwrap());
            }
            PLX2APIC if hdr.len as usize >= 16 => {
                let apic_id = u32::from_le_bytes(madt_bytes[p + 4..p + 8].try_into().unwrap());
                let flags = u32::from_le_bytes(madt_bytes[p + 8..p + 12].try_into().unwrap());
                let enabled = (flags & 1) != 0;
                cpus.push(Box::new(CpuEntry {
                    apic_id,
                    enabled,
                    _is_x2apic: true,
                }));
            }
            _ => { /* ignore others for now */ }
        }

        p += hdr.len as usize;
    }

    let m: _ = MadtInfo {
        _lapic_phys: Box::new(lapic_phys),
        cpus: Box::new(cpus),
        _ioapics: Box::new(ioapics),
    };

    Some(Box::new(m))
}
