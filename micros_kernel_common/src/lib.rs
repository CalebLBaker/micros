#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::missing_errors_doc)]

use core::{
    cmp::{max, min},
    ops::Range,
    ptr::addr_of,
};

use multiboot2::{
    BootInformation, BootInformationHeader, MbiLoadError, MemoryArea, MemoryAreaType, MemoryMapTag,
};

pub trait Architecture<'a>: Sized {
    const INITIAL_VIRTUAL_MEMORY_SIZE: usize;

    type PageTable: PageTable<'a>;

    unsafe fn get_root_page_table() -> *mut Self::PageTable;

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>);

    fn initial_page_table_counts(&self) -> &'static [usize];

    fn kernel_page_size(&self) -> usize;

    unsafe fn register_available_memory_areas_from_region(
        &mut self,
        memory_map: &MemoryMapTag,
        memory_region: Range<usize>,
    ) {
        for memory_area in available_memory_areas(memory_map) {
            self.register_memory_region(intersect(
                memory_region.clone(),
                memory_area_start(memory_area)..memory_area_end(memory_area),
            ));
        }
    }

    unsafe fn get_frame_for_page_table(
        &mut self,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
    ) -> GetFrameResponse;

    unsafe fn identity_map_entry(
        &mut self,
        entry: <Self::PageTable as PageTable<'a>>::Entry,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
        remaining_page_table_levels: usize,
        physical_memory_size: usize,
    ) -> IdentityMapEntryResult {
        if memory_state.virtual_memory_size >= physical_memory_size {
            return IdentityMapEntryResult {
                memory_state,
                finished: true,
            };
        }
        let get_frame_response = self.get_frame_for_page_table(memory_map, memory_state);
        let new_memory_state = MemoryState {
            virtual_memory_size: memory_state.virtual_memory_size,
            last_frame_added_to_allocator: get_frame_response.last_frame_added_to_allocator,
        };
        match get_frame_response.frame {
            Some(frame) => {
                entry.set(frame, Self::PageTable::kernel_page_table_flags());
                IdentityMapEntryResult {
                    memory_state: self.identity_map(
                        &mut (*(frame as *mut Self::PageTable)),
                        memory_map,
                        new_memory_state,
                        physical_memory_size,
                        remaining_page_table_levels,
                    ),
                    finished: false,
                }
            }
            None => IdentityMapEntryResult {
                memory_state: new_memory_state,
                finished: true,
            },
        }
    }

    unsafe fn identity_map(
        &mut self,
        page_table: &'a mut Self::PageTable,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
        remaining_page_table_levels: usize,
        physical_memory_size: usize,
    ) -> MemoryState {
        // If this is a L1 page table the delegate to populate_as_l1_kernel_page_table
        if remaining_page_table_levels == 0 {
            MemoryState {
                virtual_memory_size: page_table.populate_as_l1_kernel_page_table(
                    self.kernel_page_size(),
                    memory_state.virtual_memory_size,
                ),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            }
        } else {
            let mut new_memory_state = memory_state;
            let mut finished = false;
            // Populate unpopulated entries
            for entry in page_table.iter() {
                if finished {
                    entry.mark_unused();
                } else {
                    let identity_map_result = self.identity_map_entry(
                        entry,
                        memory_map,
                        memory_state,
                        remaining_page_table_levels - 1,
                        physical_memory_size,
                    );
                    finished = identity_map_result.finished;
                    new_memory_state = identity_map_result.memory_state;
                }
            }
            new_memory_state
        }
    }

    // Set up page tables so virtual address and physical address are the same
    unsafe fn identity_map_with_offset(
        &mut self,
        page_table: &'a mut Self::PageTable,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
        physical_memory_size: usize,
        page_table_offsets: &[usize],
    ) -> MemoryState {
        // If this is a L1 page table then delegate to populate_as_l1_kernel_page_table
        if page_table_offsets.is_empty() {
            MemoryState {
                virtual_memory_size: page_table.populate_as_l1_kernel_page_table(
                    self.kernel_page_size(),
                    memory_state.virtual_memory_size,
                ),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            }
        } else {
            // If some entries have already been populated then recurse into the last entry to make
            // sure that it is fully populated
            let offset = page_table_offsets[0];
            let remaining_page_table_levels = page_table_offsets.len();
            let new_offsets = &page_table_offsets[1..remaining_page_table_levels];
            let mut new_memory_state = if offset != 0 && !new_offsets.is_empty() {
                self.identity_map_with_offset(
                    &mut (*page_table.get_page_table(offset - 1)),
                    memory_map,
                    memory_state,
                    physical_memory_size,
                    new_offsets,
                )
            } else {
                memory_state
            };

            // Populate unpopulated entries
            let mut finished = false;
            for entry in page_table.iter().skip(offset) {
                if finished {
                    entry.mark_unused();
                } else {
                    let identity_map_result = self.identity_map_entry(
                        entry,
                        memory_map,
                        memory_state,
                        remaining_page_table_levels - 1,
                        physical_memory_size,
                    );
                    finished = identity_map_result.finished;
                    new_memory_state = identity_map_result.memory_state;
                }
            }
            new_memory_state
        }
    }
}

