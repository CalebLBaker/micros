use super::{set_apic_base, write_port};

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
    unsafe {
        disable_pic(MASTER_PIC, MASTER_PIC_OFFSET, SLAVE_PICS_MASK);
        disable_pic(SLAVE_PIC, SLAVE_PIC_OFFSET, SLAVE_PIC_IDENTITY);
        set_apic_base();
        TIMER_REGISTER.write_volatile(TIMER_REGISTER_VALUE);
        ERROR_REGISTER.write_volatile(InterruptIndex::Error as u8);
        SPURIOUS_INTERRUPT_REGISTER.write_volatile(SPURIOUS_INTERRUPT_REGISTER_VALUE);
    }
}

unsafe fn disable_pic(base_port_number: u16, vector_offset: u8, icw3: u8) {
    let data_port = base_port_number + 1;
    unsafe {
        write_port(base_port_number, INITIALIZE_PIC);
        write_port(data_port, vector_offset);
        write_port(data_port, icw3);
        write_port(data_port, PIC_8086);
        write_port(data_port, MASK_ALL_INTERRUPTS);
    }
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
const SPURIOUS_INTERRUPT_REGISTER: *mut u32 = 0xFEE0_00F0 as *mut u32;
const TIMER_REGISTER: *mut u32 = 0xFEE0_0320 as *mut u32;
const ERROR_REGISTER: *mut u8 = 0xFEE0_0370 as *mut u8;
