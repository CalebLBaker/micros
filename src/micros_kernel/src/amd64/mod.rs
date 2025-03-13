mod apic;
mod arch;
mod elf;
mod init;

use core::panic::PanicInfo;
use frame_allocation::amd64::Amd64FrameAllocator;
use init::GdtDescriptor;
pub use init::initialize_operating_system;
use x86_64::{instructions::hlt, structures::paging::PageTable};

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt()
}

pub fn halt() -> ! {
    loop {
        hlt();
    }
}

unsafe extern "C" {
    static mut p4_table: PageTable;
    static mut p2_tables: [PageTable; 2];
    static mut p1_table_for_stack: PageTable;
    fn launch_memory_manager(
        allocator: *mut Amd64FrameAllocator,
        boot_info_ptr: *const u8,
        root_page_table_address: usize,
        entry_point: usize,
    ) -> !;

    fn write_port(port: u16, value: u8);
    fn set_apic_base();
    fn enable_interrupts();
    fn load_tss();
    fn reset_code_segment();
    fn load_gdt(gdtr: *const GdtDescriptor);

    // These are interrupt handlers that don't actually follow the C calling
    // convention, so they should not be called from Rust code.
    fn breakpoint_handler();
    fn spurious_interrupt_handler();
    fn error_interrupt_handler();
    fn timer_interrupt_handler();
    fn double_fault_handler();
    fn page_fault_handler();
}
