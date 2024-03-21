use core::{
    mem::{align_of, size_of},
    ops::Range,
    slice, str,
};

pub const AVAILABLE_MEMORY: u32 = 1;
pub const ACPI_MEMORY: u32 = 3;

pub trait MutibootTag<'a>: TryFrom<&'a [u8]> {
    const TAG_TYPE: u32;
}

#[repr(C, align(8))]
pub struct BootInformationHeader {
    pub total_size: u32,
    reserved: u32,
}

#[repr(C)]
pub struct MemoryMapEntry {
    pub base_addr: u64,
    pub length: u64,
    pub region_type: u32,
    reserved: u32,
}

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

pub struct BootModuleTag<'a> {
    pub mod_start: u32,
    pub mod_end: u32,
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
                string: str::from_utf8(value.split_at(size_of::<BootModuleHeader>()).1)
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

pub struct FrameBufferTag<'a> {
    pub framebuffer: &'a mut[u8],
    pitch: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u8,
    framebuffer_type: FrameBufferType<'a>,
}

impl<'a> TryFrom<&'a [u8]> for FrameBufferTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let header_len = size_of::<FrameBufferTagHeader>();
        if value.len() < header_len {
            Err(())
        } else {
            let header = unsafe {
                &*aligned_pointer_cast::<FrameBufferTagHeader>(value.as_ptr()).ok_or(())?
            };
            Ok(Self {
                framebuffer: unsafe { slice::from_raw_parts_mut(header.framebuffer as *mut u8, header.pitch as usize * header.height as usize) },
                pitch: header.pitch,
                width: header.width,
                height: header.height,
                bits_per_pixel: header.bits_per_pixel,
                framebuffer_type: FrameBufferType::new(header.framebuffer_type, &value[header_len..]).ok_or(())?,
            })
        }
    }
}

impl<'a> MutibootTag<'a> for FrameBufferTag<'a> {
    const TAG_TYPE: u32 = 8;
}

pub struct BootInfoTag<'a> {
    tag_type: u32,
    data: &'a [u8],
}

#[derive(Clone, Copy)]
pub struct BootInformation<'a> {
    pub tags: &'a [u8],
}

impl<'a> BootInformation<'a> {
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

const INDEXED_COLOR_MODE: u8 = 0;
const RGB_COLOR_MODE: u8 = 1;
const EGA_TEXT_MODE: u8 = 2;

#[repr(C)]
struct FrameBufferTagHeader {
    header: BootInfoTagHeader,
    framebuffer: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bits_per_pixel: u8,
    framebuffer_type: u8,
    reserved: u8,
}

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

enum FrameBufferType<'a> {
    IndexedColor(&'a [Rgb]),
    RgbColor(FramebufferPixelDescriptor),
    Ega,
}

impl<'a> FrameBufferType<'a> {
    fn new(type_tag: u8, data: &'a [u8]) -> Option<Self> {
        match type_tag {
            INDEXED_COLOR_MODE => {
                let number_of_colors =
                    u32::from_le_bytes(data[..size_of::<u32>()].try_into().ok()?) as usize;
                if data.len() > size_of::<u32>() + number_of_colors * size_of::<Rgb>() {
                    None
                } else {
                    Some(FrameBufferType::IndexedColor(unsafe {
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
                Some(FrameBufferType::RgbColor(unsafe {*aligned_pointer_cast::<FramebufferPixelDescriptor>(data.as_ptr())?}))
            },
            EGA_TEXT_MODE => Some(FrameBufferType::Ega),
            _ => None,
        }
    }
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

fn aligned_pointer_cast<T>(pointer: *const u8) -> Option<*const T> {
    let new_pointer = pointer.cast::<T>();
    if new_pointer.is_aligned() {
        Some(new_pointer)
    } else {
        None
    }
}
