#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![feature(abi_x86_interrupt)]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::struct_field_names)]

mod apic;

use apic::{end_interrupt, InterruptIndex};
use core::{
    fmt::Write,
    mem::size_of,
    ops::Range,
    panic::PanicInfo,
    ptr::{addr_of, addr_of_mut},
};
use micros_console_writer::WRITER;
use micros_kernel_common::{
    boot_os, end_of_last_full_page, first_full_page_address, Architecture, Error, ExecutableHeader,
    FrameAllocator, GetFrameResponse, MemoryState, PageTable, PageTableEntry, SegmentHeader,
};
use multiboot2::{MbiLoadError, MemoryMapTag};
use page_table::PageTableFlags;
use x86_64::{
    addr::PhysAddr,
    instructions::{hlt, interrupts, tables::load_tss},
    registers::segmentation::{Segment, SegmentSelector, CS},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
        paging::page_table,
        tss::TaskStateSegment,
    },
    VirtAddr,
};

#[no_mangle]
pub extern "C" fn main(multiboot_info_ptr: u32, cpu_info: u32) -> ! {
    match unsafe { run_operating_system(multiboot_info_ptr, cpu_info) } {
        Ok(()) => {
            let _ = WRITER
                .lock()
                .write_str("Everything seems to be working . . . \n");
        }
        Err(err) => {
            let _ = WRITER.lock().write_str(match err {
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::IllegalAddress)) => {
                    "Illegal multiboot info address"
                }
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::IllegalTotalSize(_))) => {
                    "Illegal multiboot info size"
                }
                OsError::Generic(Error::MultibootHeaderLoad(MbiLoadError::NoEndTag)) => {
                    "No multiboot info end tag"
                }
                OsError::Generic(Error::NoMemoryManager) => {
                    "Memory manager not loaded by boot loader"
                }
                OsError::Generic(Error::NoMemoryMap) => {
                    "No memory map tag in multiboot information"
                }
                OsError::Generic(Error::InvalidMemoryManagerModule) => {
                    "Could not load memory manager"
                }
                OsError::Generic(Error::AssertionError) => "An unexpected error occurred",
                OsError::Generic(Error::FailedToSetupMemoryManagerAddressSpace) => {
                    "Failed to setup memory manager address space"
                }
                OsError::Apic(err) => err,
            });
        }
    }
    halt()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let _ = write!(WRITER.lock(), "{info}");
    halt()
}

extern "C" {
    static mut p4_table: Amd64PageTable;
    static mut p2_tables: [page_table::PageTable; 2];
    static mut p1_table_for_stack: page_table::PageTable;
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("breakpoint\n");
}

extern "x86-interrupt" fn double_fault_handler(_stack_frame: InterruptStackFrame, _: u64) -> ! {
    let _ = WRITER.lock().write_str("double fault\n");
    halt();
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    let _ = WRITER.lock().write_str("page fault\n");
    halt();
}

extern "x86-interrupt" fn spurious_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("spurious\n");
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn error_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("error\n");
    unsafe {
        end_interrupt();
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("timer\n");
    unsafe {
        end_interrupt();
    }
}

const FOUR_KILOBYTES: usize = 0x1000;
const TWO_MEGABYTES: usize = 0x20_0000;
const GIGABYTE: usize = 0x4000_0000;

const GIGABYTE_PAGES_CPUID_BIT: u32 = 0x400_0000;

const DOUBLE_FAULT_IST_INDEX: u16 = 0;
const DOUBLE_FAULT_STACK_SIZE: usize = FOUR_KILOBYTES;

const DOUBLE_FAULT_STACK_BOTTOM: *mut u8 = 0xffff_ffff_ffe0_1000 as *mut u8;
const DOUBLE_FAULT_STACK_TOP: VirtAddr = VirtAddr::new_truncate(0xffff_ffff_ffe0_2000);

const ELF_MAGIC_NUMBER: u32 = 0x464c_457f;
const ELF_64_BIT: u8 = 2;
const ELF_LITTLE_ENDIAN: u8 = 1;
const ELF_EXECUTABLE: u16 = 2;
const ELF_X86_64: u16 = 0x3e;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack([0; DOUBLE_FAULT_STACK_SIZE]);

