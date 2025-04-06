#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

#[cfg(target_arch = "x86_64")]
mod amd64;

use address::{PhysicalAddress, VirtualAddress};
use architecture::Architecture;
use core::{
    cmp::{max, min},
    iter::once,
    num::TryFromIntError,
    ops::Range,
    ptr, slice,
};
use elf::{ExecutableHeader, SegmentFlags, SegmentHeader, SegmentType};
use multiboot2::{
    BootInformation, BootInformationHeader, BootModuleTag, FramebufferTag, MemoryMapEntry,
    MemoryMapTag,
    MemoryRegionType::{AcpiMemory, AvailableMemory},
};
use ptr::addr_of;

#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    unsafe {
        let _ = amd64::initialize_operating_system(multiboot_info_ptr, cpu_info);
        amd64::halt()
    }
}

trait ArchitectureKernel: Architecture {
    type PageTable;

    type ExecutableHeader: ExecutableHeader;

    type SegmentHeader: SegmentHeader;

    unsafe fn initialize_memory_manager_page_tables(&mut self) -> Option<*mut Self::PageTable>;

    unsafe fn register_memory_region(&mut self, memory_region: Range<*mut u8>);

    unsafe fn copy_into_address_space(
        &mut self,
        root_page_table: &mut Self::PageTable,
        address: VirtualAddress,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()>;

    fn virtual_memory_end(&self) -> *const u8;
}

struct ProcessLaunchInfo {
    root_page_table_address: PhysicalAddress,
    entry_point: VirtualAddress,
}

enum BootError {
    MemoryManagerNotLoaded = 0,
    NoMemoryMap,
    InvalidAddress,
    OutOfMemory,
    InvalidElfFile,
    NegativeBootModuleSize,
    InvalidElfSegment,
    KernelBugMemoryRegionsInUseEmpty,
}

unsafe fn boot_os<Proc: ArchitectureKernel>(
    proc: &mut Proc,
    multiboot_info_ptr: *const BootInformationHeader,
    architecture_specific_reserved_memory: Range<*const u8>,
) -> Result<ProcessLaunchInfo, BootError> {
    // Initialize available memory and set up page tables
    let boot_info = unsafe { BootInformation::new(multiboot_info_ptr) };

    let memory_manager_bounds =
        memory_manager_executable(proc, boot_info).ok_or(BootError::MemoryManagerNotLoaded)?;

    // Add free frames from first 4 GB to available frame list
    let mut memory_regions_in_use_arr = [
        addr_of!(header_start)..addr_of!(kernel_end),
        boot_info.address_range(),
        memory_manager_bounds.clone(),
        architecture_specific_reserved_memory.clone(),
        ptr::null()..ptr::null(),
    ];
    let memory_regions_in_use =
        if let Some(framebuffer_tag) = boot_info.tags_of_type::<FramebufferTag>().next() {
            let framebuffer_addr = proc.physical_address_to_pointer(framebuffer_tag.framebuffer);
            memory_regions_in_use_arr[4] = framebuffer_addr
                ..framebuffer_addr
                    .wrapping_add(framebuffer_tag.height as usize * framebuffer_tag.pitch as usize);
            &mut memory_regions_in_use_arr
        } else {
            &mut memory_regions_in_use_arr[0..4]
        };
    let available_memory_regions =
        unused_memory_regions(memory_regions_in_use, proc.virtual_memory_end())
            .ok_or(BootError::KernelBugMemoryRegionsInUseEmpty)?;

    for memory_area in available_memory_areas(
        boot_info
            .tags_of_type::<MemoryMapTag>()
            .next()
            .ok_or(BootError::NoMemoryMap)?,
    ) {
        for memory_region in
            unused_memory_regions_from_area(proc, memory_area, available_memory_regions.clone())
                .map_err(|_| BootError::InvalidAddress)?
        {
            unsafe {
                proc.register_memory_region(memory_region);
            }
        }
    }

    unsafe { load_memory_manager(proc, memory_manager_bounds) }
}

fn copy_and_zero_fill(dest: &mut [u8], src: &[u8]) {
    dest[0..src.len()].copy_from_slice(src);
    dest[src.len()..].fill(0);
}

#[must_use]
fn slice_with_bounds_check(src: &[u8], index: usize, len: usize) -> &[u8] {
    &src[index.min(src.len())..(index + len).min(src.len())]
}

unsafe extern "C" {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
}

unsafe fn load_memory_manager<Proc: ArchitectureKernel>(
    proc: &mut Proc,
    exectuable_location: Range<*const u8>,
) -> Result<ProcessLaunchInfo, BootError> {
    let memory_manager_root_page_table = unsafe {
        proc.initialize_memory_manager_page_tables()
            .ok_or(BootError::OutOfMemory)?
    };

    let memory_manager_elf_header =
        unsafe { &*(exectuable_location.start.cast::<Proc::ExecutableHeader>()) };

    let executable_size = unsafe {
        exectuable_location
            .end
            .offset_from(exectuable_location.start)
    }
    .try_into()
    .map_err(|_| BootError::NegativeBootModuleSize)?;
    if !memory_manager_elf_header.is_valid(executable_size) {
        return Err(BootError::InvalidElfFile);
    }

    for segment_header in unsafe {
        slice::from_raw_parts(
            (exectuable_location
                .start
                .add(memory_manager_elf_header.segment_header_table_offset()))
            .cast::<Proc::SegmentHeader>(),
            memory_manager_elf_header.num_segments(),
        )
    }
    .iter()
    .filter(|header| header.segment_type() == SegmentType::Loadable)
    {
        if segment_header.offset() + segment_header.file_size() > executable_size
            || segment_header.file_size() > segment_header.memory_size()
        {
            return Err(BootError::InvalidElfSegment);
        }
        unsafe {
            let _ = proc
                .copy_into_address_space(
                    &mut *memory_manager_root_page_table,
                    segment_header.address(),
                    slice::from_raw_parts(
                        exectuable_location.start.add(segment_header.offset()),
                        segment_header.file_size(),
                    ),
                    segment_header.memory_size(),
                    segment_header.flags(),
                )
                .ok_or(BootError::OutOfMemory);
        };
    }

    Ok(ProcessLaunchInfo {
        root_page_table_address: proc.pointer_to_physical_address(memory_manager_root_page_table),
        entry_point: memory_manager_elf_header.entry(),
    })
}

fn memory_area_start<Proc: Architecture>(
    proc: &Proc,
    area: &MemoryMapEntry,
) -> Result<*mut u8, TryFromIntError> {
    Ok(proc.physical_address_to_pointer::<u8>(area.base_address()?))
}

fn memory_area_end<Proc: Architecture>(
    proc: &Proc,
    area: &MemoryMapEntry,
) -> Result<*mut u8, TryFromIntError> {
    Ok(
        proc.physical_address_to_pointer::<u8>(
            area.base_address()? + usize::try_from(area.length)?,
        ),
    )
}

fn memory_manager_executable<Proc: Architecture>(
    proc: &Proc,
    boot_info: BootInformation,
) -> Option<Range<*const u8>> {
    let memory_manager = boot_info
        .tags_of_type::<BootModuleTag>()
        .find(|module| module.string.contains("memory_manager"))?;
    Some(
        proc.physical_address_to_pointer(memory_manager.mod_start)
            ..proc.physical_address_to_pointer(memory_manager.mod_end),
    )
}

fn intersect(a: Range<*mut u8>, b: Range<*mut u8>) -> Range<*mut u8> {
    max(a.start, b.start)..min(a.end, b.end)
}

fn unused_memory_regions_from_area<
    'a,
    RangeIter: Iterator<Item = Range<*mut u8>> + 'a,
    Proc: Architecture,
>(
    proc: &Proc,
    memory_area: &'a MemoryMapEntry,
    unused_memory_regions: RangeIter,
) -> Result<impl Iterator<Item = Range<*mut u8>> + 'a, TryFromIntError> {
    let area = memory_area_start(proc, memory_area)?..memory_area_end(proc, memory_area)?;
    Ok(unused_memory_regions
        .map(move |region| intersect(area.clone(), region.clone()))
        .filter(|region| !region.is_empty()))
}

fn unused_memory_regions(
    memory_regions_in_use: &mut [Range<*const u8>],
    max_address: *const u8,
) -> Option<impl Iterator<Item = Range<*mut u8>> + Clone + '_> {
    memory_regions_in_use.sort_unstable_by(|a, b| a.start.cmp(&b.start));
    Some(
        once(ptr::null()..memory_regions_in_use.first()?.start)
            .chain(
                memory_regions_in_use
                    .windows(2)
                    .map(|window| window[0].end..window[1].start),
            )
            .chain(once(memory_regions_in_use.last()?.end..max_address))
            .map(|region| region.start.cast_mut()..region.end.cast_mut()),
    )
}

fn available_memory_areas(memory_map: MemoryMapTag) -> impl Iterator<Item = &MemoryMapEntry> {
    memory_map
        .entries
        .iter()
        .filter(|area| area.memory_type() == AvailableMemory || area.memory_type() == AcpiMemory)
}
