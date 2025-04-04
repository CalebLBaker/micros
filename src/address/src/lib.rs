#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

use core::{
    convert::{From, TryFrom},
    num::TryFromIntError,
    ops::Add,
};

#[derive(Clone, Copy)]
pub struct PhysicalAddress {
    pub address: usize,
}

impl PhysicalAddress {
    #[must_use]
    pub const fn new(address: usize) -> Self {
        Self { address }
    }
}

impl From<usize> for PhysicalAddress {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}

impl TryFrom<u64> for PhysicalAddress {
    type Error = TryFromIntError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(Self::new(value.try_into()?))
    }
}

impl From<u32> for PhysicalAddress {
    fn from(value: u32) -> Self {
        (value as usize).into()
    }
}

impl Add<usize> for PhysicalAddress {
    type Output = Self;

    fn add(self, other: usize) -> Self::Output {
        (self.address + other).into()
    }
}

#[derive(Clone, Copy)]
pub struct VirtualAddress {
    pub address: usize,
}

impl VirtualAddress {
    #[must_use]
    pub const fn new(address: usize) -> Self {
        Self { address }
    }
}

impl From<usize> for VirtualAddress {
    #[must_use]
    fn from(value: usize) -> Self {
        Self { address: value }
    }
}

impl Add<usize> for VirtualAddress {
    type Output = Self;

    fn add(self, other: usize) -> Self::Output {
        (self.address + other).into()
    }
}

pub trait AddressMapper {
    fn physical_to_virtual_address(&self, physical_address: PhysicalAddress) -> VirtualAddress;
    fn virtual_to_physical_address(&self, virtual_address: VirtualAddress) -> PhysicalAddress;

    fn physical_address_to_pointer<T>(&self, physical_address: PhysicalAddress) -> *mut T {
        self.physical_to_virtual_address(physical_address).address as *mut T
    }

    fn pointer_to_physical_address<T>(&self, pointer: *const T) -> PhysicalAddress {
        self.virtual_to_physical_address((pointer as usize).into())
    }
}
