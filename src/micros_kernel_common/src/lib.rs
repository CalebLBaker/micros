#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::missing_errors_doc)]
#![feature(slice_split_at_unchecked)]
#![feature(pointer_is_aligned)]

use core::{
    cmp::{max, min},
    iter::once,
    mem::{align_of, size_of},
    ops::Range,
    ptr::addr_of,
    fmt::Write,
    slice,
    str,
};

pub trait Architecture: Sized {
    const INITIAL_VIRTUAL_MEMORY_SIZE: usize;

    type PageTable;

    type ExecutableHeader: ExecutableHeader;

    type SegmentHeader: SegmentHeader;

    unsafe fn initialize_memory_manager_page_tables(&mut self) -> Option<*mut Self::PageTable>;

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>);

    unsafe fn copy_into_address_space(
        &mut self,
        root_page_table: &mut Self::PageTable,
        address: usize,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()>;
}

pub trait ExecutableHeader {
    fn is_valid(&self, file_size: usize) -> bool;

    fn num_segments(&self) -> usize;

    fn segment_header_table_offset(&self) -> usize;

    fn entry(&self) -> usize;
}

pub trait SegmentHeader {
    fn segment_type(&self) -> u32;
    fn offset(&self) -> usize;
    fn address(&self) -> usize;
    fn file_size(&self) -> usize;
    fn memory_size(&self) -> usize;
    fn flags(&self) -> SegmentFlags;
}

#[derive(Clone, Copy)]
pub struct SegmentFlags(pub u32);

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

pub struct FrameAllocator<const FRAME_SIZE: usize> {
    next: Option<*mut FrameAllocator<FRAME_SIZE>>,
}

impl<const MEMORY_FRAME_SIZE: usize> FrameAllocator<MEMORY_FRAME_SIZE> {
    const FRAME_SIZE: usize = MEMORY_FRAME_SIZE;

    pub unsafe fn add_frames(&mut self, memory_area: Range<usize>) {
        for frame in memory_area.step_by(Self::FRAME_SIZE) {
            self.add_frame(frame);
        }
    }

    pub unsafe fn get_frame(&mut self) -> Option<usize> {
        let ret = self.next?;
        self.next = (*ret).next;
        Some(ret as usize)
    }

    pub unsafe fn add_frame(&mut self, frame_address: usize) {
        let frame_ptr = frame_address as *mut Self;
        (*frame_ptr).next = self.next;
        self.next = Some(&mut *frame_ptr);
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
}

impl<const FRAME_SIZE: usize> Default for FrameAllocator<FRAME_SIZE> {
    fn default() -> Self {
        Self { next: None }
    }
}

pub struct ProcessLaunchInfo {
    pub root_page_table_address: usize,
    pub entry_point: usize,
}

pub unsafe fn boot_os<Proc: Architecture>(
    proc: &mut Proc,
    multiboot_info_ptr: u32,
) -> Option<ProcessLaunchInfo> {
    // Initialize available memory and set up page tables
    let boot_info_size = (*(multiboot_info_ptr as *const BootInformationHeader)).total_size as usize;
    let boot_info = BootInformation { tags: slice::from_raw_parts(multiboot_info_ptr as *const u8, boot_info_size).split_at_unchecked(size_of::<BootInformationHeader>()).1 };

    micros_console_writer::WRITER.lock().write_str("parsed boot info size\n");

    let mut physical_memory_size = 0;

    // Add free frames from first 4 GB to available frame list
    let memory_manager_bounds = memory_manager_executable(boot_info)?;

    micros_console_writer::WRITER.lock().write_str("found memory manager executable\n");

    let mut memory_regions_in_use = [
        addr_of!(header_start) as usize..addr_of!(kernel_end) as usize,
        boot_info.address_range(),
        memory_manager_bounds.clone(),
    ];
    let available_memory_regions = unused_memory_regions(
        &mut memory_regions_in_use,
        Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
    )?;
    micros_console_writer::WRITER.lock().write_str("found unused memory regions\n");
    for memory_area in available_memory_areas(boot_info.tags_of_type::<MemoryMapTag>().next()?) {
        writeln!(micros_console_writer::WRITER.lock(), "hi");
        physical_memory_size = max(physical_memory_size, memory_area_end(memory_area));
        writeln!(micros_console_writer::WRITER.lock(), "memory area: {:?}", memory_area);
        for memory_region in
            unused_memory_regions_from_area(memory_area, available_memory_regions.clone())
        {
            proc.register_memory_region(memory_region);
        }
    }
    micros_console_writer::WRITER.lock().write_str("registered memory regions\n");

    load_memory_manager(proc, memory_manager_bounds)
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

#[must_use]
pub fn end_of_last_full_page(end_address: usize, page_size: usize) -> usize {
    end_address - end_address % page_size
}

pub fn copy_and_zero_fill(dest: &mut [u8], src: &[u8]) {
    dest[0..src.len()].copy_from_slice(src);
    dest[src.len()..].fill(0);
}

#[must_use]
pub fn slice_with_bounds_check(src: &[u8], index: usize, len: usize) -> &[u8] {
    &src[index.min(src.len())..(index + len).min(src.len())]
}

extern "C" {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
}

const ELF_LOADABLE_SEGMENT: u32 = 1;
const ELF_WRITABLE_SEGMENT: u32 = 2;
const ELF_EXECUTABLE_SEGMENT: u32 = 1;

const AVAILABLE_MEMORY: u32 = 1;
const ACPI_MEMORY: u32 = 3;

trait MutibootTag<'a> : TryFrom<&'a [u8]> {
    const TAG_TYPE: u32;
}

#[repr(C, align(8))]
struct BootInformationHeader {
    total_size: u32,
    reserved: u32,
}

#[repr(C, align(8))]
#[derive(Debug)]
struct BootInfoTagHeader {
    tag_type: u32,
    size: u32,
}

#[repr(C)]
#[derive(Debug)]
struct MemoryMapEntry {
    base_addr: u64,
    length: u64,
    region_type: u32,
    reserved: u32,
}

#[repr(C)]
struct MemoryMapHeader {
    tag_header: BootInfoTagHeader,
    entry_size: u32,
    entry_version: u32,
}

#[derive(Clone, Copy)]
struct MemoryMapTag<'a> {
    entries: &'a[MemoryMapEntry]
}

impl<'a> TryFrom<&'a [u8]> for MemoryMapTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let entries_num_bytes = value.len() - size_of::<MemoryMapHeader>();
        if value.len() < size_of::<MemoryMapHeader>() || entries_num_bytes % size_of::<MemoryMapEntry>() != 0 {
            Err(())
        }
        else {
            let pointer = value.as_ptr();
            let header = unsafe { &*aligned_pointer_cast::<MemoryMapHeader>(pointer).ok_or(())? };
            if header.entry_size as usize != size_of::<MemoryMapHeader>() || header.entry_version != 0 {
                Err(())
            }
            else {
                let num_entries = entries_num_bytes / size_of::<MemoryMapEntry>();
                Ok(MemoryMapTag { entries: unsafe { slice::from_raw_parts(aligned_pointer_cast::<MemoryMapEntry>(pointer.add(size_of::<MemoryMapHeader>())).ok_or(())?, num_entries) } } )
            }
        }
    }
}

