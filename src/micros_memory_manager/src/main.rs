#![no_std]
#![no_main]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::empty_loop)]

use address::AddressMapper;
use core::panic::PanicInfo;
use framebuffer::StandardRgbFramebuffer;
use multiboot2::{BootInformation, BootInformationHeader, FramebufferTag};

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
    proc: *mut architecture::amd64::Amd64,
    boot_info_ptr: *const BootInformationHeader,
) -> ! {
    if let Some(mut framebuffer) = unsafe { get_framebuffer(&*proc, boot_info_ptr) } {
        framebuffer.paint_the_screen_white();
    }
    loop {}
}

unsafe fn get_framebuffer<AddrMap: AddressMapper>(
    proc: &AddrMap,
    boot_info_ptr: *const BootInformationHeader,
) -> Option<StandardRgbFramebuffer<'static>> {
    unsafe {
        StandardRgbFramebuffer::from_tag(
            proc,
            &BootInformation::new(boot_info_ptr)
                .tags_of_type::<FramebufferTag>()
                .next()?,
        )
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