pub trait PageTable<'a>: Sized + 'a {
    const PAGE_SIZE: usize;

    type Entry: PageTableEntry;

    type EntryIterator: Iterator<Item = Self::Entry>;

    fn kernel_page_table_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn kernel_page_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn get_page_table(&mut self, index: usize) -> *mut Self;

    fn iter(&'a mut self) -> Self::EntryIterator;

    #[must_use]
    fn include_remnants_of_partially_used_pages(memory_region: Range<usize>) -> Range<usize> {
        (memory_region.start - memory_region.start % Self::PAGE_SIZE)..memory_region.end
    }

    fn populate_as_l1_kernel_page_table(
        &'a mut self,
        kernel_page_size: usize,
        virtual_memory_size: usize,
    ) -> usize {
        let mut address = virtual_memory_size;
        for entry in self.iter() {
            entry.set(address, Self::kernel_page_flags());
            address += kernel_page_size;
        }
        address
    }
}

pub trait PageTableEntry {
    type Flags;
    fn set(self, address: usize, flags: Self::Flags);
    fn mark_unused(self);
}

pub struct IdentityMapEntryResult {
    memory_state: MemoryState,
    finished: bool,
}

pub enum Error {
    MultibootHeaderLoad(MbiLoadError),
    NoMemoryMap,
    NoMemoryManager,
}

#[derive(Clone, Copy)]
pub struct MemoryState {
    pub virtual_memory_size: usize,
    pub last_frame_added_to_allocator: usize,
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
        smaller_allocator.add_aligned_frames(memory_region.start..first_page);
        self.add_frames(first_page..end_of_last_page);
        smaller_allocator.add_aligned_frames(end_of_last_page..end_of_last_page);
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

pub struct GetFrameResponse {
    pub frame: Option<usize>,
    pub last_frame_added_to_allocator: usize,
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

pub unsafe fn boot_os<'a, Proc: Architecture<'a> + 'a>(
    proc: &mut Proc,
    multiboot_info_ptr: u32,
) -> Result<(), Error> {
    // Initialize available memory and set up page tables
    let boot_info = BootInformation::load(multiboot_info_ptr as *const BootInformationHeader)
        .map_err(Error::MultibootHeaderLoad)?;

    boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let memory_map_tag = boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let mut physical_memory_size = 0;

    // Add free frames from first 4 GB to available frame list
    let available_memory_regions = unused_memory_regions(
        addr_of!(header_start) as usize..addr_of!(kernel_end) as usize,
        &boot_info,
        Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
    );
    for memory_area in available_memory_areas(memory_map_tag) {
        physical_memory_size = max(physical_memory_size, memory_area_end(memory_area));
        for memory_region in unused_memory_regions_from_area(memory_area, &available_memory_regions)
        {
            proc.register_memory_region(memory_region);
        }
    }

    // Set up memory past 4 GB
    let page_table_indices = proc.initial_page_table_counts();
    let new_memory_state = proc.identity_map_with_offset(
        &mut (*Proc::get_root_page_table()),
        memory_map_tag,
        MemoryState {
            virtual_memory_size: Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
            last_frame_added_to_allocator: Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
        },
        physical_memory_size,
        page_table_indices,
    );
    proc.register_available_memory_areas_from_region(
        memory_map_tag,
        new_memory_state.last_frame_added_to_allocator..new_memory_state.virtual_memory_size,
    );

    boot_info
        .module_tags()
        .find(|module| {
            if let Ok(command) = module.cmdline() {
                command.contains("memory_manager")
            } else {
                false
            }
        })
        .ok_or(Error::NoMemoryManager)?;

    // Reclaim memory used by boot info struct
    proc.register_memory_region(Proc::PageTable::include_remnants_of_partially_used_pages(
        boot_info.start_address()..boot_info.end_address(),
    ));

    Ok(())
}

extern "C" {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
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

fn intersect(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    max(a.start, b.start)..min(a.end, b.end)
}

fn unused_memory_regions_from_area<'a>(
    memory_area: &'a MemoryArea,
    unused_memory_regions: &'a [Range<usize>],
) -> impl Iterator<Item = Range<usize>> + 'a {
    let area = memory_area_start(memory_area)..memory_area_end(memory_area);
    unused_memory_regions
        .iter()
        .map(move |region| intersect(area.clone(), region.clone()))
        .filter(|region| !region.is_empty())
}

fn unused_memory_regions(
    kernel_memory: Range<usize>,
    boot_info: &BootInformation,
    max_address: usize,
) -> [Range<usize>; 3] {
    let boot_info_start = boot_info.start_address();
    if kernel_memory.start < boot_info_start {
        [
            0..kernel_memory.start,
            kernel_memory.end..boot_info_start,
            boot_info.end_address()..max_address,
        ]
    } else {
        [
            0..boot_info_start,
            boot_info.end_address()..kernel_memory.start,
            kernel_memory.end..max_address,
        ]
    }
}

fn available_memory_areas(memory_map: &MemoryMapTag) -> impl Iterator<Item = &MemoryArea> {
    memory_map.memory_areas().iter().filter(|area| {
        area.typ() == MemoryAreaType::Available || area.typ() == MemoryAreaType::AcpiAvailable
    })
}
