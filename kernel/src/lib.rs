#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(type_alias_impl_trait)]

use display_daemon;
use core::fmt::Write;

mod arch;

use arch::x86_64 as proc;

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32) -> ! {
    let mut frame_allocator = FrameAllocator{ next: None, };
    proc::init();
    let _ = write!(display_daemon::WRITER.lock(), "address: {}\n", multiboot_info_ptr);

    // Initialize available memory and set up page tables
    let boot_info = unsafe { multiboot2::load(multiboot_info_ptr as usize) };
    if let Some(memory_map_tag) = boot_info.memory_map_tag() {

        let mut physical_memory_size = 0;
        let kernel_start = unsafe { &header_start } as *const u8 as usize;
        let kernel_end_addr = unsafe { &kernel_end } as *const u8 as usize;
        let boot_info_start = boot_info.start_address();
        let initial_num_pages = proc::ENTRIES_PER_PAGE_TABLE * proc::INITIAL_NUM_PAGE_TABLES;
        let virtual_memory_size = proc::KERNEL_PAGE_SIZE * initial_num_pages;

        // Add free frames from first 4 GB to available frame list
        for memory_area in available_memory_areas(memory_map_tag) {
            let mut region_end = memory_area.end_address() as usize % proc::PAGE_SIZE;
            physical_memory_size = max(physical_memory_size, region_end);
            region_end = min(region_end, virtual_memory_size);
            let region_start = first_full_page_address(memory_area.start_address());
            for page in (region_start .. region_end).step_by(proc::PAGE_SIZE) {
                // Make sure this page isn't part of the kernel
                let doesnt_overlap_boot_info = doesnt_overlap(page, boot_info_start, boot_info.end_address());
                if doesnt_overlap(page, kernel_start, kernel_end_addr) && doesnt_overlap_boot_info {
                    let page_ptr = page as *mut FrameAllocator;
                    unsafe {
                        (*page_ptr).next = frame_allocator.next;
                        frame_allocator.next = Some(&mut *page_ptr);
                    }
                }
            }
        }
        
        // Set up memory past 4 GB
        let mut page_table_indices = [1; proc::KERNEL_PAGE_TABLE_DEPTH - 1];
        let depth = proc::KERNEL_PAGE_TABLE_DEPTH - 2;
        page_table_indices[depth] = proc::INITIAL_NUM_PAGE_TABLES;
        let root_page_table = unsafe { &mut *proc::get_root_page_table() };
        let memory_state = MemoryState{
            max_available_address: virtual_memory_size,
            virtual_memory_size: virtual_memory_size,
        };
        let new_memory_state = root_page_table.identity_map(&mut frame_allocator, &memory_map_tag, memory_state, physical_memory_size, &mut page_table_indices);
        frame_allocator.add_frames(&memory_map_tag, new_memory_state);
    }
    let _ = display_daemon::WRITER.lock().write_str("Everything seems to be working . . . \n");
    proc::halt()
}

trait PageTableEntry {
    fn set(&mut self, address: usize, flags: proc::PageTableFlags);
}

