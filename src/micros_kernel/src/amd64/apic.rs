use spin::Mutex;
use core::ops::DerefMut;
use x2apic::lapic::{xapic_base, LocalApic, LocalApicBuilder};
use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Error = APIC_OFFSET,
    Spurious,
    Timer,
}

pub unsafe fn init() -> Option<()> {
    disable_pic(MASTER_PIC, MASTER_PIC_OFFSET, SLAVE_PICS_MASK);
    disable_pic(SLAVE_PIC, SLAVE_PIC_OFFSET, SLAVE_PIC_IDENTITY);
    let mut apic_option = LOCAL_APIC.lock();
    *apic_option = create_apic_builder()
        .set_xapic_base(xapic_base())
        .build()
        .ok();
    let apic = apic_option.deref_mut().as_mut()?;
    apic.enable();
    apic.disable_timer();
    Some(())
}

pub unsafe fn end_interrupt() {
    if let Some(apic) = LOCAL_APIC.lock().as_mut() {
        apic.end_of_interrupt();
    }
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

static LOCAL_APIC: Mutex<Option<LocalApic>> = Mutex::new(None);

fn create_apic_builder() -> LocalApicBuilder {
    let mut apic_builder = LocalApicBuilder::new();
    apic_builder
        .timer_vector(InterruptIndex::Timer as usize)
        .error_vector(InterruptIndex::Error as usize)
        .spurious_vector(InterruptIndex::Spurious as usize);
    apic_builder
}

