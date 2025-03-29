use crate::{FfiOption, FrameAllocator, MemoryFrame};

pub const FOUR_KILOBYTES: usize = 0x1000;

/// Memory frame allocator for AMD64 processors
#[repr(C)]
pub struct Amd64FrameAllocator {
    /// The allocator for 4 KB standard pages
    pub four_kilobyte_pages: FrameAllocator<FourKbFrame>,
    /// The allocator for 2 MB big pages
    pub two_megabyte_pages: FrameAllocator<TwoMbFrame>,
    /// The allocator for 1 GB huge pages
    pub gigabyte_pages: FfiOption<FrameAllocator<GbFrame>>,
}

impl Amd64FrameAllocator {
    /**
     * Retrieves a 4 kilobyte frame of available memory from the allocator
     *
     * # Safety
     *
     * This function should be safe so long as `self` is in a valid state, but may trigger
     * undefined behavior if invalid or already-in-use memory regions have been added to the
     * allocator previously.
     */
    pub unsafe fn get_4k_frame(&mut self) -> Option<*mut FourKbFrame> {
        unsafe {
            if let Some(frame) = self.four_kilobyte_pages.get_frame() {
                Some(frame)
            } else if let Some(big_frame) = self.get_2mb_frame() {
                let frame = big_frame.cast::<FourKbFrame>();
                self.four_kilobyte_pages
                    .add_frames((frame.add(1))..(big_frame.add(1).cast::<FourKbFrame>()));
                Some(frame)
            } else {
                None
            }
        }
    }

    /**
     * Retrieves a 2 megabyte frame of available memory from the allocator
     *
     * # Safety
     *
     * This function should be safe so long as `self` is in a valid state, but may trigger
     * undefined behavior if invalid or already-in-use memory regions have been added to the
     * allocator previously.
     */
    pub unsafe fn get_2mb_frame(&mut self) -> Option<*mut TwoMbFrame> {
        unsafe {
            if let Some(frame) = self.two_megabyte_pages.get_frame() {
                Some(frame)
            } else if let Some(big_frame) = self.gigabyte_pages.as_mut()?.get_frame() {
                let frame = big_frame.cast::<TwoMbFrame>();
                self.two_megabyte_pages
                    .add_frames((frame.add(1))..(big_frame.add(1).cast::<TwoMbFrame>()));
                Some(frame)
            } else {
                None
            }
        }
    }
}

#[repr(C, align(0x1000))]
pub struct FourKbFrame {
    _data: [u8; FOUR_KILOBYTES],
}

impl MemoryFrame for FourKbFrame {}

#[repr(C, align(0x20_0000))]
pub struct TwoMbFrame {
    _data: [u8; TWO_MEGABYTES],
}

impl MemoryFrame for TwoMbFrame {}

#[repr(C, align(0x2000_0000))]
pub struct GbFrame {
    _data: [u8; GIGABYTE],
}

impl MemoryFrame for GbFrame {}

const TWO_MEGABYTES: usize = 0x20_0000;
const GIGABYTE: usize = 0x4000_0000;
