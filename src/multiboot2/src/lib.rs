#![no_std]
#![feature(pointer_is_aligned)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_safety_doc)]

use core::{
    mem::{align_of, size_of},
    ops::Range,
    slice, str,
};

/// The value of the `region_type` field for `MemoryMapEntry`'s that represent available memory.
pub const AVAILABLE_MEMORY: u32 = 1;
/// The value of the `region_type` field for `MemoryMapEntry`'s that represent ACPI memory.
pub const ACPI_MEMORY: u32 = 3;

/// A type that can represent a tag from the multiboot2 boot information structure.
pub trait MutibootTag<'a>: TryFrom<&'a [u8]> {
    const TAG_TYPE: u32;
}

/// The header that is present at the beginning of the multiboot2 information structure.
#[repr(C, align(8))]
pub struct BootInformationHeader {
    /// The size of the boot information structure in bytes.
    pub total_size: u32,
    reserved: u32,
}

/// An entry in the memory map that represents a region of memory
#[repr(C)]
pub struct MemoryMapEntry {
    /// The address of the memory region
    pub base_addr: u64,
    /// The size of the memory region in bytes
    pub length: u64,
    /// The type of memory in the region (e.g. available memory or ACPI memory)
    pub region_type: u32,
    reserved: u32,
}

/// A multiboot2 tag containing a map of the device's memory
#[derive(Clone, Copy)]
pub struct MemoryMapTag<'a> {
    pub entries: &'a [MemoryMapEntry],
}

impl<'a> TryFrom<&'a [u8]> for MemoryMapTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let entries_num_bytes = value.len() - size_of::<MemoryMapHeader>();
        if value.len() < size_of::<MemoryMapHeader>()
            || entries_num_bytes % size_of::<MemoryMapEntry>() != 0
        {
            Err(())
        } else {
            let pointer = value.as_ptr();
            let header = unsafe { &*aligned_pointer_cast::<MemoryMapHeader>(pointer).ok_or(())? };
            if header.entry_size as usize != size_of::<MemoryMapEntry>()
                || header.entry_version != 0
            {
                Err(())
            } else {
                let num_entries = entries_num_bytes / size_of::<MemoryMapEntry>();
                Ok(MemoryMapTag {
                    entries: unsafe {
                        slice::from_raw_parts(
                            aligned_pointer_cast::<MemoryMapEntry>(
                                pointer.add(size_of::<MemoryMapHeader>()),
                            )
                            .ok_or(())?,
                            num_entries,
                        )
                    },
                })
            }
        }
    }
}

impl<'a> MutibootTag<'a> for MemoryMapTag<'a> {
    const TAG_TYPE: u32 = 6;
}

/// A multiboot2 info tag describing a boot module
pub struct BootModuleTag<'a> {
    /// The address of the start of the boot module
    pub mod_start: u32,
    /// The address of the end of the boot module
    pub mod_end: u32,
    /// A string value affiliated with the boot module
    pub string: &'a str,
}

impl<'a> TryFrom<&'a [u8]> for BootModuleTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        if value.len() < size_of::<BootModuleHeader>() {
            Err(())
        } else {
            let header =
                unsafe { &*aligned_pointer_cast::<BootModuleHeader>(value.as_ptr()).ok_or(())? };
            Ok(Self {
                mod_start: header.mod_start,
                mod_end: header.mod_end,
                string: str::from_utf8(
                    value
                        .split_first_chunk::<{ size_of::<BootModuleHeader>() }>()
                        .ok_or(())?
                        .1,
                )
                .map_err(|_| ())?
                .split('\0')
                .next()
                .ok_or(())?,
            })
        }
    }
}

impl<'a> MutibootTag<'a> for BootModuleTag<'a> {
    const TAG_TYPE: u32 = 3;
}

/// A multiboot2 info tag containing information about the framebuffer
pub struct FramebufferTag<'a> {
    /// A pointer to the framebuffer
    pub framebuffer: *mut u8,
    /// The size of a row in the framebuffer in bytes
    pub pitch: u32,
    /// The size of a row in the framebuffer in pixels
    pub width: u32,
    /// The number of rows in the framebuffer
    pub height: u32,
    /// The size of a pixel in bits
    pub bits_per_pixel: u8,
    /// The type of framebuffer.
    pub framebuffer_type: u8,
    /// Data about the framebuffer's color format. The layout of this data depends on the
    /// framebuffer type
    pub color_data: &'a [u8],
}

