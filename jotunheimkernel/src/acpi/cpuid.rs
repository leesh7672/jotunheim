use crate::arch::x86_64::apic::{self, lapic_id};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuId {
    apic: Option<u32>,
}

impl CpuId {
    pub fn me() -> Self {
        Self {
            apic: Some(lapic_id()),
        }
    }
    pub fn dummy() -> Self {
        Self { apic: None }
    }
}
