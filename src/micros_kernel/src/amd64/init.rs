use crate::{
    amd64::{
        apic, breakpoint_handler, double_fault_handler, elf, error_interrupt_handler,
        launch_memory_manager, p1_table_for_stack, p2_tables, p4_table, page_fault_handler,
        spurious_interrupt_handler, timer_interrupt_handler,
    },
    boot_os, copy_and_zero_fill, slice_with_bounds_check, Architecture, SegmentFlags,
};
use apic::InterruptIndex;
use core::{
    ops::Range,
    ptr::{addr_of, addr_of_mut},
    slice,
};
use elf::ProgramHeader;
use frame_allocation::{
    amd64::{Amd64FrameAllocator, FOUR_KILOBYTES, GIGABYTE},
    end_of_last_full_page, first_full_page_address, FfiOption, FrameAllocator,
};
use x86_64::{
    addr::PhysAddr,
    instructions::{interrupts, tables::load_tss},
    registers::segmentation::{Segment, SegmentSelector, CS},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        idt::InterruptDescriptorTable,
        paging::page_table::{PageTable, PageTableEntry, PageTableFlags},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

pub unsafe fn initialize_operating_system(multiboot_info_ptr: u32, cpu_info: u32) -> Option<()> {
    p1_table_for_stack[0x001].set_addr(
        PhysAddr::new_truncate(addr_of!(DOUBLE_FAULT_STACK) as u64),
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
    );

    let segment_selectors = load_gdt(&mut *addr_of_mut!(GDT), &mut *addr_of_mut!(TSS));
    CS::set_reg(segment_selectors.code_selector);
    load_tss(segment_selectors.tss_selector);
    IDT.breakpoint.set_handler_fn(breakpoint_handler);
    let double_fault_interrupt = IDT.double_fault.set_handler_fn(double_fault_handler);
    double_fault_interrupt.set_stack_index(DOUBLE_FAULT_IST_INDEX);
    IDT.page_fault.set_handler_fn(page_fault_handler);
    set_interrupt_handlers(&mut *addr_of_mut!(IDT));
    IDT.load();
    apic::init()?;
    interrupts::enable();

    // Without this line the double fault handler triggers a page fault and I have no idea why
    // I've tried flushing the translation lookaside buffer and that doesn't appear to have any
    // affect
    DOUBLE_FAULT_STACK_BOTTOM.write_volatile(0xff);

    let proc = &mut *addr_of_mut!(PROC);
    if supports_gigabyte_pages(cpu_info) {
        proc.allocator
            .four_kilobyte_pages
            .add_frame(addr_of!(p2_tables[0]) as usize);
        proc.allocator
            .four_kilobyte_pages
            .add_frame(addr_of!(p2_tables[1]) as usize);
        proc.allocator.gigabyte_pages = FfiOption::Some(FrameAllocator::default());
    }
    let boot_info_ptr = multiboot_info_ptr as *const u8;
    let memory_manager_launch_info = boot_os(proc, boot_info_ptr)?;

    launch_memory_manager(
        addr_of_mut!(proc.allocator),
        boot_info_ptr,
        memory_manager_launch_info.root_page_table_address,
        memory_manager_launch_info.entry_point,
    );
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack([0; DOUBLE_FAULT_STACK_SIZE]);

static mut PROC: Amd64 = Amd64 {
    allocator: Amd64FrameAllocator {
        four_kilobyte_pages: FrameAllocator::new(),
        two_megabyte_pages: FrameAllocator::new(),
        gigabyte_pages: FfiOption::None,
    },
};

const GIGABYTE_PAGES_CPUID_BIT: u32 = 0x400_0000;

const DOUBLE_FAULT_IST_INDEX: u16 = 0;
const DOUBLE_FAULT_STACK_SIZE: usize = FOUR_KILOBYTES;

const DOUBLE_FAULT_STACK_BOTTOM: *mut u8 = 0xffff_ffff_ffe0_1000 as *mut u8;
const DOUBLE_FAULT_STACK_TOP: VirtAddr = VirtAddr::new_truncate(0xffff_ffff_ffe0_2000);

const INTERRUPT_STACK_BOTTOM: VirtAddr = VirtAddr::new_truncate(0xffff_ffff_fff0_1000);

struct Amd64 {
    allocator: Amd64FrameAllocator,
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
        for entry in page_table_entries(page_table, page_table_level, address, size) {
            let page = if entry.is_unused() {
                let page_address = self.allocator.get_4k_frame()?;
                set_page_table_entry(entry, page_address, flags);
                (page_address as *mut u8).write_bytes(0, FOUR_KILOBYTES);
                page_address
            } else {
                update_page_table_entry_flags(entry, flags);
                entry.addr().as_u64() as usize
            };
            let page_offset = offset_in_page(page_table_level, address);
            let bytes_for_page =
                number_of_bytes_for_page(page_table_level, page_offset, size, data_offset);
            let data_for_entry = slice_with_bounds_check(data, data_offset, bytes_for_page);

            if page_table_level == 0 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                copy_and_zero_fill(
                    slice::from_raw_parts_mut((page + page_offset) as *mut u8, bytes_for_page),
                    data_for_entry,
                );
            } else {
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
        let root_table_pointer = self.allocator.get_4k_frame()? as *mut PageTable;
        let root_table = &mut (*root_table_pointer);
        root_table.zero();
        root_table[0] = (*addr_of!(p4_table))[0].clone();

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

            clear_and_set_last_entry(&mut *p1_table, self.allocator.get_4k_frame()?, stack_flags);
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

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>) {
        if let FfiOption::Some(ref mut gb_allocator) = self.allocator.gigabyte_pages {
            let first_gb_page = first_full_page_address(memory_region.start, GIGABYTE);
            let end_of_last_gb_page = end_of_last_full_page(memory_region.end, GIGABYTE);
            if end_of_last_gb_page > first_gb_page {
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
                return;
            }
        }
        self.allocator
            .two_megabyte_pages
            .add_aligned_frames_with_scrap_allocator(
                &mut self.allocator.four_kilobyte_pages,
                memory_region.clone(),
            );
    }

    unsafe fn copy_into_address_space(
        &mut self,
        root_page_table: &mut Self::PageTable,
        address: usize,
        data: &[u8],
        size: usize,
        flags: SegmentFlags,
    ) -> Option<()> {
        self.copy_into_address_space(3, root_page_table, address, data, size, flags)
    }
}

struct SegmentSelectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

#[repr(C, align(0x1000))]
struct DoubleFaultStack([u8; DOUBLE_FAULT_STACK_SIZE]);

fn load_gdt(
    gdt: &'static mut GlobalDescriptorTable,
    tss: &'static mut TaskStateSegment,
) -> SegmentSelectors {
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = DOUBLE_FAULT_STACK_TOP;
    tss.privilege_stack_table[0] = INTERRUPT_STACK_BOTTOM;
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(tss));
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.load();
    SegmentSelectors {
        code_selector,
        tss_selector,
    }
}

fn supports_gigabyte_pages(cpu_info: u32) -> bool {
    (cpu_info & GIGABYTE_PAGES_CPUID_BIT) != 0
}

fn conditionally_add_flag(flags: &mut PageTableFlags, condition: bool, new_flag: PageTableFlags) {
    if condition {
        flags.insert(new_flag);
    }
}

fn user_accessible_page() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE
}