impl<'a> TryFrom<&'a [u8]> for FramebufferTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let header_len = size_of::<FramebufferTagHeader>();
        if value.len() < header_len {
            Err(())
        } else {
            let header = unsafe {
                &*aligned_pointer_cast::<FramebufferTagHeader>(value.as_ptr()).ok_or(())?
            };
            Ok(Self {
                framebuffer: header.framebuffer as *mut u8,
                pitch: header.pitch,
                width: header.width,
                height: header.height,
                bits_per_pixel: header.bits_per_pixel,
                framebuffer_type: header.framebuffer_type,
                color_data: &value[header_len..],
            })
        }
    }
}

impl<'a> MutibootTag<'a> for FramebufferTag<'a> {
    const TAG_TYPE: u32 = 8;
}

/// A multiboot2 boot info tag
pub struct BootInfoTag<'a> {
    tag_type: u32,
    data: &'a [u8],
}

/// Boot information provided to the operating system by the boot loader
#[derive(Clone, Copy)]
pub struct BootInformation<'a> {
    pub tags: &'a [u8],
}

impl<'a> BootInformation<'a> {
    pub unsafe fn new(boot_info_ptr: *const u8) -> Self {
        let boot_info_size = (*(boot_info_ptr as *const BootInformationHeader)).total_size as usize;
        BootInformation {
            tags: slice::from_raw_parts(boot_info_ptr, boot_info_size)
                .split_at_unchecked(size_of::<BootInformationHeader>())
                .1,
        }
    }

    pub fn tags_of_type<TagType: MutibootTag<'a> + 'a>(self) -> impl Iterator<Item = TagType> + 'a {
        self.into_iter().filter_map(|tag| {
            if tag.tag_type == TagType::TAG_TYPE {
                tag.data.try_into().ok()
            } else {
                None
            }
        })
    }

    pub fn address_range(self) -> Range<usize> {
        let tag_range = self.tags.as_ptr_range();
        tag_range.start as usize - size_of::<BootInformationHeader>()..tag_range.end as usize
    }
}

impl<'a> IntoIterator for BootInformation<'a> {
    type Item = BootInfoTag<'a>;
    type IntoIter = MultibootTagIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter { tags: self.tags }
    }
}

pub struct MultibootTagIterator<'a> {
    tags: &'a [u8],
}

impl<'a> Iterator for MultibootTagIterator<'a> {
    type Item = BootInfoTag<'a>;

    // Allow casts to stricter pointer alignments since we manually correct for alignment before
    // casting
    #[allow(clippy::cast_ptr_alignment)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.tags.len() < size_of::<BootInfoTagHeader>() {
            None
        } else {
            let pointer = self.tags.as_ptr();
            let padding_size = pointer.align_offset(align_of::<BootInfoTagHeader>());
            let tag_header = unsafe { &*(pointer.add(padding_size).cast::<BootInfoTagHeader>()) };
            let tag_size = tag_header.size as usize;
            if self.tags.len() < tag_size + padding_size {
                None
            } else {
                let (tag_data, remaining_data) =
                    self.tags.split_at(padding_size).1.split_at(tag_size);
                self.tags = remaining_data;
                Some(Self::Item {
                    tag_type: tag_header.tag_type,
                    data: tag_data,
                })
            }
        }
    }
}

pub fn aligned_pointer_cast<T>(pointer: *const u8) -> Option<*const T> {
    let new_pointer = pointer.cast::<T>();
    if new_pointer.is_aligned() {
        Some(new_pointer)
    } else {
        None
    }
}

#[repr(C)]
struct FramebufferTagHeader {
    header: BootInfoTagHeader,
    framebuffer: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u8,
    framebuffer_type: u8,
    reserved: u8,
}

#[repr(C, align(8))]
struct BootInfoTagHeader {
    tag_type: u32,
    size: u32,
}

#[repr(C)]
struct MemoryMapHeader {
    tag_header: BootInfoTagHeader,
    entry_size: u32,
    entry_version: u32,
}

#[repr(C)]
struct BootModuleHeader {
    tag_header: BootInfoTagHeader,
    mod_start: u32,
    mod_end: u32,
}
