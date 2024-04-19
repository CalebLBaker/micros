#![no_std]
#![feature(try_trait_v2)]

#[cfg(target_arch = "x86_64")]
pub mod amd64;

use core::{
    convert::Infallible,
    ops::{ControlFlow, FromResidual, Range, Try},
};

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

impl<T> Try for FfiOption<T> {
    type Output = T;
    type Residual = Option<Infallible>;
    fn from_output(output: Self::Output) -> Self {
        Self::Some(output)
    }
    fn branch(self) -> ControlFlow<Self::Residual, Self::Output> {
        if let Self::Some(output) = self {
            ControlFlow::Continue(output)
        } else {
            ControlFlow::Break(None)
        }
    }
}

impl<T> FromResidual<Option<Infallible>> for FfiOption<T> {
    fn from_residual(_: Option<Infallible>) -> Self {
        Self::None
    }
}

/// A memory allocator that allocates memory in fixed-sized frames
#[repr(C)]
pub struct FrameAllocator<const FRAME_SIZE: usize> {
    next: FfiOption<*mut FrameAllocator<FRAME_SIZE>>,
}

impl<const MEMORY_FRAME_SIZE: usize> FrameAllocator<MEMORY_FRAME_SIZE> {
    const FRAME_SIZE: usize = MEMORY_FRAME_SIZE;

    /**
     * Adds available frames to the allocator
     *
     * # Safety
     *
     * `memory_area` must represent a range of valid and available memory and must be
     * `FRAME_SIZE`-aligned. If there are addresses in the range that don't represent valid memory
     * or represent memory that is already in use, then undefined behavior may occur.
     */
    pub unsafe fn add_frames(&mut self, memory_area: Range<usize>) {
        for frame in memory_area.step_by(Self::FRAME_SIZE) {
            self.add_frame(frame);
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
    unsafe fn get_frame(&mut self) -> Option<usize> {
        let ret = self.next?;
        self.next = (*ret).next;
        Some(ret as usize)
    }

    /**
     * Adds an available frame to the allocator
     *
     * # Safety
     *
     * `frame_address` must represent the start of a frame of valid and available memory. If the
     * memory frame does not exist or is already in use then undefined behavior may occur.
     */
    pub unsafe fn add_frame(&mut self, frame_address: usize) {
        let frame_ptr = frame_address as *mut Self;
        (*frame_ptr).next = self.next;
        self.next = FfiOption::Some(&mut *frame_ptr);
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
    pub unsafe fn add_aligned_frames_with_scrap_allocator<const SMALLER_FRAME_SIZE: usize>(
        &mut self,
        smaller_allocator: &mut FrameAllocator<SMALLER_FRAME_SIZE>,
        memory_region: Range<usize>,
    ) {
        let first_page = first_full_page_address(memory_region.start, Self::FRAME_SIZE);
        let end_of_last_page = end_of_last_full_page(memory_region.end, Self::FRAME_SIZE);
        if end_of_last_page > first_page {
            smaller_allocator.add_aligned_frames(memory_region.start..first_page);
            self.add_frames(first_page..end_of_last_page);
            smaller_allocator.add_aligned_frames(end_of_last_page..memory_region.end);
        } else {
            smaller_allocator.add_aligned_frames(memory_region);
        }
    }

    unsafe fn add_aligned_frames(&mut self, memory_region: Range<usize>) {
        self.add_frames(
            first_full_page_address(memory_region.start, Self::FRAME_SIZE)
                ..end_of_last_full_page(memory_region.end, Self::FRAME_SIZE),
        );
    }

    /// Constructs a new empty `FrameAllocator`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next: FfiOption::None,
        }
    }
}

impl<const FRAME_SIZE: usize> Default for FrameAllocator<FRAME_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculates the end address of the last page that ends at or before `end_address`.
#[must_use]
pub fn end_of_last_full_page(end_address: usize, page_size: usize) -> usize {
    end_address - end_address % page_size
}

/// Calculates the address of the first page that starts at or after `start_address`.
#[must_use]
pub fn first_full_page_address(start_address: usize, page_size: usize) -> usize {
    let page_offset = start_address % page_size;
    if page_offset == 0 {
        start_address
    } else {
        start_address + page_size - page_offset
    }
}