enum OsError {
    Apic(&'static str),
    Generic(Error),
}

struct Amd64 {
    four_kilobyte_pages: FrameAllocator<FOUR_KILOBYTES>,
    two_megabyte_pages: FrameAllocator<TWO_MEGABYTES>,
    gigabyte_pages: Option<FrameAllocator<GIGABYTE>>,
}

impl Amd64 {
    unsafe fn get_4k_frame(&mut self) -> Option<usize> {
        if let Some(frame) = self.four_kilobyte_pages.get_frame() {
            Some(frame)
        } else if let Some(frame) = self.get_2mb_frame() {
            self.four_kilobyte_pages
                .add_frames((frame + FOUR_KILOBYTES)..(frame + TWO_MEGABYTES));
            Some(frame)
        } else {
            None
        }
    }

    unsafe fn get_2mb_frame(&mut self) -> Option<usize> {
        if let Some(frame) = self.two_megabyte_pages.get_frame() {
            Some(frame)
        } else if let Some(frame) = self.gigabyte_pages.as_mut()?.get_frame() {
            self.two_megabyte_pages
                .add_frames((frame + TWO_MEGABYTES)..(frame + GIGABYTE));
            Some(frame)
        } else {
            None
        }
    }
}

impl<'a> Architecture<'a> for Amd64 {
    const INITIAL_VIRTUAL_MEMORY_SIZE: usize = 0x1_0000_0000;

    type PageTable = Amd64PageTable;

    type ExecutableHeader = ElfHeader;

    type SegmentHeader = ProgramHeader;

    unsafe fn get_root_page_table() -> *mut Self::PageTable {
        addr_of_mut!(p4_table)
    }

    unsafe fn initialize_memory_manager_page_tables(&mut self) -> Option<*mut Self::PageTable> {
        let root_table_pointer = self.get_4k_frame()? as *mut Self::PageTable;
        let root_table = &mut (*root_table_pointer).0;
        root_table[0] = (*Self::get_root_page_table()).0[0].clone();

        let p3_table_addr = self.get_4k_frame()?;
        let p3_table = p3_table_addr as *mut page_table::PageTable;
        let flags = Amd64PageTable::kernel_page_table_flags();
        set_last_entry(root_table, p3_table_addr, flags);

        let p2_table_addr = self.get_4k_frame()?;
        let p2_table = p2_table_addr as *mut page_table::PageTable;
        set_last_entry(&mut *p3_table, p2_table_addr, flags);

        if let Some(huge_stack) = self.get_2mb_frame() {
            set_last_entry(
                &mut *p2_table,
                huge_stack,
                Amd64PageTable::kernel_page_flags(),
            );
        } else {
            let p1_table_addr = self.get_4k_frame()?;
            let p1_table = p1_table_addr as *mut page_table::PageTable;
            set_last_entry(&mut *p2_table, p1_table_addr, flags);

            set_last_entry(&mut *p1_table, self.get_4k_frame()?, flags);
            set_entry(&mut *p1_table, 0x1fd, self.get_4k_frame()?, flags);
            set_entry(&mut *p1_table, 0x1fc, self.get_4k_frame()?, flags);
            set_entry(&mut *p1_table, 0x1fb, self.get_4k_frame()?, flags);
        }

        Some(root_table_pointer)
    }

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>) {
        if let Some(ref mut gb_allocator) = self.gigabyte_pages {
            let first_gb_page = first_full_page_address(memory_region.start, GIGABYTE);
            let end_of_last_gb_page = end_of_last_full_page(memory_region.end, GIGABYTE);
            if end_of_last_gb_page > first_gb_page {
                self.two_megabyte_pages
                    .add_aligned_frames_with_scrap_allocator(
                        &mut self.four_kilobyte_pages,
                        memory_region.start..first_gb_page,
                    );
                gb_allocator.add_frames(first_gb_page..end_of_last_gb_page);
                self.two_megabyte_pages
                    .add_aligned_frames_with_scrap_allocator(
                        &mut self.four_kilobyte_pages,
                        end_of_last_gb_page..end_of_last_gb_page,
                    );
                return;
            }
        }
        self.two_megabyte_pages
            .add_aligned_frames_with_scrap_allocator(
                &mut self.four_kilobyte_pages,
                memory_region.clone(),
            );
    }

