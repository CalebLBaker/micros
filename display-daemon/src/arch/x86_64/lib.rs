#![no_std]

use core::fmt;
use lazy_static::lazy_static;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(u8);

impl ColorCode {
    pub fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
struct Buffer {
    chars: [volatile::Volatile<ScreenChar>; BUFFER_WIDTH * BUFFER_HEIGHT],
}

pub struct Writer {
    position: usize,
    color_code: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.position += BUFFER_WIDTH - self.position % BUFFER_WIDTH,
            _ => {
                self.buffer.chars[self.position].write(ScreenChar {
                    ascii_character: byte,
                    color_code: self.color_code
                });
                self.position += 1;
            }
        }
    }
    pub fn new(pos: usize, color: ColorCode) -> Writer {
        Writer {
            position: pos,
            color_code: color,
            buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
        }
    }
    pub fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() { self.write_byte(byte); }
        Ok(())
    }
    pub fn default() -> Writer { Writer::new(0, ColorCode::new(Color::White, Color::Black)) }
}

impl fmt::Write for Writer {
  fn write_str(&mut self, s: &str) -> fmt::Result { self.write_str(s) }
}

lazy_static! {
    pub static ref WRITER: spin::Mutex<Writer> = spin::Mutex::new(Writer::default());
}

