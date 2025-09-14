// src/acpi/mod.rs
pub mod madt;

#[derive(Debug, Copy, Clone)]
pub struct CpuEntry {
    pub apic_id: u32,    // LAPIC ID (8-bit for xAPIC, 32-bit for x2APIC)
    pub enabled: bool,   // ACPI “enabled” flag
    pub is_x2apic: bool, // true if came from x2APIC (type 9) entry
}

#[derive(Debug, Copy, Clone)]
pub struct IoApic {
    pub id: u8,
    pub mmio_base_phys: u64,
    pub gsi_base: u32,
}

#[derive(Debug, Copy, Clone)]
pub struct MadtInfo {
    pub lapic_phys: u64, // Local APIC MMIO (may be overridden)
    pub cpus: &'static [CpuEntry],
    pub ioapics: &'static [IoApic],
}
