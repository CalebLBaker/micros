#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

pub struct PhysicalAddress {
    pub address: usize,
}

impl PhysicalAddress {
    #[must_use]
    pub const fn new(address: usize) -> Self {
        Self { address }
    }

    #[must_use]
    pub fn from_u64(address: u64) -> Option<Self> {
        Some(Self::new(usize::try_from(address).ok()?))
    }

    #[must_use]
    pub const fn from_u32(address: u32) -> Self {
        Self::new(address as usize)
    }
}

pub trait AddressMapper {
    fn physical_to_virtual_address<T>(&self, physical_address: PhysicalAddress) -> *mut T;
}
