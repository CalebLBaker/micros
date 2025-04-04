#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use address::VirtualAddress;

#[cfg(target_pointer_width = "64")]
pub mod elf64;

pub trait ExecutableHeader {
    fn is_valid(&self, file_size: usize) -> bool;

    fn num_segments(&self) -> usize;

    fn segment_header_table_offset(&self) -> usize;

    fn entry(&self) -> VirtualAddress;
}

pub trait SegmentHeader {
    fn segment_type(&self) -> SegmentType;
    fn offset(&self) -> usize;
    fn address(&self) -> VirtualAddress;
    fn file_size(&self) -> usize;
    fn memory_size(&self) -> usize;
    fn flags(&self) -> SegmentFlags;
}

#[derive(Clone, Copy)]
pub struct SegmentFlags(u32);

impl SegmentFlags {
    #[must_use]
    pub fn writable(self) -> bool {
        (self.0 & ELF_WRITABLE_SEGMENT) != 0
    }

    #[must_use]
    pub fn executable(self) -> bool {
        (self.0 & ELF_EXECUTABLE_SEGMENT) != 0
    }
}

#[derive(PartialEq)]
pub enum SegmentType {
    Loadable,
    Other,
}

impl From<u32> for SegmentType {
    fn from(value: u32) -> Self {
        if value == 1 {
            Self::Loadable
        } else {
            Self::Other
        }
    }
}

const ELF_WRITABLE_SEGMENT: u32 = 2;
const ELF_EXECUTABLE_SEGMENT: u32 = 1;
