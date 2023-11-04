mod apic;

use crate::Architecture;
use apic::{
    error_interrupt_handler, spurious_interrupt_handler, timer_interrupt_handler, InterruptIndex,
};
use core::ptr::addr_of_mut;
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

pub struct Amd64 {
    _private_init: (),
}

impl<'a> Architecture<'a> for Amd64 {
    const ENTRIES_PER_PAGE_TABLE: usize = 512;
    const INITIAL_NUM_PAGE_TABLES: usize = 4;
    const KERNEL_PAGE_TABLE_DEPTH: usize = 3;

    type PageTable = PageTable;
    type Error = &'static str;

    unsafe fn init() -> Result<Self, Self::Error> {
        static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];
        let segment_selectors =
            load_gdt(&mut GDT, &mut TSS, VirtAddr::from_ptr(&DOUBLE_FAULT_STACK));
        CS::set_reg(segment_selectors.code_selector);
        load_tss(segment_selectors.tss_selector);
        IDT.breakpoint.set_handler_fn(breakpoint_handler);
        let double_fault_interrupt = IDT.double_fault.set_handler_fn(double_fault_handler);
        double_fault_interrupt.set_stack_index(DOUBLE_FAULT_IST_INDEX);
        IDT.page_fault.set_handler_fn(page_fault_handler);
        set_interrupt_handlers(&mut IDT);
        IDT.load();
        apic::init()?;
        interrupts::enable();
        Ok(Self { _private_init: () })
    }

    unsafe fn get_root_page_table(self) -> *mut PageTable {
        addr_of_mut!(p4_table)
    }

    fn halt() -> ! {
        loop {
            hlt();
        }
    }
}

impl super::super::PageTableEntry for PageTableEntry {
    type Flags = PageTableFlags;

    fn set(&mut self, address: usize, flags: PageTableFlags) {
        self.set_addr(PhysAddr::new_truncate(address as u64), flags);
    }
}

impl<'a> super::super::PageTable<'a> for PageTable {
    const PAGE_SIZE: usize = 4 * 1024;
    const KERNEL_PAGE_SIZE: usize = Amd64::ENTRIES_PER_PAGE_TABLE * Self::PAGE_SIZE;

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

const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 1024 * 4;

extern "C" {
    static mut p4_table: PageTable;
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

static mut TSS: TaskStateSegment = TaskStateSegment::new();

static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

struct SegmentSelectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _: u64) -> ! {
    panic!("Double Fault\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    Amd64::halt();
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
