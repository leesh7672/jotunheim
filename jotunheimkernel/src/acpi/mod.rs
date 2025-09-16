use alloc::vec::Vec;

// src/acpi/mod.rs
pub mod madt;

#[derive(Debug, Copy, Clone)]
pub struct CpuEntry {
    pub apic_id: u32,    // LAPIC ID (8-bit for xAPIC, 32-bit for x2APIC)
    pub enabled: bool,   // ACPI “enabled” flag
    pub _is_x2apic: bool, // true if came from x2APIC (type 9) entry
}

#[derive(Debug, Copy, Clone)]
pub struct IoApic {
    pub _id: u8,
    pub _mmio_base_phys: u64,
    pub _gsi_base: u32,
}

#[derive(Debug, Clone)]
pub struct MadtInfo {
    pub _lapic_phys: u64, // Local APIC MMIO (may be overridden)
    pub cpus: Vec<CpuEntry>,
    pub _ioapics: Vec<IoApic>,
}
