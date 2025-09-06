use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.divide_error.set_handler_fn(divide_by_zero);
        idt.page_fault.set_handler_fn(page_fault);
        idt.general_protection_fault.set_handler_fn(gpf);

        // Nightly-2025-08 workaround: install DF via unsafe cast
        install_double_fault(&mut idt);

        idt
    })
    .load();
}

extern "x86-interrupt" fn divide_by_zero(stack: InterruptStackFrame) {
    crate::println!("#DE divide-by-zero\n{:#?}", stack);
}

extern "x86-interrupt" fn gpf(stack: InterruptStackFrame, _error: u64) {
    crate::println!("#GP fault\n{:#?}", stack);
}

extern "x86-interrupt" fn page_fault(
    stack: InterruptStackFrame,
    error: x86_64::structures::idt::PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;
    crate::println!("#PF addr={:?} err={:?}\n{:#?}", Cr2::read(), error, stack);
}

// Omit explicit return type to appease nightly-2025-08-15.
extern "x86-interrupt" fn double_fault_no_ret(stack: InterruptStackFrame, _error: u64) {
    crate::println!("#DF DOUBLE FAULT!\n{:#?}", stack);
    loop {
        x86_64::instructions::hlt();
    }
}

// Cast to the signature required by x86_64 0.15.x: extern "x86-interrupt" fn(_, _) -> !
fn install_double_fault(idt: &mut InterruptDescriptorTable) {
    type DFPtr = extern "x86-interrupt" fn(InterruptStackFrame, u64) -> !;

    let ptr = double_fault_no_ret as *const ();
    let df: DFPtr = unsafe { core::mem::transmute::<*const (), DFPtr>(ptr) };

    idt.double_fault.set_handler_fn(df);
}
