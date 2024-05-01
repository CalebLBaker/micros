use x86_64::{instructions::port::Port, registers::model_specific::Msr};

pub const LOCAL_APIC_START: usize = 0xFEE0_0000;
pub const LOCAL_APIC_END: usize = 0xFEE0_1000;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Error = APIC_OFFSET,
    Timer,
    Spurious = SPURIOUS_INTERRUPT_VECTOR_INDEX,
}

pub unsafe fn init() {
    disable_pic(MASTER_PIC, MASTER_PIC_OFFSET, SLAVE_PICS_MASK);
    disable_pic(SLAVE_PIC, SLAVE_PIC_OFFSET, SLAVE_PIC_IDENTITY);
    Msr::new(APIC_BASE_MODEL_SPECIFIC_REGISTER).write(APIC_BASE);
    TIMER_REGISTER.write_volatile(TIMER_REGISTER_VALUE);
    ERROR_REGISTER.write_volatile(InterruptIndex::Error as u8);
    SPURIOUS_INTERRUPT_REGISTER.write_volatile(SPURIOUS_INTERRUPT_REGISTER_VALUE);
}

pub unsafe fn end_interrupt() {
    END_OF_INTERRUPT.write_volatile(0);
}

unsafe fn disable_pic(base_port_number: u16, vector_offset: u8, icw3: u8) {
    Port::new(base_port_number).write(INITIALIZE_PIC);
    let mut data = Port::new(base_port_number + 1);
    data.write(vector_offset);
    data.write(icw3);
    data.write(PIC_8086);
    data.write(MASK_ALL_INTERRUPTS);
}

const MASTER_PIC: u16 = 0x20;
const SLAVE_PIC: u16 = 0xA0;
const INITIALIZE_PIC: u8 = 0x10;
const PIC_8086: u8 = 1;
const MASK_ALL_INTERRUPTS: u8 = 0xff;
const SLAVE_PICS_MASK: u8 = 4;
const SLAVE_PIC_IDENTITY: u8 = 2;

const MASTER_PIC_OFFSET: u8 = 0x20;
const SLAVE_PIC_OFFSET: u8 = 0x28;
const APIC_OFFSET: u8 = 0x30;

const SPURIOUS_INTERRUPT_VECTOR_INDEX: u8 = 0xFF;
const SPURIOUS_INTERRUPT_REGISTER_VALUE: u32 = 0x1FF;
const TIMER_REGISTER_VALUE: u32 = 0x10000 | InterruptIndex::Timer as u32;
const APIC_BASE: u64 = 0xFEE0_0800;
const SPURIOUS_INTERRUPT_REGISTER: *mut u32 = 0xFEE0_00F0 as *mut u32;
const TIMER_REGISTER: *mut u32 = 0xFEE0_0320 as *mut u32;
const ERROR_REGISTER: *mut u8 = 0xFEE0_0370  as *mut u8;
const END_OF_INTERRUPT: *mut u32 = 0xFEE0_00B0 as *mut u32;

const APIC_BASE_MODEL_SPECIFIC_REGISTER: u32 = 0x1B;

