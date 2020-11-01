#![no_std]
#![feature(abi_x86_interrupt)]

use display_daemon;
use core::fmt::Write;

mod arch;

use arch::x86_64 as proc;

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32) -> ! {
    proc::init();
    let _ = display_daemon::WRITER.lock().write_str("Interrupt Handlers Enabled\n");
    write!(display_daemon::WRITER.lock(), "address: {}\n", multiboot_info_ptr);
    let boot_info = unsafe { multiboot2::load(multiboot_info_ptr as usize) };
    boot_info.memory_map_tag();
    proc::halt()
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = write!(display_daemon::WRITER.lock(), "{}", info);
    proc::halt()
}

