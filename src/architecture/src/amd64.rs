use crate::Architecture;
use address::{AddressMapper, PhysicalAddress, VirtualAddress};
use frame_allocation::amd64::Amd64FrameAllocator;

#[repr(C)]
pub struct Amd64 {
    pub allocator: Amd64FrameAllocator,
    memory_offset: u64,
    kernel_offset: u64,
    memory_manager_offset: u64,
    stack_offset: u64,
}

impl Amd64 {
    #[must_use]
    pub const fn new(
        allocator: Amd64FrameAllocator,
        memory_offset: VirtualAddress,
        kernel_offset: VirtualAddress,
        memory_manager_offset: VirtualAddress,
        stack_offset: VirtualAddress,
    ) -> Self {
        Self {
            allocator,
            memory_offset: memory_offset.address as u64,
            kernel_offset: kernel_offset.address as u64,
            memory_manager_offset: memory_manager_offset.address as u64,
            stack_offset: stack_offset.address as u64,
        }
    }
}

// This struct only exists for 64-bit processors, where casting from u64 to usize should be
// safe
#[allow(clippy::cast_possible_truncation)]
impl Amd64 {
    pub fn kernel_mem_virtual_to_physical_address<T>(
        &self,
        virtual_address: *const T,
    ) -> PhysicalAddress {
        (virtual_address as usize - self.kernel_offset as usize).into()
    }

    #[must_use]
    pub fn mapped_physical_memory_start(&self) -> VirtualAddress {
        (self.memory_offset as usize).into()
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
