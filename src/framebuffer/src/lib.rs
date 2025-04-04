#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use address::AddressMapper;
use core::{mem::size_of, slice};
use multiboot2::{FramebufferTag, aligned_pointer_cast};

pub enum Framebuffer<'a> {
    IndexedColor(IndexedColorFramebuffer<'a>),
    RgbColor(RgbColorFramebuffer<'a>),
    Ega,
}

impl<'a> Framebuffer<'a> {
    /// # Safety
    ///
    /// To avoid constructing an invalid framebuffer, the information contained in `tag` must be completely accurate.
    pub unsafe fn new<AddrMap: AddressMapper>(
        address_mapper: &AddrMap,
        tag: &FramebufferTag<'a>,
    ) -> Option<Self> {
        let core = FramebufferCore {
            framebuffer: unsafe {
                slice::from_raw_parts_mut(
                    address_mapper.physical_address_to_pointer(tag.framebuffer),
                    tag.pitch as usize * tag.height as usize,
                )
            },
            pitch: tag.pitch,
            width: tag.width,
            height: tag.height,
            bits_per_pixel: tag.bits_per_pixel,
        };
        match tag.framebuffer_type {
            INDEXED_COLOR_MODE => {
                let (number, palette) = tag.color_data.split_first_chunk::<4>()?;
                let number_of_colors = u32::from_le_bytes(*number) as usize;
                if tag.color_data.len() > size_of::<u32>() + number_of_colors * size_of::<Rgb>() {
                    None
                } else {
                    Some(Self::IndexedColor(IndexedColorFramebuffer {
                        _core: core,
                        _color_palette: unsafe {
                            slice::from_raw_parts(
                                aligned_pointer_cast::<Rgb>(palette.as_ptr())?,
                                number_of_colors,
                            )
                        },
                    }))
                }
            }
            RGB_COLOR_MODE => {
                if tag.color_data.len() < size_of::<FramebufferPixelDescriptor>() {
                    None
                } else {
                    Some(Self::RgbColor(RgbColorFramebuffer {
                        core,
                        pixel_descriptor: unsafe {
                            *aligned_pointer_cast::<FramebufferPixelDescriptor>(
                                tag.color_data.as_ptr(),
                            )?
                        },
                    }))
                }
            }
            EGA_TEXT_MODE => Some(Self::Ega),
            _ => None,
        }
    }
}

pub struct StandardRgbFramebuffer<'a> {
    framebuffer: &'a mut [u8],
    pitch: u32,
    _width: u32,
    _height: u32,
    bytes_per_pixel: u8,
    _pixel_descriptor: FramebufferPixelDescriptor,
}

impl<'a> StandardRgbFramebuffer<'a> {
    #[must_use]
    pub fn new(framebuffer: Framebuffer<'a>) -> Option<Self> {
        match framebuffer {
            Framebuffer::RgbColor(buffer) => {
                if buffer.core.bits_per_pixel <= 0x40
                    && buffer.core.bits_per_pixel.trailing_zeros() >= 3
                {
                    Some(Self {
                        framebuffer: buffer.core.framebuffer,
                        pitch: buffer.core.pitch,
                        _width: buffer.core.width,
                        _height: buffer.core.height,
                        bytes_per_pixel: buffer.core.bits_per_pixel >> 3,
                        _pixel_descriptor: buffer.pixel_descriptor,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// # Safety
    ///
    /// To avoid constructing an invalid framebuffer, the information contained in `tag` must be completely accurate.
    pub unsafe fn from_tag<AddrMap: AddressMapper>(
        address_mapper: &AddrMap,
        tag: &FramebufferTag<'a>,
    ) -> Option<Self> {
        Self::new(unsafe { Framebuffer::new(address_mapper, tag) }?)
    }

    pub fn draw_pixel(&mut self, row: u32, column: u32, color: [u8; 8]) {
        let bpp = self.bytes_per_pixel as usize;
        let location = row as usize * self.pitch as usize + column as usize * bpp;
        if location + bpp <= self.framebuffer.len() {
            self.framebuffer[location..location + bpp].copy_from_slice(&color[..bpp]);
        }
    }

    pub fn paint_the_screen_white(&mut self) {
        self.framebuffer.fill(0xff);
    }

    pub const WHITE: [u8; 8] = [0xff; 8];
}

pub struct IndexedColorFramebuffer<'a> {
    _core: FramebufferCore<'a>,
    _color_palette: &'a [Rgb],
}

pub struct RgbColorFramebuffer<'a> {
    core: FramebufferCore<'a>,
    pixel_descriptor: FramebufferPixelDescriptor,
}

#[repr(C)]
pub struct Rgb {
    red: u8,
    green: u8,
    blue: u8,
}

const INDEXED_COLOR_MODE: u8 = 0;
const RGB_COLOR_MODE: u8 = 1;
const EGA_TEXT_MODE: u8 = 2;

struct FramebufferCore<'a> {
    framebuffer: &'a mut [u8],
    pitch: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u8,
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
