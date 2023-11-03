#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![feature(abi_x86_interrupt)]

use core::{
    cmp::{max, min},
    fmt::Write,
    iter::StepBy,
    ops::Range,
    panic::PanicInfo,
};
use display_daemon::WRITER;

use multiboot2::{
    BootInformation, BootInformationHeader, MbiLoadError, MemoryArea, MemoryAreaType, MemoryMapTag,
};

mod arch;

use arch::amd64::Amd64;

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32) -> ! {
    match unsafe { boot_os::<Amd64>(multiboot_info_ptr) } {
        Ok(()) => {
            let _ = WRITER
                .lock()
                .write_str("Everything seems to be working . . . \n");
        }
        Err(err) => {
            let _ = WRITER.lock().write_str(match err {
                Error::MultibootHeaderLoad(MbiLoadError::IllegalAddress) => {
                    "Illegal multiboot info address"
                }
                Error::MultibootHeaderLoad(MbiLoadError::IllegalTotalSize(_)) => {
                    "Illegal multiboot info size"
                }
                Error::MultibootHeaderLoad(MbiLoadError::NoEndTag) => "No multiboot info end tag",
                Error::ArchitectureSpecific(err) => err,
                Error::NoMemoryMap => "No memory map tag in multiboot information",
            });
        }
    }
    Amd64::halt()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let _ = write!(WRITER.lock(), "{}", info);
    Amd64::halt()
}

extern "C" {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
}

trait Architecture<'a>: Sized {
    const KERNEL_PAGE_TABLE_DEPTH: usize;
    const INITIAL_NUM_PAGE_TABLES: usize;
    const ENTRIES_PER_PAGE_TABLE: usize;

    const INITIAL_VIRTUAL_MEMORY_SIZE: usize = Self::PageTable::KERNEL_PAGE_SIZE
        * Self::ENTRIES_PER_PAGE_TABLE
        * Self::INITIAL_NUM_PAGE_TABLES;

    type PageTable: PageTable<'a>;

    type Error;

    unsafe fn init() -> Result<Self, Self::Error>;

    unsafe fn get_root_page_table(self) -> *mut Self::PageTable;

    fn halt() -> !;
}

trait PageTable<'a>: Sized + 'a {
    const PAGE_SIZE: usize;
    const KERNEL_PAGE_SIZE: usize;

    type Entry: PageTableEntry;

    type EntryIterator: Iterator<Item = &'a mut Self::Entry>
    where
        <Self as PageTable<'a>>::Entry: 'a;

    fn kernel_page_table_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn kernel_page_flags() -> <Self::Entry as PageTableEntry>::Flags;

    fn get_page_table(&mut self, index: usize) -> *mut Self;

