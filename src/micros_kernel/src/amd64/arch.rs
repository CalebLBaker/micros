use crate::{
    Architecture, SegmentFlags,
    amd64::{elf, p4_table},
    copy_and_zero_fill, slice_with_bounds_check,
};
use core::{
    iter::{Skip, Take},
    ops::{BitOr, Index, IndexMut, Range},
    ptr::addr_of,
    slice,
};
use elf::ProgramHeader;
use frame_allocation::{
    FfiOption, FrameAllocator,
    amd64::{Amd64FrameAllocator, FOUR_KILOBYTES, GIGABYTE},
    end_of_last_full_page, first_full_page_address,
};

pub static mut PROC: Amd64 = Amd64 {
    allocator: Amd64FrameAllocator {
        four_kilobyte_pages: FrameAllocator::new(),
        two_megabyte_pages: FrameAllocator::new(),
        gigabyte_pages: FfiOption::None,
    },
};

pub struct Amd64 {
    pub allocator: Amd64FrameAllocator,
}

impl Amd64 {
    // This code is explicitly only enabled for 64 bit processors, so casting from u64 to usize is
    // safe here.
    #[allow(clippy::cast_possible_truncation)]
    unsafe fn copy_into_address_space(
        &mut self,
        page_table_level: u8,
        page_table: &mut PageTable,
        mut address: usize,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()> {
        let mut data_offset = 0;
        for entry in page_table.entries(page_table_level, address, size) {
            let page = if entry.is_empty() {
                let page_address = unsafe { self.allocator.get_4k_frame() }?;
                set_page_table_entry(entry, page_address, flags);
                unsafe { (page_address as *mut u8).write_bytes(0, FOUR_KILOBYTES) };
                page_address
            } else {
                update_page_table_entry_flags(entry, flags);
                entry.address() as usize
            };
            let page_offset = offset_in_page(page_table_level, address);
            let bytes_for_page =
                number_of_bytes_for_page(page_table_level, page_offset, size, data_offset);
            let data_for_entry = slice_with_bounds_check(data, data_offset, bytes_for_page);

            if page_table_level == 0 || entry.has_flags(PageTableFlags::HUGE_PAGE) {
                copy_and_zero_fill(
                    unsafe {
                        slice::from_raw_parts_mut((page + page_offset) as *mut u8, bytes_for_page)
                    },
                    data_for_entry,
                );
            } else {
                unsafe {
                    let sub_page_table = &mut *(page as *mut PageTable);
                    self.copy_into_address_space(
                        page_table_level - 1,
                        sub_page_table,
                        address,
                        data_for_entry,
                        bytes_for_page,
                        flags,
                    )?;
                }
            }
            data_offset += bytes_for_page;
            address += bytes_for_page;
        }
        Some(())
    }
}

impl Architecture for Amd64 {
    const INITIAL_VIRTUAL_MEMORY_SIZE: usize = 0x1_0000_0000;

    type PageTable = PageTable;

    type ExecutableHeader = elf::Header;

    type SegmentHeader = ProgramHeader;

    unsafe fn initialize_memory_manager_page_tables(&mut self) -> Option<*mut Self::PageTable> {
        unsafe {
            let root_table_pointer = self.allocator.get_4k_frame()? as *mut PageTable;
            let root_table = &mut (*root_table_pointer);
            root_table.clear();
            root_table[0] = (*addr_of!(p4_table))[0];

            let p3_table_addr = self.allocator.get_4k_frame()?;
            let p3_table = p3_table_addr as *mut PageTable;
            let flags = user_accessible_page() | PageTableFlags::WRITABLE;
            set_last_entry(root_table, p3_table_addr, flags);

            let p2_table_addr = self.allocator.get_4k_frame()?;
            let p2_table = p2_table_addr as *mut PageTable;
            clear_and_set_last_entry(&mut *p3_table, p2_table_addr, flags);

            if let Some(huge_stack) = self.allocator.get_2mb_frame() {
                clear_and_set_last_entry(
                    &mut *p2_table,
                    huge_stack,
                    flags | PageTableFlags::HUGE_PAGE | PageTableFlags::NO_EXECUTE,
                );
            } else {
                let stack_flags = flags | PageTableFlags::NO_EXECUTE;
                let p1_table_addr = self.allocator.get_4k_frame()?;
                let p1_table = p1_table_addr as *mut PageTable;
                clear_and_set_last_entry(&mut *p2_table, p1_table_addr, flags);

                clear_and_set_last_entry(
                    &mut *p1_table,
                    self.allocator.get_4k_frame()?,
                    stack_flags,
                );
                set_entry(
                    &mut *p1_table,
                    0x1fd,
                    self.allocator.get_4k_frame()?,
                    stack_flags,
                );
                set_entry(
                    &mut *p1_table,
                    0x1fc,
                    self.allocator.get_4k_frame()?,
                    stack_flags,
                );
                set_entry(
                    &mut *p1_table,
                    0x1fb,
                    self.allocator.get_4k_frame()?,
                    stack_flags,
                );
            }

            let p1_table_addr = self.allocator.get_4k_frame()?;
            let p1_table = p1_table_addr as *mut PageTable;
            set_entry(
                &mut *p2_table,
                0x100,
                p1_table_addr,
                interrupt_stack_flags(),
            );

            set_last_entry(
                &mut *p1_table,
                self.allocator.get_4k_frame()?,
                interrupt_stack_flags(),
            );

            Some(root_table_pointer)
        }
    }

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>) {
        if let FfiOption::Some(ref mut gb_allocator) = self.allocator.gigabyte_pages {
            let first_gb_page = first_full_page_address(memory_region.start, GIGABYTE);
            let end_of_last_gb_page = end_of_last_full_page(memory_region.end, GIGABYTE);
            if end_of_last_gb_page > first_gb_page {
                unsafe {
                    self.allocator
                        .two_megabyte_pages
                        .add_aligned_frames_with_scrap_allocator(
                            &mut self.allocator.four_kilobyte_pages,
                            memory_region.start..first_gb_page,
                        );
                    gb_allocator.add_frames(first_gb_page..end_of_last_gb_page);
                    self.allocator
                        .two_megabyte_pages
                        .add_aligned_frames_with_scrap_allocator(
                            &mut self.allocator.four_kilobyte_pages,
                            end_of_last_gb_page..end_of_last_gb_page,
                        );
                }
                return;
            }
        }
        unsafe {
            self.allocator
                .two_megabyte_pages
                .add_aligned_frames_with_scrap_allocator(
                    &mut self.allocator.four_kilobyte_pages,
                    memory_region.clone(),
                );
        }
    }

