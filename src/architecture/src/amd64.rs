use crate::Architecture;
use address::{AddressMapper, PhysicalAddress, VirtualAddress};
use frame_allocation::amd64::Amd64FrameAllocator;

#[repr(C)]
pub struct Amd64 {
    pub allocator: Amd64FrameAllocator,
    memory_offset: u64,
    kernel_offset: i64,
}

impl Amd64 {
    // Casting address offsets from u64 to i64 is safe since wrapping only occurs for 64-bit values
    // and the address space is only 48 bits.
    #[allow(clippy::cast_possible_wrap)]
    #[must_use]
    pub const fn new(
        allocator: Amd64FrameAllocator,
        memory_offset: VirtualAddress,
        kernel_offset: i64,
    ) -> Self {
        Self {
            allocator,
            memory_offset: memory_offset.address as u64,
            kernel_offset: kernel_offset + memory_offset.address as i64,
        }
    }
}

// This struct only exists for 64-bit processors, where casting from u64 to usize should be
// safe
#[allow(clippy::cast_possible_truncation)]
impl Amd64 {
    #[must_use]
    pub fn mapped_physical_memory_start(&self) -> VirtualAddress {
        (self.memory_offset as usize).into()
    }

    #[must_use]
    pub fn kernel_pointer_to_mapped_physical_memory_pointer<T>(&self, pointer: *mut T) -> *mut T {
        (pointer as i64 + self.kernel_offset) as *mut T
    }
}

// This struct only exists for 64-bit processors, where casting from u64 to usize should be
// safe
#[allow(clippy::cast_possible_truncation)]
impl AddressMapper for Amd64 {
    fn physical_to_virtual_address(&self, physical_address: PhysicalAddress) -> VirtualAddress {
        (physical_address.address + self.memory_offset as usize).into()
    }

    fn virtual_to_physical_address(&self, virtual_address: VirtualAddress) -> PhysicalAddress {
        (virtual_address.address - self.memory_offset as usize).into()
    }
}

impl Architecture for Amd64 {}
