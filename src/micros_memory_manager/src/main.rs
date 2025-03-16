#![no_std]
#![no_main]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::empty_loop)]

use core::panic::PanicInfo;
use framebuffer::StandardRgbFramebuffer;
use multiboot2::{BootInformation, FramebufferTag};

/// # Safety
///
/// This is the entry function for the memory manager. Incorrect assumtpions here could mess up
/// **ALL** processes both kernel and userspace.
///
/// ## Assumptions:
///
/// * `boot_info_ptr` must point to a valid multiboot2 boot information structure
/// 
/// * `frame_allocator` must be in a valid state
///
/// * The current process' virtual address space must be identity mapped to the entirety of the
///   machine's physical address space
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(
    _: *mut frame_allocation::amd64::Amd64FrameAllocator,
    boot_info_ptr: *const u8,
) -> ! {
    if let Some(mut framebuffer) = unsafe { get_framebuffer(boot_info_ptr) } {
        framebuffer.paint_the_screen_white();
    }
    loop {}
}

unsafe fn get_framebuffer(boot_info_ptr: *const u8) -> Option<StandardRgbFramebuffer<'static>> {
    unsafe {
        StandardRgbFramebuffer::from_tag(
            BootInformation::new(boot_info_ptr)
                .tags_of_type::<FramebufferTag>()
                .next()?,
        )
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
