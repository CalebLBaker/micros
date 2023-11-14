mod apic;

use crate::{
    boot_os, end_of_last_full_page, first_full_page_address, Architecture, Error, FrameAllocator,
    GetFrameResponse, MemoryMapTag, MemoryState,
};
use apic::{
    error_interrupt_handler, spurious_interrupt_handler, timer_interrupt_handler, InterruptIndex,
};
use core::{
    ops::Range,
    ptr::{addr_of, addr_of_mut},
};
use x86_64::{
    addr::PhysAddr,
    instructions::{hlt, interrupts, tables::load_tss},
    registers::segmentation::{Segment, SegmentSelector, CS},
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
        paging::page_table::{PageTable, PageTableEntry, PageTableFlags},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

pub enum OsError {
    Apic(&'static str),
    Generic(Error),
}

pub unsafe fn run_operating_system(multiboot_info_ptr: u32, cpu_info: u32) -> Result<(), OsError> {
    static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];
    let segment_selectors = load_gdt(&mut GDT, &mut TSS, VirtAddr::from_ptr(&DOUBLE_FAULT_STACK));
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

    boot_os(
        &mut if supports_gigabyte_pages(cpu_info) {
            let mut four_kilobyte_pages = FrameAllocator { next: None };
            four_kilobyte_pages.add_frame(addr_of!(p2_tables[0]) as usize);
            four_kilobyte_pages.add_frame(addr_of!(p2_tables[1]) as usize);
            Amd64 {
                four_kilobyte_pages,
                two_megabyte_pages: FrameAllocator { next: None },
                gigabyte_pages: Some(FrameAllocator { next: None }),
            }
        } else {
            Amd64 {
                four_kilobyte_pages: FrameAllocator { next: None },
                two_megabyte_pages: FrameAllocator { next: None },
                gigabyte_pages: None,
            }
        },
        multiboot_info_ptr,
    )
    .map_err(OsError::Generic)
}

pub fn halt() -> ! {
    loop {
        hlt();
    }
}

extern "C" {
    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _: u64) -> ! {
    panic!("Double Fault\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    halt();
}

const FOUR_KILOBYTES: usize = 0x1000;
const TWO_MEGABYTES: usize = 0x20_0000;
const GIGABYTE: usize = 0x4000_0000;

const GIGABYTE_PAGES_CPUID_BIT: u32 = 0x400_0000;

const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = FOUR_KILOBYTES;

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

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

    type PageTable = PageTable;

    unsafe fn get_root_page_table() -> *mut PageTable {
        addr_of_mut!(p4_table)
    }

    unsafe fn register_memory_region(&mut self, memory_region: Range<usize>) {
        if let Some(ref mut gb_allocator) = self.gigabyte_pages {
            let first_gb_page = first_full_page_address(memory_region.start, GIGABYTE);
            let end_of_last_gb_page = end_of_last_full_page(memory_region.end, GIGABYTE);
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
        } else {
            self.two_megabyte_pages
                .add_aligned_frames_with_scrap_allocator(
                    &mut self.four_kilobyte_pages,
                    memory_region,
                );
        }
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

impl super::super::PageTableEntry for PageTableEntry {
    type Flags = PageTableFlags;

    fn set(&mut self, address: usize, flags: PageTableFlags) {
        self.set_addr(PhysAddr::new_truncate(address as u64), flags);
    }

    fn mark_unused(&mut self) {
        self.set_unused();
    }
}

impl<'a> super::super::PageTable<'a> for PageTable {
    const PAGE_SIZE: usize = FOUR_KILOBYTES;

    type Entry = PageTableEntry;
    type EntryIterator = impl Iterator<Item = &'a mut PageTableEntry>;

    fn iter_mut(&'a mut self) -> Self::EntryIterator {
        self.iter_mut()
    }
    fn get_page_table(&mut self, index: usize) -> *mut Self {
        self[index].addr().as_u64() as *mut PageTable
    }

    fn kernel_page_table_flags() -> PageTableFlags {
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE
    }

    fn kernel_page_flags() -> PageTableFlags {
        Self::kernel_page_table_flags() | PageTableFlags::HUGE_PAGE
    }
}

struct SegmentSelectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

fn supports_gigabyte_pages(cpu_info: u32) -> bool {
    (cpu_info & GIGABYTE_PAGES_CPUID_BIT) != 0
}

fn load_gdt(
    gdt: &'static mut GlobalDescriptorTable,
    tss: &'static mut TaskStateSegment,
    double_fault_stack: VirtAddr,
) -> SegmentSelectors {
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
        double_fault_stack + DOUBLE_FAULT_STACK_SIZE;
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
