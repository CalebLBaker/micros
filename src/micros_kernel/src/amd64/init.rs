use crate::{
    BootError,
    amd64::{
        apic,
        arch::{PROC, PageTableEntry, PageTableFlags},
        breakpoint_handler, double_fault_handler, enable_interrupts, error_interrupt_handler,
        launch_memory_manager, load_gdt, load_idt, load_tss, p1_table_for_stack, p2_tables,
        page_fault_handler, reset_code_segment, spurious_interrupt_handler,
        timer_interrupt_handler,
    },
    boot_os,
};
use address::AddressMapper;
use apic::{InterruptIndex, LOCAL_APIC_END, LOCAL_APIC_START};
use core::ptr;
use frame_allocation::{FfiOption, FrameAllocator};
use ptr::{addr_of, addr_of_mut};

#[repr(C, packed(2))]
pub struct GdtDescriptor {
    size: u16,
    offset: *const GlobalDescriptorTable,
}

#[repr(C, packed(2))]
pub struct IdtDescriptor {
    size: u16,
    offset: *const InterruptDescriptorTable,
}

#[repr(C)]
pub struct InterruptServiceRoutine {
    _fake: u8,
}

pub unsafe fn initialize_operating_system(
    multiboot_info_ptr: u32,
    cpu_info: u32,
) -> Result<(), BootError> {
    unsafe {
        let proc = &mut *addr_of_mut!(PROC);
        load_global_descriptor_table();
        reset_code_segment();
        load_tss();
        setup_interrupts(proc);

        if supports_gigabyte_pages(cpu_info) {
            proc.allocator.four_kilobyte_pages.add_frame(
                proc.kernel_pointer_to_mapped_physical_memory_pointer(addr_of_mut!(p2_tables[0]))
                    .cast(),
            );
            proc.allocator.four_kilobyte_pages.add_frame(
                proc.kernel_pointer_to_mapped_physical_memory_pointer(addr_of_mut!(p2_tables[1]))
                    .cast(),
            );
            proc.allocator.gigabyte_pages = FfiOption::Some(FrameAllocator::default());
        }
        let boot_info_ptr = proc.physical_address_to_pointer(multiboot_info_ptr.into());
        let memory_manager_launch_info = boot_os(
            proc,
            boot_info_ptr,
            proc.physical_address_to_pointer(LOCAL_APIC_START)
                ..proc.physical_address_to_pointer(LOCAL_APIC_END),
        )?;

        let double_fault_stack = proc
            .allocator
            .get_4k_frame()
            .ok_or(BootError::OutOfMemory)?;
        p1_table_for_stack[0x001] = PageTableEntry::new(
            proc.pointer_to_physical_address(double_fault_stack),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        );

        launch_memory_manager(
            ptr::from_mut(proc),
            boot_info_ptr,
            memory_manager_launch_info.root_page_table_address.address,
            memory_manager_launch_info.entry_point.address,
        );
    }
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable {
    division_error: IdtGate::EMPTY,
    debug: IdtGate::EMPTY,
    non_maskable_interrupt: IdtGate::EMPTY,
    breakpoint: IdtGate::EMPTY,
    overflow: IdtGate::EMPTY,
    bound_range_exceeded: IdtGate::EMPTY,
    invalid_opcode: IdtGate::EMPTY,
    device_not_available: IdtGate::EMPTY,
    double_fault: IdtGate::EMPTY,
    coprocessor_segment_overrun: IdtGate::EMPTY,
    invalid_tss: IdtGate::EMPTY,
    segment_not_present: IdtGate::EMPTY,
    stack_segmentation_fault: IdtGate::EMPTY,
    general_protection_fault: IdtGate::EMPTY,
    page_fault: IdtGate::EMPTY,
    reserved_0: IdtGate::EMPTY,
    x87_floating_point_exception: IdtGate::EMPTY,
    alignment_check: IdtGate::EMPTY,
    machine_check: IdtGate::EMPTY,
    simd_floating_point_exception: IdtGate::EMPTY,
    virtualization_exception: IdtGate::EMPTY,
    control_point_exception: IdtGate::EMPTY,
    reserved_1: [IdtGate::EMPTY; 0xa],
    interrupts: [IdtGate::EMPTY; 0xe0],
};

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

// size_of::<InterruptDescriptorTable>() is known to be 0x1000, which is within the bounds for u16
#[allow(clippy::cast_possible_truncation)]
static mut IDTR: IdtDescriptor = IdtDescriptor {
    size: (size_of::<InterruptDescriptorTable>() - 1) as u16,
    offset: ptr::null(),
};

// size_of::<GlobalDescriptorTable>() is known to be 0x30, which is within the bounds for u16
#[allow(clippy::cast_possible_truncation)]
static mut GDTR: GdtDescriptor = GdtDescriptor {
    size: (size_of::<GlobalDescriptorTable>() - 1) as u16,
    offset: ptr::null(),
};

const GIGABYTE_PAGES_CPUID_BIT: u32 = 0x400_0000;

const DOUBLE_FAULT_IST_INDEX: u8 = 0;

const DOUBLE_FAULT_STACK_TOP: u64 = 0xffff_ffff_ffe0_2000;

const INTERRUPT_STACK_BOTTOM: u64 = 0xffff_ffff_fff0_1000;

const LONG_MODE_TSS_SEGMENT_FLAG: u8 = 0x20;
const LONG_MODE_FLAT_SEGMENT_FLAG: u8 = 0x2f;
const TSS_ACCESS_BYTE: u8 = 0x89;
const KERNEL_CODE_SEGMENT_ACCESS_BYTE: u8 = 0x9a;
const USER_DATA_SEGMENT_ACCESS_BYTE: u8 = 0xf2;
const USER_CODE_SEGMENT_ACCESS_BYTE: u8 = 0xfa;

const KERNEL_CODE_SEGMENT_SELECTOR: u16 = 0x10;

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

#[repr(C)]
struct IdtGate {
    offset_0_16: u16,
    segment_selector: u16,
    interrupt_stack_index: u8,
    flags: u8,
    offset_16_32: u16,
    offset_32_64: u32,
    reserved: u32,
}

impl IdtGate {
    const INTERRUPT_GATE: u8 = 0xe;
    const PRESENT: u8 = 0x8e;

    const EMPTY: Self = Self {
        offset_0_16: 0,
        segment_selector: KERNEL_CODE_SEGMENT_SELECTOR,
        interrupt_stack_index: 0,
        flags: Self::INTERRUPT_GATE,
        offset_16_32: 0,
        offset_32_64: 0,
        reserved: 0,
    };

    fn set_address(&mut self, isr: *const InterruptServiceRoutine) {
        let address = isr as u64;
        self.flags |= Self::PRESENT;
        self.offset_0_16 = (address & 0xffff) as u16;
        self.offset_16_32 = ((address & 0xffff_0000) >> 16) as u16;
        self.offset_32_64 = ((address & 0xffff_ffff_0000_0000) >> 32) as u32;
    }
}

#[repr(C)]
struct InterruptDescriptorTable {
    division_error: IdtGate,
    debug: IdtGate,
    non_maskable_interrupt: IdtGate,
    breakpoint: IdtGate,
    overflow: IdtGate,
    bound_range_exceeded: IdtGate,
    invalid_opcode: IdtGate,
    device_not_available: IdtGate,
    double_fault: IdtGate,
    coprocessor_segment_overrun: IdtGate,
    invalid_tss: IdtGate,
    segment_not_present: IdtGate,
    stack_segmentation_fault: IdtGate,
    general_protection_fault: IdtGate,
    page_fault: IdtGate,
    reserved_0: IdtGate,
    x87_floating_point_exception: IdtGate,
    alignment_check: IdtGate,
    machine_check: IdtGate,
    simd_floating_point_exception: IdtGate,
    virtualization_exception: IdtGate,
    control_point_exception: IdtGate,
    reserved_1: [IdtGate; 0xa],
    interrupts: [IdtGate; 0xe0],
}

impl InterruptDescriptorTable {
    fn interrupt(&mut self, index: u16) -> &mut IdtGate {
        &mut self.interrupts[(index - 0x20) as usize]
    }
}

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

unsafe fn setup_interrupts<AddrMap: AddressMapper>(address_mapper: &AddrMap) {
    let idt_ref = unsafe { &mut *{ (&raw mut IDT) } };
    idt_ref.breakpoint.set_address(addr_of!(breakpoint_handler));
    idt_ref
        .double_fault
        .set_address(addr_of!(double_fault_handler));
    idt_ref.double_fault.interrupt_stack_index = DOUBLE_FAULT_IST_INDEX;
    idt_ref.page_fault.set_address(addr_of!(page_fault_handler));
    set_interrupt_handlers(idt_ref);
    unsafe {
        IDTR.offset = addr_of!(IDT);
        load_idt(addr_of!(IDTR));
        apic::init(address_mapper);
        enable_interrupts();
    }
}

fn set_interrupt_handlers(idt: &mut InterruptDescriptorTable) {
    idt.interrupt(InterruptIndex::Timer as u16)
        .set_address(addr_of!(timer_interrupt_handler));
    idt.interrupt(InterruptIndex::Spurious as u16)
        .set_address(addr_of!(spurious_interrupt_handler));
    idt.interrupt(InterruptIndex::Error as u16)
        .set_address(addr_of!(error_interrupt_handler));
}