impl<'a> MutibootTag<'a> for MemoryMapTag<'a> {
    const TAG_TYPE: u32 = 6;
}

#[repr(C)]
struct BootModuleHeader {
    tag_header: BootInfoTagHeader,
    mod_start: u32,
    mod_end: u32,
}

struct BootModuleTag<'a> {
    mod_start: u32,
    mod_end: u32,
    string: &'a str,
}

impl<'a> TryFrom<&'a [u8]> for BootModuleTag<'a> {
    type Error = ();

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        if value.len() < size_of::<BootModuleHeader>() {
            Err(())
        }
        else {
            let header = unsafe { &*aligned_pointer_cast::<BootModuleHeader>(value.as_ptr()).ok_or(())? };
            Ok(Self {
                mod_start: header.mod_start,
                mod_end: header.mod_end,
                string: str::from_utf8(value.split_at(size_of::<BootModuleHeader>()).1).map_err(|_| ())?.split('\0').next().ok_or(())?,
            })
        }
    }
}

impl<'a> MutibootTag<'a> for BootModuleTag<'a> {
    const TAG_TYPE: u32 = 3;
}

struct BootInfoTag<'a> {
    tag_type: u32,
    data: &'a [u8],
}

#[derive(Clone, Copy)]
struct BootInformation<'a> {
    tags: &'a [u8]
}

impl<'a> BootInformation<'a> {
    fn tags_of_type<TagType: MutibootTag<'a> + 'a>(self) -> impl Iterator<Item = TagType> + 'a {
        self.into_iter().filter_map(|tag| {
            if tag.tag_type == TagType::TAG_TYPE {
                tag.data.try_into().ok()
            }
            else {
                None
            }
        })
    }

    fn address_range(self) -> Range<usize> {
        let tag_range = self.tags.as_ptr_range();
        tag_range.start as usize - size_of::<BootInformationHeader>() .. tag_range.end as usize
    }
}

impl<'a> IntoIterator for BootInformation<'a> {
    type Item = BootInfoTag<'a>;
    type IntoIter = MultibootTagIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter { tags: self.tags }
    }
}

struct MultibootTagIterator<'a> {
    tags: &'a [u8]
}

