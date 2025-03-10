mod apic;
mod elf;
mod init;

use core::panic::PanicInfo;
use frame_allocation::amd64::Amd64FrameAllocator;
pub use init::initialize_operating_system;
use x86_64::{
    instructions::hlt,
    structures::paging::PageTable,
};

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

    fn breakpoint_handler();
    fn spurious_interrupt_handler();
    fn error_interrupt_handler();
    fn timer_interrupt_handler();
    fn double_fault_handler();
    fn page_fault_handler();
}

