use crate::{
    Architecture, SegmentFlags,
    amd64::{
        apic, breakpoint_handler, double_fault_handler, elf, enable_interrupts,
        error_interrupt_handler, launch_memory_manager, load_gdt, load_tss, p1_table_for_stack,
        p2_tables, p4_table, page_fault_handler, reset_code_segment, spurious_interrupt_handler,
        timer_interrupt_handler,
    },
    boot_os, copy_and_zero_fill, slice_with_bounds_check,
};
use apic::{InterruptIndex, LOCAL_APIC_END, LOCAL_APIC_START};
use core::{ops::Range, ptr, slice};
use elf::ProgramHeader;
use frame_allocation::{
    FfiOption, FrameAllocator,
    amd64::{Amd64FrameAllocator, FOUR_KILOBYTES, GIGABYTE},
    end_of_last_full_page, first_full_page_address,
};
use ptr::{addr_of, addr_of_mut};
use x86_64::{
    VirtAddr,
    addr::PhysAddr,
    structures::{
        idt::InterruptDescriptorTable,
        paging::page_table::{PageTable, PageTableEntry, PageTableFlags},
        tss::TaskStateSegment,
    },
};

#[repr(C, packed(2))]
pub struct GdtDescriptor {
    size: u16,
    offset: *const GlobalDescriptorTable,
}

