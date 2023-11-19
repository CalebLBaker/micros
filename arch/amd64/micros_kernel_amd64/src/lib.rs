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
    ops::Range,
    panic::PanicInfo,
    ptr::{addr_of, addr_of_mut},
};
use micros_console_writer::WRITER;
use micros_kernel_common::{
    boot_os, end_of_last_full_page, first_full_page_address, Architecture, Error, FrameAllocator,
    GetFrameResponse, MemoryState, PageTable, PageTableEntry,
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
                OsError::Apic(err) => err,
                OsError::Generic(Error::NoMemoryMap) => {
                    "No memory map tag in multiboot information"
                }
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
    static mut p2_tables: [Amd64PageTable; 2];
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    let _ = WRITER.lock().write_str("breakpoint\n");
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _: u64) -> ! {
    panic!("Double Fault\n{:#?}", stack_frame);
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

extern "x86-interrupt" fn timer_interrupt_handler(stack_frame: InterruptStackFrame) {
    let _ = write!(
        WRITER.lock(),
        "timer: stack frame: {:?}\naddr: {:}\n",
        stack_frame,
        addr_of!(stack_frame) as usize
    );
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

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

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

    unsafe fn get_root_page_table() -> *mut Self::PageTable {
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

unsafe fn run_operating_system(multiboot_info_ptr: u32, cpu_info: u32) -> Result<(), OsError> {
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
