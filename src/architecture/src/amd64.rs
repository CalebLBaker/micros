use crate::Architecture;
use frame_allocation::amd64::Amd64FrameAllocator;
use physical_address::{AddressMapper, PhysicalAddress};

#[repr(C)]
pub struct Amd64 {
    pub allocator: Amd64FrameAllocator,
    memory_offset: u64,
}

impl Amd64 {
    #[must_use]
    pub const fn new(allocator: Amd64FrameAllocator, memory_offset: u64) -> Self {
        Self {
            allocator,
            memory_offset,
        }
    }
}

impl AddressMapper for Amd64 {
    // This function only exists for 64-bit processors, where casting from u64 to usize should be
    // safe
    #[allow(clippy::cast_possible_truncation)]
    fn physical_to_virtual_address<T>(&self, physical_address: PhysicalAddress) -> *mut T {
        (physical_address.address + self.memory_offset as usize) as *mut T
    }
}

impl Architecture for Amd64 {}
