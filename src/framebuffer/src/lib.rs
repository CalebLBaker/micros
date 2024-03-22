#![no_std]
#![feature(pointer_is_aligned)]

use core::{
    mem::size_of,
    slice
};
use multiboot2::{aligned_pointer_cast, FramebufferTag};

pub struct Framebuffer<'a> {
    framebuffer: &'a mut[u8],
    pitch: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u8,
    framebuffer_type: FramebufferType<'a>,
}

impl<'a> Framebuffer<'a> {
    fn new(tag: FramebufferTag<'a>) -> Option<Self> {
        Some(Self {
            framebuffer: tag.framebuffer,
            pitch: tag.pitch,
            width: tag.width,
            height: tag.height,
            bits_per_pixel: tag.bits_per_pixel,
            framebuffer_type: FramebufferType::new(tag.framebuffer_type, tag.color_data)?,
        })
    }
}

const INDEXED_COLOR_MODE: u8 = 0;
const RGB_COLOR_MODE: u8 = 1;
const EGA_TEXT_MODE: u8 = 2;

#[repr(C)]
struct Rgb {
    red: u8,
    green: u8,
    blue: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FramebufferPixelColorDescriptor {
    position: u8,
    size: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FramebufferPixelDescriptor {
    red: FramebufferPixelColorDescriptor,
    green: FramebufferPixelColorDescriptor,
    blue: FramebufferPixelColorDescriptor,
}

enum FramebufferType<'a> {
    IndexedColor(&'a [Rgb]),
    RgbColor(FramebufferPixelDescriptor),
    Ega,
}

impl<'a> FramebufferType<'a> {
    fn new(type_tag: u8, data: &'a [u8]) -> Option<Self> {
        match type_tag {
            INDEXED_COLOR_MODE => {
                let number_of_colors =
                    u32::from_le_bytes(data[..size_of::<u32>()].try_into().ok()?) as usize;
                if data.len() > size_of::<u32>() + number_of_colors * size_of::<Rgb>() {
                    None
                } else {
                    Some(FramebufferType::IndexedColor(unsafe {
                        slice::from_raw_parts(
                            aligned_pointer_cast::<Rgb>(
                                data.as_ptr().add(size_of::<u32>()),
                            )?,
                            number_of_colors,
                        )
                    }))
                }
            },
            RGB_COLOR_MODE =>
                if data.len() < size_of::<FramebufferPixelDescriptor>() {
                    None
                }
            else {
                Some(FramebufferType::RgbColor(unsafe {*aligned_pointer_cast::<FramebufferPixelDescriptor>(data.as_ptr())?}))
            },
            EGA_TEXT_MODE => Some(FramebufferType::Ega),
            _ => None,
        }
    }
}