fn interrupt_stack_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE
}

fn set_entry(page_table: &mut PageTable, index: usize, address: usize, flags: PageTableFlags) {
    page_table[index].set_addr(PhysAddr::new_truncate(address as u64), flags);
}

fn clear_and_set_last_entry(page_table: &mut PageTable, address: usize, flags: PageTableFlags) {
    page_table.zero();
    set_last_entry(page_table, address, flags);
}

fn set_last_entry(page_table: &mut PageTable, address: usize, flags: PageTableFlags) {
    set_entry(page_table, 0x1ff, address, flags);
}

fn set_interrupt_handlers(idt: &mut InterruptDescriptorTable) {
    idt[InterruptIndex::Timer as u8].set_handler_fn(timer_interrupt_handler);
    idt[InterruptIndex::Spurious as u8].set_handler_fn(spurious_interrupt_handler);
    idt[InterruptIndex::Error as u8].set_handler_fn(error_interrupt_handler);
}

const fn page_size(page_table_level: u8) -> usize {
    if page_table_level == 0 {
        FOUR_KILOBYTES
    } else {
        page_size(page_table_level - 1) << 9
    }
}

const fn offset_in_page(page_table_level: u8, address: usize) -> usize {
    address & (page_size(page_table_level) - 1)
}

fn page_table_entries(
    page_table: &mut PageTable,
    page_table_level: u8,
    base_address: usize,
    size: usize,
) -> impl Iterator<Item = &mut PageTableEntry> {
    let first_index = page_table_entry(page_table_level, base_address);
    page_table
        .iter_mut()
        .skip(first_index)
        .take(page_table_entry(page_table_level, base_address + size - 1) + 1 - first_index)
}

fn number_of_bytes_for_page(
    page_table_level: u8,
    page_offset: usize,
    size: usize,
    data_offset: usize,
) -> usize {
    (page_size(page_table_level) - page_offset).min(size - data_offset)
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
    page_table_entry.set_addr(PhysAddr::new_truncate(address as u64), page_flags);
}

fn update_page_table_entry_flags(
    page_table_entry: &mut PageTableEntry,
    segment_flags: SegmentFlags,
) {
    let mut page_flags = page_table_entry.flags();
    conditionally_add_flag(
        &mut page_flags,
        segment_flags.writable(),
        PageTableFlags::WRITABLE,
    );
    if segment_flags.executable() {
        page_flags.remove(PageTableFlags::NO_EXECUTE);
    }
    page_table_entry.set_flags(page_flags);
}

const fn page_table_entry(page_table_level: u8, address: usize) -> usize {
    (address & page_table_entry_mask(page_table_level))
        >> page_table_entry_offset_in_address(page_table_level)
}

const fn page_table_entry_offset_in_address(page_table_level: u8) -> u8 {
    12 + 9 * page_table_level
}

const fn page_table_entry_mask(page_table_level: u8) -> usize {
    if page_table_level == 0 {
        0x0000_0000_001f_f000
    } else {
        (page_table_entry_mask(page_table_level - 1) << 9) | 0x0000_0000_001f_f000
    }
}
