#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

#[cfg(target_arch = "x86_64")]
pub mod amd64;

use core::ops::Range;

/// Like `Option`, but with a stable ABI so that it can be used in foreign function interfaces.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum FfiOption<T> {
    None,
    Some(T),
}

impl<T> FfiOption<T> {
    fn as_mut(&mut self) -> Option<&mut T> {
        if let Self::Some(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

pub trait MemoryFrame: Sized {
    /// Calculates the address of the first page that starts at or after `start_address`.
    #[must_use]
    fn first_full_page(start_address: *mut u8) -> *mut Self {
        start_address
            .wrapping_add(start_address.align_offset(size_of::<Self>()))
            .cast::<Self>()
    }

    /// Calculates the end address of the last page that ends at or before `end_address`.
    #[must_use]
    fn end_of_last_full_page(end_address: *mut u8) -> *mut Self {
        if (end_address as usize) % size_of::<Self>() == 0 {
            end_address.cast::<Self>()
        } else {
            unsafe { Self::first_full_page(end_address).sub(1) }
        }
    }
}

/// A memory allocator that allocates memory in fixed-sized frames
#[repr(C)]
pub struct FrameAllocator<Frame: MemoryFrame> {
    next: FfiOption<*mut FrameAllocator<Frame>>,
}

impl<Frame: MemoryFrame> FrameAllocator<Frame> {
    const FRAME_SIZE: usize = size_of::<Frame>();

    /**
     * Adds available frames to the allocator
     *
     * # Safety
     *
     * `memory_area` must represent a range of valid and available memory and must be
     * `FRAME_SIZE`-aligned. If there are addresses in the range that don't represent valid memory
     * or represent memory that is already in use, then undefined behavior may occur.
     */
    pub unsafe fn add_frames(&mut self, memory_area: Range<*mut Frame>) {
        let mut frame = memory_area.start;
        while frame < memory_area.end {
            unsafe {
                self.add_frame(frame);
                frame = frame.byte_add(Self::FRAME_SIZE);
            }
        }
    }

    /**
     * Retrieves a frame of available memory from the allocator
     *
     * # Safety
     *
     * This function should be safe so long as `self` is in a valid state, but may trigger
     * undefined behavior if invalid or already-in-use memory regions have been added to the
     * allocator previously.
     */
    unsafe fn get_frame(&mut self) -> Option<*mut Frame> {
        if let FfiOption::Some(ret) = self.next {
            self.next = unsafe { (*ret).next };
            Some(ret.cast::<Frame>())
        } else {
            None
        }
    }

    /**
     * Adds an available frame to the allocator
     *
     * # Safety
     *
     * `frame_address` must represent the start of a frame of valid and available memory. If the
     * memory frame does not exist or is already in use then undefined behavior may occur.
     */
    pub unsafe fn add_frame(&mut self, frame: *mut Frame) {
        let frame_ptr = frame.cast::<Self>();
        unsafe {
            (*frame_ptr).next = self.next;
            self.next = FfiOption::Some(&mut *frame_ptr);
        }
    }

    /**
     * Adds available frames from a memory region to this allocator and then takes any portions of
     * the memory region that could not be used due to alignment issues and attempts to add them to
     * another allocator with a smaller frame size.
     *
     * # Safety
     *
     * `memory_region` must represent a range of valid and available memory. If there are addresses
     * in the range that don't represent valid memory or represent memory that is already in use,
     * then undefined behavior may occur.
     */
    pub unsafe fn add_aligned_frames_with_scrap_allocator<SmallerFrame: MemoryFrame>(
        &mut self,
        smaller_allocator: &mut FrameAllocator<SmallerFrame>,
        memory_region: Range<*mut u8>,
    ) {
        let first_page = Frame::first_full_page(memory_region.start);
        let end_of_last_page = Frame::end_of_last_full_page(memory_region.end);
        unsafe {
            if end_of_last_page > first_page {
                smaller_allocator.add_aligned_frames(memory_region.start..first_page.cast::<u8>());
                self.add_frames(first_page..end_of_last_page);
                smaller_allocator
                    .add_aligned_frames(end_of_last_page.cast::<u8>()..memory_region.end);
            } else {
                smaller_allocator.add_aligned_frames(memory_region);
            }
        }
    }

    unsafe fn add_aligned_frames(&mut self, memory_region: Range<*mut u8>) {
        unsafe {
            self.add_frames(
                Frame::first_full_page(memory_region.start)
                    ..Frame::end_of_last_full_page(memory_region.end),
            );
        }
    }

    /// Constructs a new empty `FrameAllocator`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next: FfiOption::None,
        }
    }
}

impl<Frame: MemoryFrame> Default for FrameAllocator<Frame> {
    fn default() -> Self {
        Self::new()
    }
}
