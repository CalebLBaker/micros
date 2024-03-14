#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]
#![feature(slice_split_at_unchecked)]
#![feature(pointer_is_aligned)]
#![feature(abi_x86_interrupt)]

mod multiboot2;

#[cfg(target_arch = "x86_64")]
mod amd64;

use core::{
    cmp::{max, min},
    iter::once,
    mem::size_of,
    ops::Range,
    ptr::addr_of,
    slice,
};
use multiboot2::{
    BootInformation, BootInformationHeader, BootModuleTag, MemoryMapEntry, MemoryMapTag,
    ACPI_MEMORY, AVAILABLE_MEMORY,
};

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    unsafe {
        amd64::initialize_operating_system(multiboot_info_ptr, cpu_info);
    }
    amd64::halt()
}

trait Architecture: Sized {
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

trait ExecutableHeader {
    fn is_valid(&self, file_size: usize) -> bool;

    fn num_segments(&self) -> usize;

    fn segment_header_table_offset(&self) -> usize;

    fn entry(&self) -> usize;
}

trait SegmentHeader {
    fn segment_type(&self) -> u32;
    fn offset(&self) -> usize;
    fn address(&self) -> usize;
    fn file_size(&self) -> usize;
    fn memory_size(&self) -> usize;
    fn flags(&self) -> SegmentFlags;
}

#[derive(Clone, Copy)]
struct SegmentFlags(u32);

impl SegmentFlags {
    #[must_use]
    fn writable(self) -> bool {
        (self.0 & ELF_WRITABLE_SEGMENT) != 0
    }

    #[must_use]
    fn executable(self) -> bool {
        (self.0 & ELF_EXECUTABLE_SEGMENT) != 0
    }
}

struct ProcessLaunchInfo {
    root_page_table_address: usize,
    entry_point: usize,
}

unsafe fn boot_os<Proc: Architecture>(
    proc: &mut Proc,
    multiboot_info_ptr: u32,
) -> Option<ProcessLaunchInfo> {
    // Initialize available memory and set up page tables
    let boot_info_size =
        (*(multiboot_info_ptr as *const BootInformationHeader)).total_size as usize;
    let boot_info = BootInformation {
        tags: slice::from_raw_parts(multiboot_info_ptr as *const u8, boot_info_size)
            .split_at_unchecked(size_of::<BootInformationHeader>())
            .1,
    };

    let mut physical_memory_size = 0;

    // Add free frames from first 4 GB to available frame list
    let memory_manager_bounds = memory_manager_executable(boot_info)?;

    let mut memory_regions_in_use = [
        addr_of!(header_start) as usize..addr_of!(kernel_end) as usize,
        boot_info.address_range(),
        memory_manager_bounds.clone(),
    ];
    let available_memory_regions = unused_memory_regions(
        &mut memory_regions_in_use,
        Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
    )?;

    for memory_area in available_memory_areas(boot_info.tags_of_type::<MemoryMapTag>().next()?) {
        physical_memory_size = max(physical_memory_size, memory_area_end(memory_area));
        for memory_region in
            unused_memory_regions_from_area(memory_area, available_memory_regions.clone())
        {
            proc.register_memory_region(memory_region);
        }
    }

    load_memory_manager(proc, memory_manager_bounds)
}

fn copy_and_zero_fill(dest: &mut [u8], src: &[u8]) {
    dest[0..src.len()].copy_from_slice(src);
    dest[src.len()..].fill(0);
}

#[must_use]
fn slice_with_bounds_check(src: &[u8], index: usize, len: usize) -> &[u8] {
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
    let memory_manager = boot_info
        .tags_of_type::<BootModuleTag>()
        .find(|module| module.string.contains("memory_manager"))?;
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
    memory_map
        .entries
        .iter()
        .filter(|area| area.region_type == AVAILABLE_MEMORY || area.region_type == ACPI_MEMORY)
}
