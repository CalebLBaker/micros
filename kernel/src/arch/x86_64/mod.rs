mod pic;

use lazy_static::lazy_static;
use core::fmt::Write;
use x86_64::structures;
use structures::idt;
// use core::ops::BitOr;

pub use structures::paging::page_table::PageTableFlags;
pub use structures::paging::page_table::PageTable;
pub use structures::paging::page_table::PageTableEntry;

pub const PAGE_SIZE : usize = 4 * 1024;

pub const ENTRIES_PER_PAGE_TABLE : usize = 512;
pub const INITIAL_NUM_PAGE_TABLES : usize = 4;
pub const KERNEL_PAGE_SIZE : usize = ENTRIES_PER_PAGE_TABLE * PAGE_SIZE;
pub const KERNEL_PAGE_TABLE_DEPTH : usize = 3;

pub fn kernel_page_table_flags() -> PageTableFlags { PageTableFlags::PRESENT | PageTableFlags::WRITABLE }

pub fn kernel_page_flags() -> PageTableFlags { kernel_page_table_flags() | PageTableFlags::HUGE_PAGE }

// pub const KERNEL_PAGE_TABLE_FLAGS : PageTableFlags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
// pub const KERNEL_PAGE_FLAGS : PageTableFlags = KERNEL_PAGE_TABLE_FLAGS | PageTableFlags::HUGE_PAGE;

pub fn init() {
    GDT.0.load();
    let code_selector = GDT.1.code_selector;
    let tss_selector = GDT.1.tss_selector;
    unsafe {
        x86_64::instructions::segmentation::set_cs(code_selector);
        x86_64::instructions::tables::load_tss(tss_selector);
    }
    IDT.load();
    unsafe {
        pic::init();
    }
    x86_64::instructions::interrupts::enable();
}

pub fn halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn get_root_page_table() -> *mut PageTable {
    unsafe { &mut p4_table as *mut PageTable }
}

impl super::super::PageTableEntry for PageTableEntry {
    fn set(&mut self, address: usize, flags: PageTableFlags) {
        self.set_addr(x86_64::addr::PhysAddr::new_truncate(address as u64), flags);
    }
}

impl<'a> super::super::PageTable<'a> for PageTable {
    type EntryIterator = impl Iterator<Item = &'a mut PageTableEntry>;
    fn iter_mut(&'a mut self) -> Self::EntryIterator { self.iter_mut() }
    fn get_page_table(&mut self, index: usize) -> *mut Self { self[index].addr().as_u64() as *mut PageTable }
}

struct Selectors {
    code_selector: structures::gdt::SegmentSelector,
    tss_selector: structures::gdt::SegmentSelector,
}

extern {
    static mut p4_table: PageTable;
}

const DOUBLE_FAULT_IST_INDEX: u16 = 0;

lazy_static! {
    static ref IDT: idt::InterruptDescriptorTable = {
        let mut idt = idt::InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        let double_fault_interrupt = idt.double_fault.set_handler_fn(double_fault_handler);
        unsafe {
            double_fault_interrupt.set_stack_index(DOUBLE_FAULT_IST_INDEX);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt[pic::InterruptIndex::Timer as usize].set_handler_fn(pic::timer_interrupt_handler);
        idt[pic::InterruptIndex::Keyboard as usize].set_handler_fn(pic::keyboard_interrupt_handler);
        idt
    };
}

lazy_static! {
    static ref TSS: structures::tss::TaskStateSegment = {
        let mut tss = structures::tss::TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 1024 * 4;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            let stack_start = x86_64::VirtAddr::from_ptr(unsafe { &STACK });
            let stack_end = stack_start + STACK_SIZE;
            stack_end
        };
        tss
    };
}

lazy_static! {
    static ref GDT: (structures::gdt::GlobalDescriptorTable, Selectors) = {
        let mut gdt = structures::gdt::GlobalDescriptorTable::new();
        let code_selector = gdt.add_entry(structures::gdt::Descriptor::kernel_code_segment());
        let tss_selector = gdt.add_entry(structures::gdt::Descriptor::tss_segment(&TSS));
        (gdt, Selectors { code_selector, tss_selector })
    };
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: &mut idt::InterruptStackFrame) {
    let _ = write!(display_daemon::WRITER.lock(), "Breakpoint hit\n{:#?}\n", stack_frame); 
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: &mut idt::InterruptStackFrame, _: u64) -> !{
    panic!("Double Fault\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(_stack_frame: &mut idt::InterruptStackFrame, _error_code: idt::PageFaultErrorCode) {
    let _virtual_address = x86_64::registers::control::Cr2::read();
    let _ = write!(display_daemon::WRITER.lock(), "Page Fault\n");
}