    fn iter_mut(&'a mut self) -> Self::EntryIterator;

    fn pages(start: usize, end: usize) -> StepBy<Range<usize>> {
        ((start - start % Self::PAGE_SIZE)..end).step_by(Self::PAGE_SIZE)
    }

    fn populate_as_l1_kernel_page_table(&'a mut self, virtual_memory_size: usize) -> usize {
        let mut address = virtual_memory_size;
        for entry in self.iter_mut() {
            entry.set(address, Self::kernel_page_flags());
            address += Self::KERNEL_PAGE_SIZE;
        }
        address
    }

    unsafe fn identity_map_entry(
        entry: &'a mut Self::Entry,
        frame_allocator: &mut FrameAllocator<'a, Self>,
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
        let get_frame_response = frame_allocator.get_frame_add_if_needed(memory_map, memory_state);
        let new_memory_state = MemoryState {
            virtual_memory_size: memory_state.virtual_memory_size,
            last_frame_added_to_allocator: get_frame_response.last_frame_added_to_allocator,
        };
        match get_frame_response.frame {
            Some(frame) => {
                entry.set(frame, Self::kernel_page_table_flags());
                IdentityMapEntryResult {
                    memory_state: (*(frame as *mut Self)).identity_map(
                        frame_allocator,
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
        &'a mut self,
        frame_allocator: &mut FrameAllocator<'a, Self>,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
        remaining_page_table_levels: usize,
        physical_memory_size: usize,
    ) -> MemoryState {
        // If this is a L1 page table the delegate to populate_as_l1_kernel_page_table
        if remaining_page_table_levels == 0 {
            MemoryState {
                virtual_memory_size: self
                    .populate_as_l1_kernel_page_table(memory_state.virtual_memory_size),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            }
        } else {
            let mut new_memory_state = memory_state;
            // Populate unpopulated entries
            for entry in self.iter_mut() {
                let identity_map_result = Self::identity_map_entry(
                    entry,
                    frame_allocator,
                    memory_map,
                    memory_state,
                    remaining_page_table_levels - 1,
                    physical_memory_size,
                );
                if identity_map_result.finished {
                    return identity_map_result.memory_state;
                } else {
                    new_memory_state = identity_map_result.memory_state
                }
            }
            new_memory_state
        }
    }

    // Set up page tables so virtual address and physical address are the same
    unsafe fn identity_map_with_offset(
        &'a mut self,
        frame_allocator: &mut FrameAllocator<'a, Self>,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
        physical_memory_size: usize,
        page_table_offsets: &[usize],
    ) -> MemoryState {
        // If this is a L1 page table the delegate to populate_as_l1_kernel_page_table
        if page_table_offsets.is_empty() {
            MemoryState {
                virtual_memory_size: self
                    .populate_as_l1_kernel_page_table(memory_state.virtual_memory_size),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            }
        } else {
            // If some entries have already been populated then recurse into the last entry to make
            // sure that it is fully populated
            let offset = page_table_offsets[0];
            let remaining_page_table_levels = page_table_offsets.len();
            let new_offsets = &page_table_offsets[1..remaining_page_table_levels];
            let mut new_memory_state = if offset != 0 && !new_offsets.is_empty() {
                (*self.get_page_table(offset - 1)).identity_map_with_offset(
                    frame_allocator,
                    memory_map,
                    memory_state,
                    physical_memory_size,
                    new_offsets,
                )
            } else {
                memory_state
            };

            // Populate unpopulated entries
            for entry in self.iter_mut().skip(offset) {
                let identity_map_result = Self::identity_map_entry(
                    entry,
                    frame_allocator,
                    memory_map,
                    memory_state,
                    remaining_page_table_levels - 1,
                    physical_memory_size,
                );
                if identity_map_result.finished {
                    return identity_map_result.memory_state;
                } else {
                    new_memory_state = identity_map_result.memory_state
                }
            }
            new_memory_state
        }
    }

    fn doesnt_overlap(frame_start: usize, region_start: usize, region_end: usize) -> bool {
        frame_start > region_end || frame_start + Self::PAGE_SIZE < region_start
    }

    fn first_full_page_address(start_address: u64) -> usize {
        let start = start_address as usize;
        let page_offset = start % Self::PAGE_SIZE;
        if page_offset == 0 {
            start
        } else {
            start + Self::PAGE_SIZE - page_offset
        }
    }
}

trait PageTableEntry {
    type Flags;
    fn set(&mut self, address: usize, flags: Self::Flags);
}

struct IdentityMapEntryResult {
    memory_state: MemoryState,
    finished: bool,
}

enum Error<ArchError> {
    ArchitectureSpecific(ArchError),
    MultibootHeaderLoad(MbiLoadError),
    NoMemoryMap,
}

#[derive(Clone, Copy)]
struct MemoryState {
    virtual_memory_size: usize,
    last_frame_added_to_allocator: usize,
}

struct FrameAllocator<'a, PageTableT: PageTable<'a>> {
    next: Option<*mut FrameAllocator<'a, PageTableT>>,
}

impl<'a, PageTableT: PageTable<'a>> FrameAllocator<'a, PageTableT> {
    unsafe fn add_frames(&mut self, memory_map: &MemoryMapTag, memory_state: MemoryState) {
        for page in available_pages_not_in_allocator::<PageTableT>(memory_map, memory_state) {
            self.add_frame(page);
        }
    }

    unsafe fn get_frame(&mut self) -> Option<usize> {
        let ret = self.next?;
        self.next = (*ret).next;
        Some(ret as usize)
    }

    unsafe fn get_frame_add_if_needed(
        &mut self,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
    ) -> GetFrameResponse {
        match self.get_frame() {
            Some(frame) => GetFrameResponse {
                frame: Some(frame),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            },
            None => {
                self.add_frames(memory_map, memory_state);
                GetFrameResponse {
                    frame: self.get_frame(),
                    last_frame_added_to_allocator: memory_state.virtual_memory_size,
                }
            }
        }
    }

    unsafe fn add_frame(&mut self, frame_address: usize) {
        let frame_ptr = frame_address as *mut Self;
        (*frame_ptr).next = self.next;
        self.next = Some(&mut *frame_ptr);
    }
}

struct GetFrameResponse {
    frame: Option<usize>,
    last_frame_added_to_allocator: usize,
}

fn unused_page_frames_from_initial_virtual_address_space<'a, 'b, Proc: Architecture<'a> + 'b>(
    memory_area: &'b MemoryArea,
    kernel_start: usize,
    kernel_end_addr: usize,
    boot_info: &'b BootInformation,
) -> impl Iterator<Item = usize> + 'b {
    (Proc::PageTable::first_full_page_address(memory_area.start_address())
        ..min(
            memory_area.end_address() as usize,
            Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
        ))
        .step_by(Proc::PageTable::PAGE_SIZE)
        .filter(move |page| {
            Proc::PageTable::doesnt_overlap(*page, kernel_start, kernel_end_addr)
                && Proc::PageTable::doesnt_overlap(
                    *page,
                    boot_info.start_address(),
                    boot_info.end_address(),
                )
        })
}

unsafe fn boot_os<'a, Proc: Architecture<'a> + 'a>(
    multiboot_info_ptr: u32,
) -> Result<(), Error<Proc::Error>> {
    let mut frame_allocator = FrameAllocator::<'a, Proc::PageTable> { next: None };

    // Initialize available memory and set up page tables
    let boot_info = BootInformation::load(multiboot_info_ptr as *const BootInformationHeader)
        .map_err(Error::MultibootHeaderLoad)?;

    let proc = Proc::init().map_err(Error::ArchitectureSpecific)?;

    boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let memory_map_tag = boot_info.memory_map_tag().ok_or(Error::NoMemoryMap)?;
    let mut physical_memory_size = 0;

    // Add free frames from first 4 GB to available frame list
    for memory_area in available_memory_areas(memory_map_tag) {
        physical_memory_size = max(physical_memory_size, memory_area.end_address() as usize);
        for page in unused_page_frames_from_initial_virtual_address_space::<Proc>(
            memory_area,
            &header_start as *const u8 as usize,
            &kernel_end as *const u8 as usize,
            &boot_info,
        ) {
            frame_allocator.add_frame(page);
        }
    }

    // Set up memory past 4 GB
    // TODO: replace Amd64 with Proc once https://github.com/rust-lang/rust/issues/76560 is closed
    let mut page_table_indices = [1; <Amd64 as Architecture>::KERNEL_PAGE_TABLE_DEPTH - 1];
    page_table_indices[Proc::KERNEL_PAGE_TABLE_DEPTH - 2] = Proc::INITIAL_NUM_PAGE_TABLES;
    let new_memory_state = (*proc.get_root_page_table()).identity_map_with_offset(
        &mut frame_allocator,
        memory_map_tag,
        MemoryState {
            virtual_memory_size: Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
            last_frame_added_to_allocator: Proc::INITIAL_VIRTUAL_MEMORY_SIZE,
        },
        physical_memory_size,
        &page_table_indices,
    );
    frame_allocator.add_frames(memory_map_tag, new_memory_state);

    // Reclaim memory used by boot info struct
    for frame in Proc::PageTable::pages(boot_info.start_address(), boot_info.end_address()) {
        frame_allocator.add_frame(frame);
    }

    Ok(())
}

fn available_pages_not_in_allocator<'a, PageTableT: PageTable<'a>>(
    memory_map: &MemoryMapTag,
    memory_state: MemoryState,
) -> impl Iterator<Item = usize> + '_ {
    available_memory_areas(memory_map).flat_map(move |memory_area| {
        let end_address = memory_area.end_address() as usize;
        (max(
            PageTableT::first_full_page_address(memory_area.start_address()),
            memory_state.last_frame_added_to_allocator,
        )
            ..min(
                if end_address % PageTableT::PAGE_SIZE == 0 {
                    end_address
                } else {
                    end_address + PageTableT::PAGE_SIZE
                },
                memory_state.virtual_memory_size,
            ))
            .step_by(PageTableT::PAGE_SIZE)
    })
}

fn available_memory_areas(memory_map: &MemoryMapTag) -> impl Iterator<Item = &MemoryArea> {
    memory_map.memory_areas().iter().filter(|area| {
        area.typ() == MemoryAreaType::Available || area.typ() == MemoryAreaType::AcpiAvailable
    })
}
