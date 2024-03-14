use crate::{FfiOption, FrameAllocator};

pub const FOUR_KILOBYTES: usize = 0x1000;
pub const GIGABYTE: usize = 0x4000_0000;

#[repr(C)]
pub struct Amd64FrameAllocator {
    pub four_kilobyte_pages: FrameAllocator<FOUR_KILOBYTES>,
    pub two_megabyte_pages: FrameAllocator<TWO_MEGABYTES>,
    pub gigabyte_pages: FfiOption<FrameAllocator<GIGABYTE>>,
}

impl Amd64FrameAllocator {
    pub unsafe fn get_4k_frame(&mut self) -> Option<usize> {
        if let Some(frame) = self.four_kilobyte_pages.get_frame() {
            Some(frame)
        } else if let Some(frame) = self.get_2mb_frame() {
            self.four_kilobyte_pages
                .add_frames((frame + FOUR_KILOBYTES)..(frame + TWO_MEGABYTES));
            Some(frame)
        } else {
            None
        }
    }

    pub unsafe fn get_2mb_frame(&mut self) -> Option<usize> {
        if let Some(frame) = self.two_megabyte_pages.get_frame() {
            Some(frame)
        } else if let Some(frame) = self.gigabyte_pages.as_mut()?.get_frame() {
            self.two_megabyte_pages
                .add_frames((frame + TWO_MEGABYTES)..(frame + GIGABYTE));
            Some(frame)
        } else {
            None
        }
    }
}

const TWO_MEGABYTES: usize = 0x20_0000;
