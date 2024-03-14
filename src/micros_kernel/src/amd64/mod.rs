#![allow(clippy::struct_field_names)]

mod apic;
mod elf;
mod init;

use apic::end_interrupt;
use core::panic::PanicInfo;
use frame_allocation::amd64::Amd64FrameAllocator;
pub use init::initialize_operating_system;
use x86_64::{
    instructions::hlt,
    structures::{
        idt::{InterruptStackFrame, PageFaultErrorCode},
        paging::PageTable,
    },
};

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt()
}

pub fn halt() -> ! {
    loop {
        hlt();
    }
}

extern "C" {
    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
    static mut p1_table_for_stack: PageTable;
    fn launch_memory_manager(
        allocator: *mut Amd64FrameAllocator,
        root_page_table_address: usize,
        entry_point: usize,
    ) -> !;
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
