#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

pub unsafe fn init() {
    PICS.lock().initialize();
}

pub extern "x86-interrupt" fn keyboard_interrupt(_: &mut x86_64::structures::idt::InterruptStackFrame) {
    let _ = display_daemon::WRITER.lock().write_str("k");
    let mut pic = PICS.lock();
    unsafe {
        pic.notify_end_of_interrupt(InterruptIndex::Keyboard as u8);
    }
}

pub extern "x86-interrupt" fn timer_interrupt_handler(_: &mut x86_64::structures::idt::InterruptStackFrame) {
    let _ = display_daemon::WRITER.lock().write_str(".");
    let mut pic = PICS.lock();
    unsafe {
        pic.notify_end_of_interrupt(InterruptIndex::Timer as u8);
    }
}

const PIC_1_OFFSET: u8 = 32;
const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: spin::Mutex<pic8259_simple::ChainedPics> = spin::Mutex::new(unsafe {
    pic8259_simple::ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET)
});