    unsafe fn copy_into_address_space(
        &mut self,
        root_page_table: &mut Self::PageTable,
        address: usize,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()> {
        unsafe { self.copy_into_address_space(3, root_page_table, address, data, size, flags) }
    }
}

#[repr(C)]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    fn entries(
        &mut self,
        page_table_level: u8,
        base_address: usize,
        size: usize,
    ) -> Take<Skip<slice::IterMut<PageTableEntry>>> {
        let first_index = page_table_entry(page_table_level, base_address);
        self.entries
            .iter_mut()
            .skip(first_index)
            .take(page_table_entry(page_table_level, base_address + size - 1) + 1 - first_index)
    }

    fn clear(&mut self) {
        self.entries.fill(PageTableEntry { entry: 0 });
    }
}

impl Index<u16> for PageTable {
    type Output = PageTableEntry;
    fn index(&self, index: u16) -> &Self::Output {
        &self.entries[index as usize]
    }
}

impl IndexMut<u16> for PageTable {
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        &mut self.entries[index as usize]
    }
}

const PAGE_TABLE_ENTRY_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    pub const fn new(address: u64, flags: PageTableFlags) -> Self {
        Self {
            entry: address | flags.0,
        }
    }

    const fn is_empty(self) -> bool {
        self.entry == 0
    }

    const fn address(self) -> u64 {
        self.entry & PAGE_TABLE_ENTRY_ADDRESS_MASK
    }

    const fn has_flags(self, flags: PageTableFlags) -> bool {
        (self.entry & flags.0) == flags.0
    }
}

#[derive(Clone, Copy)]
pub struct PageTableFlags(u64);

impl PageTableFlags {
    pub const PRESENT: Self = Self(1);
    pub const WRITABLE: Self = Self(2);
    pub const NO_EXECUTE: Self = Self(0x8000_0000_0000_0000);
    const USER_ACCESSIBLE: Self = Self(4);
    const HUGE_PAGE: Self = Self(0x80);
}

impl BitOr for PageTableFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

fn set_page_table_entry(
    page_table_entry: &mut PageTableEntry,
    address: usize,
    segment_flags: SegmentFlags,
) {
    let mut page_flags = user_accessible_page();
    conditionally_add_flag(
        &mut page_flags,
        segment_flags.writable(),
        PageTableFlags::WRITABLE,
    );
    conditionally_add_flag(
        &mut page_flags,
        !segment_flags.executable(),
        PageTableFlags::NO_EXECUTE,
    );
    *page_table_entry = PageTableEntry::new(address as u64, page_flags);
}

fn update_page_table_entry_flags(
    page_table_entry: &mut PageTableEntry,
    segment_flags: SegmentFlags,
) {
    if segment_flags.writable() {
        page_table_entry.entry |= PageTableFlags::WRITABLE.0;
    }
    if segment_flags.executable() {
        page_table_entry.entry &= !PageTableFlags::NO_EXECUTE.0;
    }
}

const fn offset_in_page(page_table_level: u8, address: usize) -> usize {
    address & (page_size(page_table_level) - 1)
}

fn number_of_bytes_for_page(
    page_table_level: u8,
    page_offset: usize,
    size: usize,
    data_offset: usize,
) -> usize {
    (page_size(page_table_level) - page_offset).min(size - data_offset)
}

fn user_accessible_page() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE
}

fn clear_and_set_last_entry(page_table: &mut PageTable, address: usize, flags: PageTableFlags) {
    page_table.clear();
    set_last_entry(page_table, address, flags);
}

fn set_last_entry(page_table: &mut PageTable, address: usize, flags: PageTableFlags) {
    set_entry(page_table, 0x1ff, address, flags);
}

fn set_entry(page_table: &mut PageTable, index: u16, address: usize, flags: PageTableFlags) {
    page_table[index] = PageTableEntry::new(address as u64, flags);
}

fn interrupt_stack_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE
}

const fn page_table_entry(page_table_level: u8, address: usize) -> usize {
    (address & page_table_entry_mask(page_table_level))
        >> page_table_entry_offset_in_address(page_table_level)
}

fn conditionally_add_flag(flags: &mut PageTableFlags, condition: bool, new_flag: PageTableFlags) {
    if condition {
        flags.0 |= new_flag.0;
    }
}

const fn page_size(page_table_level: u8) -> usize {
    if page_table_level == 0 {
        FOUR_KILOBYTES
    } else {
        page_size(page_table_level - 1) << 9
    }
}

const fn page_table_entry_mask(page_table_level: u8) -> usize {
    if page_table_level == 0 {
        0x0000_0000_001f_f000
    } else {
        (page_table_entry_mask(page_table_level - 1) << 9) | 0x0000_0000_001f_f000
    }
}

const fn page_table_entry_offset_in_address(page_table_level: u8) -> u8 {
    12 + 9 * page_table_level
}