trait PageTable<'a> {
    
    type EntryIterator : Iterator<Item = &'a mut proc::PageTableEntry>;

    fn identity_map(&'a mut self, frame_allocator: &mut FrameAllocator, memory_map: &multiboot2::MemoryMapTag, memory_state: MemoryState, physical_memory_size: usize, page_table_indices: &mut[usize]) -> MemoryState {
        if page_table_indices.is_empty() {
            let mut address = memory_state.virtual_memory_size;
            for entry in self.iter_mut() {
                entry.set(address, proc::kernel_page_flags());
                address += proc::KERNEL_PAGE_SIZE;
            }
            MemoryState { max_available_address: memory_state.max_available_address, virtual_memory_size: address }
        }
        else {
            let index = page_table_indices[0];
            let num_new_indicies = page_table_indices.len() - 1;
            let new_indices = &mut page_table_indices[1 .. num_new_indicies];
            let mut new_memory_state = if index != 0  && !new_indices.is_empty() {
                let page_table_ptr = self.get_page_table(index - 1);
                let page_table = unsafe { &mut *page_table_ptr };
                page_table.identity_map(frame_allocator, &memory_map, memory_state, physical_memory_size, new_indices)
            }
            else {
                memory_state
            };
            for entry in self.iter_mut().skip(index) {
                if new_memory_state.virtual_memory_size > physical_memory_size {
                    return new_memory_state;
                }
                if let (Some(frame_addr), new_max_available_address) = frame_allocator.get_frame_add_if_needed(&memory_map, new_memory_state) {
                    new_memory_state.max_available_address = new_max_available_address;
                    entry.set(frame_addr, proc::kernel_page_table_flags());
                    let frame_ptr = frame_addr as *mut proc::PageTable;
                    let frame = unsafe { &mut *frame_ptr };
                    new_memory_state = frame.identity_map(frame_allocator, &memory_map, new_memory_state, physical_memory_size, new_indices);
                }
                else {
                    return new_memory_state;
                }
            }
            page_table_indices[0] = 0;
            new_memory_state
        }
    }

    fn get_page_table(&mut self, index: usize) -> *mut Self;

    fn iter_mut(&'a mut self) -> Self::EntryIterator;
}

#[derive(Clone, Copy)]
struct MemoryState {
    max_available_address: usize,
    virtual_memory_size: usize,
}

struct FrameAllocator {
    next: Option<*mut FrameAllocator>,
}

impl FrameAllocator {
    fn add_frames(&mut self, memory_map: &multiboot2::MemoryMapTag, memory_state: MemoryState) {
        for memory_area in available_memory_areas(memory_map) {
            let end_address = memory_area.end_address() as usize;
            let region_end = min(end_address % proc::PAGE_SIZE, memory_state.virtual_memory_size);
            let region_start_pre_clamp = first_full_page_address(memory_area.start_address());
            let region_start = max(region_start_pre_clamp, memory_state.max_available_address);
            for page in (region_start .. region_end).step_by(proc::PAGE_SIZE) {
                // Make sure this page isn't part of the kernel
                let page_ptr = page as *mut FrameAllocator;
                unsafe {
                    (*page_ptr).next = self.next;
                    self.next = Some(&mut *page_ptr);
                }
            }
        }
    }

    fn get_frame_add_if_needed(&mut self, memory_map: &multiboot2::MemoryMapTag, memory_state: MemoryState) -> (Option<usize>, usize) {
        if let Some(ret) = self.next {
            self.next = unsafe { &mut *ret }.next;
            (Some(ret as usize), memory_state.max_available_address)
        }
        else {
            self.add_frames(memory_map, memory_state);
            (None, memory_state.virtual_memory_size)
        }
    }
}

extern {
    // These aren't real variables. We just need the address of the start and end of the kernel
    static header_start: u8;
    static kernel_end: u8;
}

fn max(x: usize, y: usize) -> usize { if x > y { x } else { y } }

fn min(x: usize, y: usize) -> usize { if x < y { x } else { y } }

fn available_memory_areas(memory_map: &multiboot2::MemoryMapTag) -> impl Iterator<Item = &multiboot2::MemoryArea> {
    memory_map.all_memory_areas().filter(|area| {
        area.typ() == multiboot2::MemoryAreaType::Available || area.typ() == multiboot2::MemoryAreaType::AcpiAvailable
    })
}

fn doesnt_overlap(frame_start: usize, region_start: usize, region_end: usize) -> bool {
    frame_start > region_end && frame_start + proc::PAGE_SIZE < region_start
}

fn first_full_page_address(start_address: u64) -> usize {
    let start = start_address as usize;
    let page_offset = start % proc::PAGE_SIZE;
    if page_offset == 0 {
        start
    } else {
        start + proc::PAGE_SIZE - page_offset
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = write!(display_daemon::WRITER.lock(), "{}", info);
    proc::halt()
}

