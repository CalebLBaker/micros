use lazy_static::lazy_static;

pub unsafe fn init() -> bool {
    let mut local_apic = LOCAL_APIC.lock();
    if local_apic.is_some() {
        local_apic.as_mut().unwrap().enable();
        true
    }
    else {
        false
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Error = PIC_OFFSET,
    Spurious,
    Timer,
}

pub unsafe fn end_interrupt() {
    LOCAL_APIC.lock().as_mut().unwrap().end_of_interrupt();
}

pub extern "x86-interrupt" fn spurious_interrupt_handler(_: &mut x86_64::structures::idt::InterruptStackFrame) {
    let _ = display_daemon::WRITER.lock().write_str("Spurious");
    unsafe { end_interrupt(); }
}

pub extern "x86-interrupt" fn error_interrupt_handler(_: &mut x86_64::structures::idt::InterruptStackFrame) {
    let _ = display_daemon::WRITER.lock().write_str("Error");
    unsafe { end_interrupt(); }
}

pub extern "x86-interrupt" fn timer_interrupt_handler(_: &mut x86_64::structures::idt::InterruptStackFrame) {
    let _ = display_daemon::WRITER.lock().write_str(".");
    unsafe { end_interrupt(); }
}

const PIC_OFFSET: u8 = 32;

lazy_static! {
    pub static ref LOCAL_APIC: spin::Mutex<Option<x2apic::lapic::LocalApic>> = spin::Mutex::new(
        match x2apic::lapic::LocalApicBuilder::new().timer_vector(InterruptIndex::Timer as usize).error_vector(InterruptIndex::Error as usize).spurious_vector(InterruptIndex::Spurious as usize).build() {
            Ok(ret) => Some(ret),
            _ => None,
        }
    );
}

