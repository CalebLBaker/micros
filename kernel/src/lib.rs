#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![feature(abi_x86_interrupt)]

use core::{
    cmp::{max, min},
    fmt::Write,
    ops::Range,
    panic::PanicInfo,
    ptr::addr_of,
};
use display_daemon::WRITER;

use multiboot2::{
    BootInformation, BootInformationHeader, MbiLoadError, MemoryArea, MemoryAreaType, MemoryMapTag,
};

mod arch;

use arch::amd64;

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    match unsafe { amd64::run_operating_system(multiboot_info_ptr, cpu_info) } {
        Ok(()) => {
            let _ = WRITER
                .lock()
                .write_str("Everything seems to be working . . . \n");
        }
        Err(err) => {
            let _ = WRITER.lock().write_str(match err {
                amd64::OsError::Generic(Error::MultibootHeaderLoad(
                    MbiLoadError::IllegalAddress,
                )) => "Illegal multiboot info address",
                amd64::OsError::Generic(Error::MultibootHeaderLoad(
                    MbiLoadError::IllegalTotalSize(_),
                )) => "Illegal multiboot info size",
                amd64::OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::NoEndTag)) => {
                    "No multiboot info end tag"
                }
                amd64::OsError::Apic(err) => err,
                amd64::OsError::Generic(Error::NoMemoryMap) => {
                    "No memory map tag in multiboot information"
                }
            });
        }
    }
    amd64::halt()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let _ = write!(WRITER.lock(), "{info}");
    amd64::halt()
}

extern "C" {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
}

trait Architecture<'a>: Sized {
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
        entry: &'a mut <Self::PageTable as PageTable<'a>>::Entry,
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
            for entry in page_table.iter_mut() {
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
            for entry in page_table.iter_mut().skip(offset) {
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

trait PageTable<'a>: Sized + 'a {
    const PAGE_SIZE: usize;

    type Entry: PageTableEntry;

    type EntryIterator: Iterator<Item = &'a mut Self::Entry>
    where
        <Self as PageTable<'a>>::Entry: 'a;

    fn kernel_page_table_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn kernel_page_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn get_page_table(&mut self, index: usize) -> *mut Self;

    fn iter_mut(&'a mut self) -> Self::EntryIterator;

    fn include_remnants_of_partially_used_pages(memory_region: Range<usize>) -> Range<usize> {
        (memory_region.start - memory_region.start % Self::PAGE_SIZE)..memory_region.end
    }

    fn populate_as_l1_kernel_page_table(
        &'a mut self,
        kernel_page_size: usize,
        virtual_memory_size: usize,
    ) -> usize {
        let mut address = virtual_memory_size;
        for entry in self.iter_mut() {
            entry.set(address, Self::kernel_page_flags());
            address += kernel_page_size;
        }
        address
    }
}

trait PageTableEntry {
    type Flags;
    fn set(&mut self, address: usize, flags: Self::Flags);
    fn mark_unused(&mut self);
}

struct IdentityMapEntryResult {
    memory_state: MemoryState,
    finished: bool,
}

enum Error {
    MultibootHeaderLoad(MbiLoadError),
    NoMemoryMap,
}

#[derive(Clone, Copy)]
struct MemoryState {
    virtual_memory_size: usize,
    last_frame_added_to_allocator: usize,
}

struct FrameAllocator<const FRAME_SIZE: usize> {
    next: Option<*mut FrameAllocator<FRAME_SIZE>>,
}

impl<const MEMORY_FRAME_SIZE: usize> FrameAllocator<MEMORY_FRAME_SIZE> {
    const FRAME_SIZE: usize = MEMORY_FRAME_SIZE;

    unsafe fn add_frames(&mut self, memory_area: Range<usize>) {
        for frame in memory_area.step_by(Self::FRAME_SIZE) {
            self.add_frame(frame);
        }
    }

    unsafe fn get_frame(&mut self) -> Option<usize> {
        let ret = self.next?;
        self.next = (*ret).next;
        Some(ret as usize)
    }

    unsafe fn add_frame(&mut self, frame_address: usize) {
        let frame_ptr = frame_address as *mut Self;
        (*frame_ptr).next = self.next;
        self.next = Some(&mut *frame_ptr);
    }

    unsafe fn add_aligned_frames_with_scrap_allocator<const SMALLER_FRAME_SIZE: usize>(
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

struct GetFrameResponse {
    frame: Option<usize>,
    last_frame_added_to_allocator: usize,
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

fn first_full_page_address(start_address: usize, page_size: usize) -> usize {
    let page_offset = start_address % page_size;
    if page_offset == 0 {
        start_address
    } else {
        start_address + page_size - page_offset
    }
}

fn end_of_last_full_page(end_address: usize, page_size: usize) -> usize {
    end_address - end_address % page_size
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

unsafe fn boot_os<'a, Proc: Architecture<'a> + 'a>(
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
    // TODO: replace Amd64 with Proc once https://github.com/rust-lang/rust/issues/76560 is closed
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

    // Reclaim memory used by boot info struct
    proc.register_memory_region(Proc::PageTable::include_remnants_of_partially_used_pages(
        boot_info.start_address()..boot_info.end_address(),
    ));

    Ok(())
}

fn available_memory_areas(memory_map: &MemoryMapTag) -> impl Iterator<Item = &MemoryArea> {
    memory_map.memory_areas().iter().filter(|area| {
        area.typ() == MemoryAreaType::Available || area.typ() == MemoryAreaType::AcpiAvailable
    })
}
