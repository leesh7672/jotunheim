use core::sync::atomic::{AtomicU64, Ordering};
use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

static LAST_VEC: AtomicU64 = AtomicU64::new(0xFF);

use crate::arch::x86_64::apic; // exposes TIMER_VECTOR, SPURIOUS_VECTOR and EOI helper

static IDT: Once<InterruptDescriptorTable> = Once::new();
static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        for vec in 0x21u8..=0xFEu8 {
            idt[vec].set_handler_fn(dummy_isr);
        }
        idt.divide_error.set_handler_fn(divide_by_zero);
        idt.general_protection_fault.set_handler_fn(gpf);
        idt.page_fault.set_handler_fn(page_fault);

        idt.non_maskable_interrupt.set_handler_fn(nmi);

        install_mce(&mut idt);
        install_double_fault(&mut idt);

        idt[apic::TIMER_VECTOR].set_handler_fn(timer_apic); // EOI (and re-arm if deadline)
        idt[apic::SPURIOUS_VECTOR].set_handler_fn(spurious_apic); // NO EOI for spurious

        idt
    })
    .load();
}

// -------- IRQ handlers (no printing here) --------

extern "x86-interrupt" fn timer_apic(_stack: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        apic::timer_isr_eoi_and_rearm_deadline();
    }
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// -------- Exception handlers (safe to log + halt) --------

extern "x86-interrupt" fn divide_by_zero(stack: InterruptStackFrame) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn gpf(stack: InterruptStackFrame, _error: u64) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn page_fault(stack: InterruptStackFrame, error: PageFaultErrorCode) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn double_fault_no_ret(stack: InterruptStackFrame, _error: u64) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}
extern "x86-interrupt" fn nmi(_stack: InterruptStackFrame) {
    // NMI delivery doesn’t honor IF; keep it minimal.
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn dummy_isr(_stack: InterruptStackFrame) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn spurious_apic(_stack: InterruptStackFrame) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}

extern "x86-interrupt" fn mce(_stack: InterruptStackFrame) {
    unsafe {
        crate::arch::x86_64::apic::eoi();
    }
}
// Cast body returning () to required DF signature -> !
fn install_double_fault(idt: &mut InterruptDescriptorTable) {
    type DFPtr = extern "x86-interrupt" fn(InterruptStackFrame, u64) -> !;
    let ptr = double_fault_no_ret as *const ();
    let df: DFPtr = unsafe { core::mem::transmute::<*const (), DFPtr>(ptr) };
    idt.double_fault.set_handler_fn(df);
}
fn install_mce(idt: &mut InterruptDescriptorTable) {
    type MCPtr = extern "x86-interrupt" fn(InterruptStackFrame) -> !;

    let ptr = mce as *const ();
    let mc: MCPtr = unsafe { core::mem::transmute::<*const (), MCPtr>(ptr) };
    idt.machine_check.set_handler_fn(mc);
}