impl<'a> Iterator for MultibootTagIterator<'a> {
    type Item = BootInfoTag<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.tags.len() < size_of::<BootInfoTagHeader>() {
            writeln!(micros_console_writer::WRITER.lock(), "tags too small; size: {}", self.tags.len());
            None
        }
        else {
            let pointer = self.tags.as_ptr();
            let padding_size = pointer.align_offset(align_of::<BootInfoTagHeader>());
            let tag_header = unsafe { &*(pointer.add(padding_size) as *const BootInfoTagHeader) };
            let tag_size = tag_header.size as usize;
            if self.tags.len() < tag_size + padding_size {
                writeln!(micros_console_writer::WRITER.lock(), "tag overflow; header: {:?}, remaining data: {}, padding: {}", tag_header, self.tags.len(), padding_size);
                None
            }
            else {
                let (tag_data, remaining_data) = self.tags.split_at(padding_size).1.split_at(tag_size);
                self.tags = remaining_data;
                writeln!(micros_console_writer::WRITER.lock(), "tag: header: {:?}, address: {:?}", tag_header, self.tags.as_ptr());
                Some(Self::Item {
                    tag_type: tag_header.tag_type,
                    data: tag_data,
                })
            }
        }
    }
}

fn aligned_pointer_cast<T>(pointer: *const u8) -> Option<*const T> {
    let new_pointer = pointer.cast::<T>();
    if new_pointer.is_aligned() {
        micros_console_writer::WRITER.lock().write_str("successful aligned cast\n");
        Some(new_pointer)
    }
    else {
        micros_console_writer::WRITER.lock().write_str("misaligned pointer\n");
        None
    }
}

unsafe fn load_memory_manager<Proc: Architecture>(
    proc: &mut Proc,
    exectuable_location: Range<usize>,
) -> Option<ProcessLaunchInfo> {
    let memory_manager_root_page_table = proc.initialize_memory_manager_page_tables()?;

    let memory_manager_elf_header = &*(exectuable_location.start as *const Proc::ExecutableHeader);

    if !memory_manager_elf_header.is_valid(exectuable_location.len()) {
        return None;
    }

    for segment_header in slice::from_raw_parts(
        (exectuable_location.start + memory_manager_elf_header.segment_header_table_offset())
            as *const Proc::SegmentHeader,
        memory_manager_elf_header.num_segments(),
    )
    .iter()
    .filter(|header| header.segment_type() == ELF_LOADABLE_SEGMENT)
    {
        if segment_header.offset() + segment_header.file_size() > exectuable_location.len()
            || segment_header.file_size() > segment_header.memory_size()
        {
            return None;
        }
        proc.copy_into_address_space(
            &mut *memory_manager_root_page_table,
            segment_header.address(),
            slice::from_raw_parts(
                (exectuable_location.start + segment_header.offset()) as *const u8,
                segment_header.file_size(),
            ),
            segment_header.memory_size(),
            segment_header.flags(),
        );
    }

    Some(ProcessLaunchInfo {
        root_page_table_address: memory_manager_root_page_table as usize,
        entry_point: memory_manager_elf_header.entry(),
    })
}

// I'm only supporting 64 bit machines as of now so casting from u64 to usize shouldn't result
// in any truncation. Will need to revisit if I ever add support for 32 bit machines.
#[allow(clippy::cast_possible_truncation)]
fn memory_area_start(area: &MemoryMapEntry) -> usize {
    area.base_addr as usize
}

#[allow(clippy::cast_possible_truncation)]
fn memory_area_end(area: &MemoryMapEntry) -> usize {
    (area.base_addr + area.length) as usize
}

fn memory_manager_executable(boot_info: BootInformation) -> Option<Range<usize>> {
    let memory_manager = boot_info.tags_of_type::<BootModuleTag>().find(|module| { module.string.contains("memory_manager") })?;
    Some(memory_manager.mod_start as usize..memory_manager.mod_end as usize)
}

fn intersect(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    max(a.start, b.start)..min(a.end, b.end)
}

fn unused_memory_regions_from_area<'a, RangeIter: Iterator<Item = Range<usize>> + 'a>(
    memory_area: &'a MemoryMapEntry,
    unused_memory_regions: RangeIter,
) -> impl Iterator<Item = Range<usize>> + 'a {
    let area = memory_area_start(memory_area)..memory_area_end(memory_area);
    unused_memory_regions
        .map(move |region| intersect(area.clone(), region.clone()))
        .filter(|region| !region.is_empty())
}

fn unused_memory_regions(
    memory_regions_in_use: &mut [Range<usize>],
    max_address: usize,
) -> Option<impl Iterator<Item = Range<usize>> + Clone + '_> {
    memory_regions_in_use.sort_unstable_by(|a, b| a.start.cmp(&b.start));
    Some(
        once(0..memory_regions_in_use.first()?.start)
            .chain(
                memory_regions_in_use
                    .windows(2)
                    .map(|window| window[0].end..window[1].start),
            )
            .chain(once(memory_regions_in_use.last()?.end..max_address)),
    )
}

fn available_memory_areas(memory_map: MemoryMapTag) -> impl Iterator<Item = &MemoryMapEntry> {
    micros_console_writer::WRITER.lock().write_str("hi\n");
    memory_map.entries.iter().filter(|area| {
        micros_console_writer::WRITER.lock().write_str("hello\n");
        area.region_type == AVAILABLE_MEMORY || area.region_type == ACPI_MEMORY
    })
}
