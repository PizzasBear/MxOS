//! This module contains everything related to the IDT.
//!
//! To initialize the IDT call `crate::idt::init_idt()`.
//!

use crate::gdt::*;
use crate::serial::Indent;
use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX);
        }
        unsafe {
            idt.page_fault
                .set_handler_fn(page_fault_handler)
                .set_stack_index(PAGE_FAULT_IST_INDEX);
        }
        idt
    };
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    log::info!("BREAKPOINT_INTERRUPT: {:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, code: u64) -> ! {
    unsafe {
        crate::serial::SERIAL_LOGGER.force_unlock();
        crate::sprintln!();
    }
    log::error!("DOUBLE_FAULT(code={}): {:#?}", code, stack_frame);

    loop {}
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    unsafe {
        crate::serial::SERIAL_LOGGER.force_unlock();
        crate::sprintln!();
    }
    log::error!(
        "PAGE_FAULT(\n    error_code: {:?},\n    accessed_address: {:?},\n    stack_frame: {:#?},\n)",
        code,
        Cr2::read(),
        Indent::new(1, &stack_frame),
    );

    loop {}
}

/// Initializes the IDT
pub fn init_idt() {
    IDT.load();
}
