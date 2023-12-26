#![no_std]
#![no_main]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::empty_loop)]

use core::{fmt::Write, panic::PanicInfo};
use micros_console_writer::WRITER;

#[no_mangle]
pub extern "C" fn main() -> ! {
    let _ = WRITER.lock().write_str("Hello, World!");
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
