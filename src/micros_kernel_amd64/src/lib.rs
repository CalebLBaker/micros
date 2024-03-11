#![no_std]
#![feature(abi_x86_interrupt)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::struct_field_names)]

mod apic;
mod elf;
mod init;

use apic::end_interrupt;
use core::panic::PanicInfo;
use init::initialize_operating_system;
use x86_64::{
    instructions::hlt,
    structures::{
        idt::{InterruptStackFrame, PageFaultErrorCode},
        paging::PageTable,
    },
};

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    unsafe {
        initialize_operating_system(multiboot_info_ptr, cpu_info);
    }
    halt()
}

#[no_mangle]
pub extern "C" fn halt() -> ! {
    loop {
        hlt();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt()
}

extern "C" {
    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
    static mut p1_table_for_stack: PageTable;
    fn launch_memory_manager(root_page_table_address: usize, entry_point: usize) -> !;
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn double_fault_handler(_stack_frame: InterruptStackFrame, _: u64) -> ! {
    halt();
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    halt();
}

extern "x86-interrupt" fn spurious_interrupt_handler(_: InterruptStackFrame) {
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn error_interrupt_handler(_: InterruptStackFrame) {
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_: InterruptStackFrame) {
    unsafe {
        end_interrupt();
    }
}
