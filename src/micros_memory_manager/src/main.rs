#![no_std]
#![no_main]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::empty_loop)]
#![allow(clippy::missing_safety_doc)]

use core::panic::PanicInfo;
use framebuffer::StandardRgbFramebuffer;
use multiboot2::{BootInformation, FramebufferTag};

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub unsafe extern "C" fn main(
    _: *mut frame_allocation::amd64::Amd64FrameAllocator,
    boot_info_ptr: *const u8,
) -> ! {
    if let Some(mut framebuffer) = get_framebuffer(boot_info_ptr) {
        framebuffer.paint_the_screen_white();
    }
    loop {}
}

unsafe fn get_framebuffer(boot_info_ptr: *const u8) -> Option<StandardRgbFramebuffer<'static>> {
    StandardRgbFramebuffer::from_tag(
        BootInformation::new(boot_info_ptr)
            .tags_of_type::<FramebufferTag>()
            .next()?,
    )
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
