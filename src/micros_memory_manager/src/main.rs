#![no_std]
#![no_main]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::empty_loop)]

use core::{fmt::Write, panic::PanicInfo};
use micros_console_writer::WRITER;

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn main(_: *mut frame_allocation::amd64::Amd64FrameAllocator) -> ! {
    let _ = WRITER.lock().write_str("Hello, World!");
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = WRITER.lock().write_str("We're panicing!");
    loop {}
}
