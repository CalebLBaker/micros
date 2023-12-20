#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![feature(abi_x86_interrupt)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::struct_field_names)]

mod apic;
mod elf;
mod init;

use apic::end_interrupt;
use core::{fmt::Write, panic::PanicInfo};
use init::{initialize_operating_system, OsError};
use micros_console_writer::WRITER;
use micros_kernel_common::Error;
use multiboot2::MbiLoadError;
use x86_64::{
    instructions::hlt,
    structures::{
        idt::{InterruptStackFrame, PageFaultErrorCode},
        paging::PageTable,
    },
};

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    match unsafe { initialize_operating_system(multiboot_info_ptr, cpu_info) } {
        Ok(_) => {
            let _ = WRITER
                .lock()
                .write_str("Function that wasn't supposed to return returned . . . \n");
        }
        Err(err) => {
            let _ = WRITER.lock().write_str(match err {
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::IllegalAddress)) => {
                    "Illegal multiboot info address"
                }
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::IllegalTotalSize(_))) => {
                    "Illegal multiboot info size"
                }
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::NoEndTag)) => {
                    "No multiboot info end tag"
                }
                OsError::Generic(Error::NoMemoryManager) => {
                    "Memory manager not loaded by boot loader"
                }
                OsError::Generic(Error::NoMemoryMap) => {
                    "No memory map tag in multiboot information"
                }
                OsError::Generic(Error::InvalidMemoryManagerModule) => {
                    "Could not load memory manager"
                }
                OsError::Generic(Error::AssertionError) => "An unexpected error occurred",
                OsError::Generic(Error::FailedToSetupMemoryManagerAddressSpace) => {
                    "Failed to setup memory manager address space"
                }
                OsError::Apic(err) => err,
            });
        }
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
fn panic(info: &PanicInfo) -> ! {
    let _ = write!(WRITER.lock(), "{info}");
    halt()
}

extern "C" {
    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
    static mut p1_table_for_stack: PageTable;
    fn launch_memory_manager(root_page_table_address: usize, entry_point: usize) -> !;
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("breakpoint\n");
}

extern "x86-interrupt" fn double_fault_handler(_stack_frame: InterruptStackFrame, _: u64) -> ! {
    let _ = WRITER.lock().write_str("double fault\n");
    halt();
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    let _ = WRITER.lock().write_str("page fault\n");
    halt();
}

extern "x86-interrupt" fn spurious_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("spurious\n");
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn error_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("error\n");
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("timer\n");
    unsafe {
        end_interrupt();
    }
}
