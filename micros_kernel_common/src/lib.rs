#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::missing_errors_doc)]

use core::{
    cmp::{max, min},
    iter::once,
    ops::Range,
    ptr::addr_of,
    slice,
};

use multiboot2::{
    BootInformation, BootInformationHeader, MbiLoadError, MemoryArea, MemoryAreaType, MemoryMapTag,
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
}

pub enum Error {
    MultibootHeaderLoad(MbiLoadError),
    NoMemoryMap,
    NoMemoryManager,
    AssertionError,
    InvalidMemoryManagerModule,
    FailedToSetupMemoryManagerAddressSpace,
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
) -> Result<ProcessLaunchInfo, Error> {
    // Initialize available memory and set up page tables
    let boot_info = BootInformation::load(multiboot_info_ptr as *const BootInformationHeader)
        .map_err(Error::MultibootHeaderLoad)?;

    boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let memory_map_tag = boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let mut physical_memory_size = 0;

    // Add free frames from first 4 GB to available frame list
    let memory_manager_bounds =
        memory_manager_executable(&boot_info).ok_or(Error::NoMemoryManager)?;

    let mut memory_regions_in_use = [
        addr_of!(header_start) as usize..addr_of!(kernel_end) as usize,
        boot_info.start_address()..boot_info.end_address(),
        memory_manager_bounds.clone(),
    ];
    let available_memory_regions = unused_memory_regions(
        &mut memory_regions_in_use,
        Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
    )
    .ok_or(Error::AssertionError)?;
    for memory_area in available_memory_areas(memory_map_tag).take(2) {
        physical_memory_size = max(physical_memory_size, memory_area_end(memory_area));
        for memory_region in
            unused_memory_regions_from_area(memory_area, available_memory_regions.clone())
        {
            proc.register_memory_region(memory_region);
        }
    }

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

unsafe fn load_memory_manager<Proc: Architecture>(
    proc: &mut Proc,
    exectuable_location: Range<usize>,
) -> Result<ProcessLaunchInfo, Error> {
    let memory_manager_root_page_table = proc
        .initialize_memory_manager_page_tables()
        .ok_or(Error::FailedToSetupMemoryManagerAddressSpace)?;

    let memory_manager_elf_header = &*(exectuable_location.start as *const Proc::ExecutableHeader);

    if !memory_manager_elf_header.is_valid(exectuable_location.len()) {
        return Err(Error::InvalidMemoryManagerModule);
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
            return Err(Error::InvalidMemoryManagerModule);
        }
        proc.copy_into_address_space(
            &mut *memory_manager_root_page_table,
            segment_header.address(),
            slice::from_raw_parts(
                (exectuable_location.start + segment_header.offset()) as *const u8,
                segment_header.file_size(),
            ),
            segment_header.memory_size(),
        );
    }

    Ok(ProcessLaunchInfo {
        root_page_table_address: memory_manager_root_page_table as usize,
        entry_point: memory_manager_elf_header.entry(),
    })
}

// I'm only supporting 64 bit machines as of now so casting from u64 to usize shouldn't result
// in any truncation. Will need to revisit if I ever add support for 32 bit machines.
#[allow(clippy::cast_possible_truncation)]
fn memory_area_start(area: &MemoryArea) -> usize {
    area.start_address() as usize
}

#[allow(clippy::cast_possible_truncation)]
fn memory_area_end(area: &MemoryArea) -> usize {
    area.end_address() as usize
}

fn memory_manager_executable(boot_info: &BootInformation) -> Option<Range<usize>> {
    let memory_manager = boot_info.module_tags().find(|module| {
        if let Ok(command) = module.cmdline() {
            command.contains("memory_manager")
        } else {
            false
        }
    })?;
    Some(memory_manager.start_address() as usize..memory_manager.end_address() as usize)
}

fn intersect(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    max(a.start, b.start)..min(a.end, b.end)
}

fn unused_memory_regions_from_area<'a, RangeIter: Iterator<Item = Range<usize>> + 'a>(
    memory_area: &'a MemoryArea,
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

fn available_memory_areas(memory_map: &MemoryMapTag) -> impl Iterator<Item = &MemoryArea> {
    memory_map.memory_areas().iter().filter(|area| {
        area.typ() == MemoryAreaType::Available || area.typ() == MemoryAreaType::AcpiAvailable
    })
}
