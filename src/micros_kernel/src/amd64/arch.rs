use crate::{ArchitectureKernel, amd64::p4_table, copy_and_zero_fill, slice_with_bounds_check};
use address::{AddressMapper, PhysicalAddress, VirtualAddress};
use architecture::amd64::Amd64;
use core::{
    iter::{Skip, Take},
    ops::{BitOr, Index, IndexMut, Range},
    slice,
};
use elf::{SegmentFlags, elf64};
use elf64::ProgramHeader;
use frame_allocation::{
    FfiOption, FrameAllocator, MemoryFrame,
    amd64::{Amd64FrameAllocator, FOUR_KILOBYTES, FourKbFrame, GbFrame},
};

pub static mut PROC: Amd64 = Amd64::new(
    Amd64FrameAllocator {
        four_kilobyte_pages: FrameAllocator::new(),
        two_megabyte_pages: FrameAllocator::new(),
        gigabyte_pages: FfiOption::None,
    },
    VirtualAddress::new(0),
    0,
);

unsafe fn copy_into_address_space(
    proc: &mut Amd64,
    page_table_level: u8,
    page_table: &mut PageTable,
    mut address: VirtualAddress,
    data: &[u8],
    size: usize,
    flags: SegmentFlags,
) -> Option<()> {
    let mut data_offset = 0;
    for entry in page_table.entries(page_table_level, address, size) {
        let page = if entry.is_empty() {
            let page_address = unsafe { proc.allocator.get_4k_frame() }?;
            set_page_table_entry(proc, entry, page_address, flags);
            unsafe { page_address.write_bytes(0, 1) };
            page_address
        } else {
            update_page_table_entry_flags(entry, flags);
            proc.physical_address_to_pointer::<FourKbFrame>(entry.address())
        };
        let page_offset = offset_in_page(page_table_level, address);
        let bytes_for_page =
            number_of_bytes_for_page(page_table_level, page_offset, size, data_offset);
        let data_for_entry = slice_with_bounds_check(data, data_offset, bytes_for_page);

        if page_table_level == 0 || entry.has_flags(PageTableFlags::HUGE_PAGE) {
            copy_and_zero_fill(
                unsafe {
                    slice::from_raw_parts_mut((page.cast::<u8>()).add(page_offset), bytes_for_page)
                },
                data_for_entry,
            );
        } else {
            unsafe {
                let sub_page_table = &mut *(page.cast::<PageTable>());
                copy_into_address_space(
                    proc,
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
        address = address + bytes_for_page;
    }
    Some(())
}

impl ArchitectureKernel for Amd64 {
    type PageTable = PageTable;

    type ExecutableHeader = elf64::Header;

    type SegmentHeader = ProgramHeader;

    unsafe fn initialize_memory_manager_page_tables(&mut self) -> Option<*mut Self::PageTable> {
        unsafe {
            let root_table_pointer = self.allocator.get_4k_frame()?.cast::<PageTable>();
            let root_table = &mut (*root_table_pointer);
            root_table.clear();
            let memory_index = page_table_entry(3, self.mapped_physical_memory_start());
            root_table[memory_index] = p4_table[memory_index];

            let p3_table_addr = self.allocator.get_4k_frame()?;
            let p3_table = p3_table_addr.cast::<PageTable>();
            let flags = user_accessible_page() | PageTableFlags::WRITABLE;
            set_last_entry(self, root_table, p3_table_addr, flags);

            let p2_table_addr = self.allocator.get_4k_frame()?;
            let p2_table = p2_table_addr.cast::<PageTable>();
            clear_and_set_last_entry(self, &mut *p3_table, p2_table_addr, flags);

            if let Some(huge_stack) = self.allocator.get_2mb_frame() {
                clear_and_set_last_entry(
                    self,
                    &mut *p2_table,
                    huge_stack,
                    flags | PageTableFlags::HUGE_PAGE | PageTableFlags::NO_EXECUTE,
                );
            } else {
                let stack_flags = flags | PageTableFlags::NO_EXECUTE;
                let p1_table_addr = self.allocator.get_4k_frame()?;
                let p1_table = p1_table_addr.cast::<PageTable>();
                clear_and_set_last_entry(self, &mut *p2_table, p1_table_addr, flags);

                let page = self.allocator.get_4k_frame()?;
                clear_and_set_last_entry(self, &mut *p1_table, page, stack_flags);
                let second_page = self.allocator.get_4k_frame()?;
                set_entry(self, &mut *p1_table, 0x1fd, second_page, stack_flags);
                let third_page = self.allocator.get_4k_frame()?;
                set_entry(self, &mut *p1_table, 0x1fc, third_page, stack_flags);
                let fourth_page = self.allocator.get_4k_frame()?;
                set_entry(self, &mut *p1_table, 0x1fb, fourth_page, stack_flags);
            }

            let p1_table_addr = self.allocator.get_4k_frame()?;
            let p1_table = p1_table_addr.cast::<PageTable>();
            set_entry(
                self,
                &mut *p2_table,
                0x100,
                p1_table_addr,
                interrupt_stack_flags(),
            );

            let page = self.allocator.get_4k_frame()?;
            set_last_entry(self, &mut *p1_table, page, interrupt_stack_flags());

            Some(root_table_pointer)
        }
    }

    unsafe fn register_memory_region(&mut self, memory_region: Range<*mut u8>) {
        if let FfiOption::Some(ref mut gb_allocator) = self.allocator.gigabyte_pages {
            let first_gb_page = GbFrame::first_full_page(memory_region.start);
            let end_of_last_gb_page = GbFrame::end_of_last_full_page(memory_region.end);
            if end_of_last_gb_page > first_gb_page {
                unsafe {
                    self.allocator
                        .two_megabyte_pages
                        .add_aligned_frames_with_scrap_allocator(
                            &mut self.allocator.four_kilobyte_pages,
                            memory_region.start..(first_gb_page.cast::<u8>()),
                        );
                    gb_allocator.add_frames(first_gb_page..end_of_last_gb_page);
                    self.allocator
                        .two_megabyte_pages
                        .add_aligned_frames_with_scrap_allocator(
                            &mut self.allocator.four_kilobyte_pages,
                            (end_of_last_gb_page.cast::<u8>())..memory_region.end,
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
        address: VirtualAddress,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()> {
        unsafe { copy_into_address_space(self, 3, root_page_table, address, data, size, flags) }
    }

    fn virtual_memory_end(&self) -> *const u8 {
        self.physical_address_to_pointer(PhysicalAddress::new(0x1_0000_0000))
    }
}

#[repr(C, align(0x1000))]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    fn entries(
        &mut self,
        page_table_level: u8,
        base_address: VirtualAddress,
        size: usize,
    ) -> Take<Skip<slice::IterMut<PageTableEntry>>> {
        let first_index = page_table_entry(page_table_level, base_address);
        self.entries.iter_mut().skip(first_index as usize).take(
            (page_table_entry(page_table_level, base_address + (size - 1)) + 1 - first_index)
                as usize,
        )
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
    pub const fn new(address: PhysicalAddress, flags: PageTableFlags) -> Self {
        Self {
            entry: address.address as u64 | flags.0,
        }
    }

    const fn is_empty(self) -> bool {
        self.entry == 0
    }

    const fn address(self) -> PhysicalAddress {
        PhysicalAddress::new((self.entry & PAGE_TABLE_ENTRY_ADDRESS_MASK) as usize)
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
    proc: &Amd64,
    page_table_entry: &mut PageTableEntry,
    address: *const FourKbFrame,
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
    *page_table_entry = PageTableEntry::new(proc.pointer_to_physical_address(address), page_flags);
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

fn offset_in_page(page_table_level: u8, address: VirtualAddress) -> usize {
    address.address & (page_size(page_table_level) - 1)
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

fn clear_and_set_last_entry<T: MemoryFrame>(
    proc: &Amd64,
    page_table: &mut PageTable,
    address: *const T,
    flags: PageTableFlags,
) {
    page_table.clear();
    set_last_entry(proc, page_table, address, flags);
}

fn set_last_entry<T: MemoryFrame>(
    proc: &Amd64,
    page_table: &mut PageTable,
    address: *const T,
    flags: PageTableFlags,
) {
    set_entry(proc, page_table, 0x1ff, address, flags);
}

fn set_entry<T: MemoryFrame>(
    proc: &Amd64,
    page_table: &mut PageTable,
    index: u16,
    address: *const T,
    flags: PageTableFlags,
) {
    page_table[index] = PageTableEntry::new(proc.pointer_to_physical_address(address), flags);
}

fn interrupt_stack_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE
}

// Result should always be <= 0x200 and so should fit in a u16
#[allow(clippy::cast_possible_truncation)]
fn page_table_entry(page_table_level: u8, address: VirtualAddress) -> u16 {
    ((address.address & page_table_entry_mask(page_table_level))
        >> page_table_entry_offset_in_address(page_table_level)) as u16
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