    fn initial_page_table_counts(&self) -> &'static [usize] {
        if self.gigabyte_pages.is_some() {
            &[1]
        } else {
            &[1, 4]
        }
    }

    fn kernel_page_size(&self) -> usize {
        if self.gigabyte_pages.is_some() {
            GIGABYTE
        } else {
            TWO_MEGABYTES
        }
    }

    unsafe fn get_frame_for_page_table(
        &mut self,
        memory_map: &MemoryMapTag,
        memory_state: MemoryState,
    ) -> GetFrameResponse {
        if let Some(frame) = self.four_kilobyte_pages.get_frame() {
            GetFrameResponse {
                frame: Some(frame),
                last_frame_added_to_allocator: memory_state.last_frame_added_to_allocator,
            }
        } else {
            self.register_available_memory_areas_from_region(
                memory_map,
                memory_state.last_frame_added_to_allocator..memory_state.virtual_memory_size,
            );
            GetFrameResponse {
                frame: self.get_4k_frame(),
                last_frame_added_to_allocator: memory_state.virtual_memory_size,
            }
        }
    }
}

struct Amd64PageTableEntry<'a>(&'a mut page_table::PageTableEntry);

impl<'a> PageTableEntry for Amd64PageTableEntry<'a> {
    type Flags = PageTableFlags;

    fn set(self, address: usize, flags: PageTableFlags) {
        self.0
            .set_addr(PhysAddr::new_truncate(address as u64), flags);
    }

    fn mark_unused(self) {
        self.0.set_unused();
    }
}

#[repr(transparent)]
struct Amd64PageTable(page_table::PageTable);

impl<'a> PageTable<'a> for Amd64PageTable {
    const PAGE_SIZE: usize = FOUR_KILOBYTES;

    type Entry = Amd64PageTableEntry<'a>;
    type EntryIterator = impl Iterator<Item = Self::Entry>;

    fn iter(&'a mut self) -> Self::EntryIterator {
        self.0.iter_mut().map(Amd64PageTableEntry)
    }

    fn get_page_table(&mut self, index: usize) -> *mut Self {
        self.0[index].addr().as_u64() as *mut Self
    }

    fn kernel_page_table_flags() -> PageTableFlags {
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE
    }

    fn kernel_page_flags() -> PageTableFlags {
        Self::kernel_page_table_flags() | PageTableFlags::HUGE_PAGE
    }
}

struct SegmentSelectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

#[repr(C, align(0x1000))]
struct DoubleFaultStack([u8; DOUBLE_FAULT_STACK_SIZE]);

#[repr(C)]
struct ElfHeader {
    ident_magic: u32,
    ident_width_class: u8,
    ident_data_endianness: u8,
    ident_version: u8,
    ident_os_abi: u8,
    ident_abi_version: u8,
    ident_padding_0: u8,
    ident_padding_1: u8,
    ident_padding_2: u8,
    ident_padding_3: u8,
    ident_padding_4: u8,
    ident_padding_5: u8,
    ident_padding_6: u8,
    file_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    program_header_offset: u64,
    section_header_offset: u64,
    flags: u32,
    elf_header_size: u16,
    program_header_entry_size: u16,
    program_header_num: u16,
    section_header_entry_size: u16,
    section_header_num: u16,
    shstrndx: u16,
}

#[allow(clippy::cast_possible_truncation)]
impl ExecutableHeader for ElfHeader {
    fn is_valid(&self, file_size: usize) -> bool {
        size_of::<ElfHeader>() <= file_size
            && self.ident_magic == ELF_MAGIC_NUMBER
            && self.ident_width_class == ELF_64_BIT
            && self.ident_data_endianness == ELF_LITTLE_ENDIAN
            && self.ident_version == 1
            && self.file_type == ELF_EXECUTABLE
            && self.machine == ELF_X86_64
            && self.program_header_offset as usize
                + self.program_header_num as usize * size_of::<ProgramHeader>()
                <= file_size
    }

    fn num_segments(&self) -> usize {
        self.program_header_num as usize
    }