// This code is explicitly only enabled for 64 bit processors, so casting from pointer to u64 is
// safe here.
#[allow(clippy::fn_to_numeric_cast)]
pub unsafe fn initialize_operating_system(multiboot_info_ptr: u32, cpu_info: u32) -> Option<()> {
    unsafe {
        p1_table_for_stack[0x001].set_addr(
            PhysAddr::new_truncate(addr_of!(DOUBLE_FAULT_STACK) as u64),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        );

        load_global_descriptor_table(&mut *addr_of_mut!(TSS));
        reset_code_segment();
        load_tss();
        let idt_ref = &mut *{ (&raw mut IDT) };
        idt_ref
            .breakpoint
            .set_handler_addr(VirtAddr::new(breakpoint_handler as u64));
        let double_fault_interrupt = idt_ref
            .double_fault
            .set_handler_addr(VirtAddr::new(double_fault_handler as u64));
        double_fault_interrupt.set_stack_index(DOUBLE_FAULT_IST_INDEX);
        idt_ref
            .page_fault
            .set_handler_addr(VirtAddr::new(page_fault_handler as u64));
        set_interrupt_handlers(idt_ref);
        idt_ref.load();
        apic::init();
        enable_interrupts();

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
        let memory_manager_launch_info =
            boot_os(proc, boot_info_ptr, LOCAL_APIC_START..LOCAL_APIC_END)?;

        launch_memory_manager(
            addr_of_mut!(proc.allocator),
            boot_info_ptr,
            memory_manager_launch_info.root_page_table_address,
            memory_manager_launch_info.entry_point,
        );
    }
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable {
    null: SegmentDescriptor::empty(),
    tss: SystemSegmentDescriptor::new(SegmentDescriptor::empty(), 0),
    kernel_code: SegmentDescriptor::empty(),
    user_data: SegmentDescriptor::empty(),
    user_code: SegmentDescriptor::empty(),
};

// size_of::<GlobalDescriptorTable>() is known to be 0x30, which is within the bounds for u16
#[allow(clippy::cast_possible_truncation)]
static mut GDTR: GdtDescriptor = GdtDescriptor {
    size: (size_of::<GlobalDescriptorTable>() - 1) as u16,
    offset: ptr::null(),
};

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

const DOUBLE_FAULT_STACK_TOP: VirtAddr = VirtAddr::new_truncate(0xffff_ffff_ffe0_2000);

const INTERRUPT_STACK_BOTTOM: VirtAddr = VirtAddr::new_truncate(0xffff_ffff_fff0_1000);

const LONG_MODE_TSS_SEGMENT_FLAG: u8 = 0x20;
const LONG_MODE_FLAT_SEGMENT_FLAG: u8 = 0x2f;
const TSS_ACCESS_BYTE: u8 = 0x89;
const KERNEL_CODE_SEGMENT_ACCESS_BYTE: u8 = 0x9a;
const USER_DATA_SEGMENT_ACCESS_BYTE: u8 = 0xf2;
const USER_CODE_SEGMENT_ACCESS_BYTE: u8 = 0xfa;

#[repr(C)]
struct SegmentDescriptor {
    limit_0_16: u16,
    base_0_16: u16,
    base_16_24: u8,
    access: u8,
    flags: u8,
    base_24_32: u8,
}

impl SegmentDescriptor {
    const fn new(access_byte: u8, flags: u8, base: u32, limit: u16) -> Self {
        Self::raw(
            access_byte,
            flags,
            (base & 0xffff) as u16,
            ((base & 0xff_0000) >> 16) as u8,
            ((base & 0xff00_0000) >> 24) as u8,
            limit,
        )
    }

    const fn raw(
        access_byte: u8,
        flags: u8,
        base_0_16: u16,
        base_16_24: u8,
        base_24_32: u8,
        limit: u16,
    ) -> Self {
        Self {
            limit_0_16: limit,
            base_0_16,
            base_16_24,
            access: access_byte,
            flags,
            base_24_32,
        }
    }

    const fn flat(access_byte: u8, flags: u8) -> Self {
        Self::raw(access_byte, flags, 0, 0, 0, 0xffff)
    }

    const fn empty() -> Self {
        Self::raw(0, 0, 0, 0, 0, 0)
    }
}

#[repr(C)]
struct SystemSegmentDescriptor {
    standard_descriptor: SegmentDescriptor,
    base_32_64: u32,
    reserved: u32,
}

impl SystemSegmentDescriptor {
    const fn new(standard_descriptor: SegmentDescriptor, base_32_64: u32) -> Self {
        Self {
            standard_descriptor,
            base_32_64,
            reserved: 0,
        }
    }
}

#[repr(C)]
struct GlobalDescriptorTable {
    null: SegmentDescriptor,
    kernel_code: SegmentDescriptor,
    tss: SystemSegmentDescriptor,
    user_data: SegmentDescriptor,
    user_code: SegmentDescriptor,
}

impl GlobalDescriptorTable {
    // size_of::<TaskStateSegment>() is known to be 0x68, which is within the allowed range for u16
    #[allow(clippy::cast_possible_truncation)]
    fn new(tss: *const TaskStateSegment) -> Self {
        Self {
            null: SegmentDescriptor::empty(),
            tss: SystemSegmentDescriptor::new(
                SegmentDescriptor::new(
                    TSS_ACCESS_BYTE,
                    LONG_MODE_TSS_SEGMENT_FLAG,
                    tss as u32,
                    (size_of::<TaskStateSegment>() - 1) as u16,
                ),
                ((tss as u64 & 0xffff_ffff_0000_0000) >> 32) as u32,
            ),
            kernel_code: SegmentDescriptor::flat(
                KERNEL_CODE_SEGMENT_ACCESS_BYTE,
                LONG_MODE_FLAT_SEGMENT_FLAG,
            ),
            user_data: SegmentDescriptor::flat(
                USER_DATA_SEGMENT_ACCESS_BYTE,
                LONG_MODE_FLAT_SEGMENT_FLAG,
            ),
            user_code: SegmentDescriptor::flat(
                USER_CODE_SEGMENT_ACCESS_BYTE,
                LONG_MODE_FLAT_SEGMENT_FLAG,
            ),
        }
    }
}

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
                let page_address = unsafe { self.allocator.get_4k_frame() }?;
                set_page_table_entry(entry, page_address, flags);
                unsafe { (page_address as *mut u8).write_bytes(0, FOUR_KILOBYTES) };
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

#[repr(C, align(0x1000))]
struct DoubleFaultStack([u8; DOUBLE_FAULT_STACK_SIZE]);

unsafe fn load_global_descriptor_table(tss: &'static mut TaskStateSegment) {
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = DOUBLE_FAULT_STACK_TOP;
    tss.privilege_stack_table[0] = INTERRUPT_STACK_BOTTOM;
    unsafe {
        GDT = GlobalDescriptorTable::new(ptr::from_ref(tss));
        GDTR.offset = addr_of!(GDT);
        load_gdt(addr_of!(GDTR));
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

// This code is explicitly only enabled for 64 bit processors, so casting from pointer to u64 is
// safe here.
#[allow(clippy::fn_to_numeric_cast)]
fn set_interrupt_handlers(idt: &mut InterruptDescriptorTable) {
    unsafe {
        idt[InterruptIndex::Timer as u8]
            .set_handler_addr(VirtAddr::new(timer_interrupt_handler as u64));
        idt[InterruptIndex::Spurious as u8]
            .set_handler_addr(VirtAddr::new(spurious_interrupt_handler as u64));
        idt[InterruptIndex::Error as u8]
            .set_handler_addr(VirtAddr::new(error_interrupt_handler as u64));
    }
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
