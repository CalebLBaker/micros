mod apic;
mod arch;
mod init;

use arch::PageTable;
use architecture::amd64::Amd64;
use core::panic::PanicInfo;
pub use init::initialize_operating_system;
use init::{GdtDescriptor, IdtDescriptor, InterruptServiceRoutine};
use multiboot2::BootInformationHeader;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe { halt() }
}

unsafe extern "C" {
    pub fn halt() -> !;

    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
    static mut p1_table_for_stack: PageTable;
    fn launch_memory_manager(
        arch: *mut Amd64,
        boot_info_ptr: *const BootInformationHeader,
        root_page_table_address: usize,
        entry_point: usize,
    ) -> !;

    fn write_port(port: u16, value: u8);
    fn set_apic_base();
    fn enable_interrupts();
    fn load_tss();
    fn reset_code_segment();
    fn load_gdt(gdtr: *const GdtDescriptor);
    fn load_idt(idtr: *const IdtDescriptor);

    // These are interrupt handlers that don't actually follow the C calling
    // convention, so they should not be called from Rust code.
    static breakpoint_handler: InterruptServiceRoutine;
    static spurious_interrupt_handler: InterruptServiceRoutine;
    static error_interrupt_handler: InterruptServiceRoutine;
    static timer_interrupt_handler: InterruptServiceRoutine;
    static double_fault_handler: InterruptServiceRoutine;
    static page_fault_handler: InterruptServiceRoutine;
}