    fn segment_header_table_offset(&self) -> usize {
        self.program_header_offset as usize
    }
}

#[repr(C)]
struct ProgramHeader {
    segment_type: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    physical_address: u64,
    file_size: u64,
    memory_size: u64,
    align: u64,
}

#[allow(clippy::cast_possible_truncation)]
impl SegmentHeader for ProgramHeader {
    fn offset(&self) -> usize {
        self.offset as usize
    }

    fn segment_type(&self) -> u32 {
        self.segment_type
    }

    fn virtual_address(&self) -> usize {
        self.virtual_address as usize
    }

    fn file_size(&self) -> usize {
        self.file_size as usize
    }
}

fn set_entry(
    page_table: &mut page_table::PageTable,
    index: usize,
    address: usize,
    flags: PageTableFlags,
) {
    page_table[index].set_addr(PhysAddr::new_truncate(address as u64), flags);
}

fn set_last_entry(page_table: &mut page_table::PageTable, address: usize, flags: PageTableFlags) {
    set_entry(page_table, 0x1fe, address, flags);
}

fn kernel_stack_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE
}

unsafe fn run_operating_system(multiboot_info_ptr: u32, cpu_info: u32) -> Result<(), OsError> {
    p1_table_for_stack[0x001].set_addr(
        PhysAddr::new_truncate(addr_of!(DOUBLE_FAULT_STACK) as u64),
        kernel_stack_flags(),
    );

    let segment_selectors = load_gdt(&mut GDT, &mut TSS);
    CS::set_reg(segment_selectors.code_selector);
    load_tss(segment_selectors.tss_selector);
    IDT.breakpoint.set_handler_fn(breakpoint_handler);
    let double_fault_interrupt = IDT.double_fault.set_handler_fn(double_fault_handler);
    double_fault_interrupt.set_stack_index(DOUBLE_FAULT_IST_INDEX);
    IDT.page_fault.set_handler_fn(page_fault_handler);
    set_interrupt_handlers(&mut IDT);
    IDT.load();
    apic::init().map_err(OsError::Apic)?;
    interrupts::enable();

    // Without this line the double fault handler triggers a page fault and I have no idea why
    // I've tried flushing the translation lookaside buffer and that doesn't appear to have any
    // affect
    DOUBLE_FAULT_STACK_BOTTOM.write_volatile(0xff);

    boot_os(
        &mut if supports_gigabyte_pages(cpu_info) {
            let mut four_kilobyte_pages = FrameAllocator::default();
            four_kilobyte_pages.add_frame(addr_of!(p2_tables[0]) as usize);
            four_kilobyte_pages.add_frame(addr_of!(p2_tables[1]) as usize);
            Amd64 {
                four_kilobyte_pages,
                two_megabyte_pages: FrameAllocator::default(),
                gigabyte_pages: Some(FrameAllocator::default()),
            }
        } else {
            Amd64 {
                four_kilobyte_pages: FrameAllocator::default(),
                two_megabyte_pages: FrameAllocator::default(),
                gigabyte_pages: None,
            }
        },
        multiboot_info_ptr,
    )
    .map_err(OsError::Generic)
}

fn halt() -> ! {
    loop {
        hlt();
    }
}

fn supports_gigabyte_pages(cpu_info: u32) -> bool {
    (cpu_info & GIGABYTE_PAGES_CPUID_BIT) != 0
}

fn load_gdt(
    gdt: &'static mut GlobalDescriptorTable,
    tss: &'static mut TaskStateSegment,
) -> SegmentSelectors {
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = DOUBLE_FAULT_STACK_TOP;
    let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
    let tss_selector = gdt.add_entry(Descriptor::tss_segment(tss));
    gdt.load();
    SegmentSelectors {
        code_selector,
        tss_selector,
    }
}

fn set_interrupt_handlers(idt: &mut InterruptDescriptorTable) {
    idt[InterruptIndex::Timer as usize].set_handler_fn(timer_interrupt_handler);
    idt[InterruptIndex::Spurious as usize].set_handler_fn(spurious_interrupt_handler);
    idt[InterruptIndex::Error as usize].set_handler_fn(error_interrupt_handler);
}
