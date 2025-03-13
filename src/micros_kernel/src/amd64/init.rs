use crate::{
    amd64::{
        apic, arch::PROC, breakpoint_handler, double_fault_handler, enable_interrupts,
        error_interrupt_handler, launch_memory_manager, load_gdt, load_tss, p1_table_for_stack,
        p2_tables, page_fault_handler, reset_code_segment, spurious_interrupt_handler,
        timer_interrupt_handler,
    },
    boot_os,
};
use apic::{InterruptIndex, LOCAL_APIC_END, LOCAL_APIC_START};
use core::ptr;
use frame_allocation::{FfiOption, FrameAllocator, amd64::FOUR_KILOBYTES};
use ptr::{addr_of, addr_of_mut};
use x86_64::{
    VirtAddr,
    addr::PhysAddr,
    structures::{idt::InterruptDescriptorTable, paging::page_table::PageTableFlags},
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

        load_global_descriptor_table();
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

static TSS: TaskStateSegment = TaskStateSegment {
    reserved_0: 0,
    privilege_stack_table: [INTERRUPT_STACK_BOTTOM, 0, 0],
    reserved_1: 0,
    interrupt_stack_table: [DOUBLE_FAULT_STACK_TOP, 0, 0, 0, 0, 0, 0],
    reserved_2: 0,
    reserved_3: 0,
    io_permission_bitmap: TASK_STATE_SEGMENT_SIZE,
};

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

const GIGABYTE_PAGES_CPUID_BIT: u32 = 0x400_0000;

const DOUBLE_FAULT_IST_INDEX: u16 = 0;
const DOUBLE_FAULT_STACK_SIZE: usize = FOUR_KILOBYTES;

const DOUBLE_FAULT_STACK_TOP: u64 = 0xffff_ffff_ffe0_2000;

const INTERRUPT_STACK_BOTTOM: u64 = 0xffff_ffff_fff0_1000;

const LONG_MODE_TSS_SEGMENT_FLAG: u8 = 0x20;
const LONG_MODE_FLAT_SEGMENT_FLAG: u8 = 0x2f;
const TSS_ACCESS_BYTE: u8 = 0x89;
const KERNEL_CODE_SEGMENT_ACCESS_BYTE: u8 = 0x9a;
const USER_DATA_SEGMENT_ACCESS_BYTE: u8 = 0xf2;
const USER_CODE_SEGMENT_ACCESS_BYTE: u8 = 0xfa;

// size_of::<TaskStateSegment>() is known to be 0x68, which is within the allowed range for u16
#[allow(clippy::cast_possible_truncation)]
const TASK_STATE_SEGMENT_SIZE: u16 = size_of::<TaskStateSegment>() as u16;

#[repr(C, packed(4))]
struct TaskStateSegment {
    reserved_0: u32,
    privilege_stack_table: [u64; 3],
    reserved_1: u64,
    interrupt_stack_table: [u64; 7],
    reserved_2: u64,
    reserved_3: u16,
    io_permission_bitmap: u16,
}

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
    fn new(tss: *const TaskStateSegment) -> Self {
        Self {
            null: SegmentDescriptor::empty(),
            tss: SystemSegmentDescriptor::new(
                SegmentDescriptor::new(
                    TSS_ACCESS_BYTE,
                    LONG_MODE_TSS_SEGMENT_FLAG,
                    tss as u32,
                    TASK_STATE_SEGMENT_SIZE - 1,
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

#[repr(C, align(0x1000))]
struct DoubleFaultStack([u8; DOUBLE_FAULT_STACK_SIZE]);

unsafe fn load_global_descriptor_table() {
    unsafe {
        GDT = GlobalDescriptorTable::new(addr_of!(TSS));
        GDTR.offset = addr_of!(GDT);
        load_gdt(addr_of!(GDTR));
    }
}

fn supports_gigabyte_pages(cpu_info: u32) -> bool {
    (cpu_info & GIGABYTE_PAGES_CPUID_BIT) != 0
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
