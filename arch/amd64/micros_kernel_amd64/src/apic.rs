use spin::Mutex;
use x2apic::lapic::{xapic_base, LocalApic, LocalApicBuilder};

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Error = PIC_OFFSET,
    Spurious,
    Timer,
}

pub unsafe fn init() -> Result<(), &'static str> {
    let mut apic = create_apic_builder().set_xapic_base(xapic_base()).build()?;
    apic.enable();
    set_local_apic(apic);
    Ok(())
}

pub unsafe fn end_interrupt() {
    if let Some(apic) = LOCAL_APIC.lock().as_mut() {
        apic.end_of_interrupt();
    }
}

const PIC_OFFSET: u8 = 32;

static LOCAL_APIC: Mutex<Option<LocalApic>> = Mutex::new(None);

fn create_apic_builder() -> LocalApicBuilder {
    let mut apic_builder = LocalApicBuilder::new();
    apic_builder
        .timer_vector(InterruptIndex::Timer as usize)
        .error_vector(InterruptIndex::Error as usize)
        .spurious_vector(InterruptIndex::Spurious as usize);
    apic_builder
}

fn set_local_apic(apic: LocalApic) {
    *LOCAL_APIC.lock() = Some(apic);
}
