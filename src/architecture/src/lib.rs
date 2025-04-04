#![no_std]
#![deny(clippy::all)]
#![deny(clippy::pedantic)]

#[cfg(target_arch = "x86_64")]
pub mod amd64;

use address::AddressMapper;

pub trait Architecture: Sized + AddressMapper {}
