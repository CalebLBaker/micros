#![no_std]
#![feature(try_trait_v2)]
#![allow(clippy::missing_safety_doc)]

#[cfg(target_arch = "x86_64")]
pub mod amd64;

use core::{
    convert::Infallible,
    ops::{ControlFlow, FromResidual, Range, Try},
};

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

#[repr(C)]
pub struct FrameAllocator<const FRAME_SIZE: usize> {
    next: FfiOption<*mut FrameAllocator<FRAME_SIZE>>,
}

impl<const MEMORY_FRAME_SIZE: usize> FrameAllocator<MEMORY_FRAME_SIZE> {
    const FRAME_SIZE: usize = MEMORY_FRAME_SIZE;

    pub unsafe fn add_frames(&mut self, memory_area: Range<usize>) {
        for frame in memory_area.step_by(Self::FRAME_SIZE) {
            self.add_frame(frame);
        }
    }

    unsafe fn get_frame(&mut self) -> Option<usize> {
        let ret = self.next?;
        self.next = (*ret).next;
        Some(ret as usize)
    }

    pub unsafe fn add_frame(&mut self, frame_address: usize) {
        let frame_ptr = frame_address as *mut Self;
        (*frame_ptr).next = self.next;
        self.next = FfiOption::Some(&mut *frame_ptr);
    }

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

#[must_use]
pub fn end_of_last_full_page(end_address: usize, page_size: usize) -> usize {
    end_address - end_address % page_size
}

#[must_use]
pub fn first_full_page_address(start_address: usize, page_size: usize) -> usize {
    let page_offset = start_address % page_size;
    if page_offset == 0 {
        start_address
    } else {
        start_address + page_size - page_offset
    }
}
