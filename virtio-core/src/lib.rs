#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free VirtIO 1.x protocol state machines.
//!
//! Hardware access is injected through [`RegisterIo`] and [`DmaMemory`]; this
//! crate never dereferences device addresses and contains no unsafe code.

pub mod block;
pub mod dma;
pub mod input;
pub mod interrupt;
pub mod net;
pub mod queue;
pub mod transport;

/// Width-checked register access supplied by a bus implementation.
pub trait RegisterIo {
    type Error;
    fn read_u8(&self, offset: u64) -> Result<u8, Self::Error>;
    fn read_u16(&self, offset: u64) -> Result<u16, Self::Error>;
    fn read_u32(&self, offset: u64) -> Result<u32, Self::Error>;
    fn read_u64(&self, offset: u64) -> Result<u64, Self::Error>;
    fn write_u8(&mut self, offset: u64, value: u8) -> Result<(), Self::Error>;
    fn write_u16(&mut self, offset: u64, value: u16) -> Result<(), Self::Error>;
    fn write_u32(&mut self, offset: u64, value: u32) -> Result<(), Self::Error>;
    fn write_u64(&mut self, offset: u64, value: u64) -> Result<(), Self::Error>;
}

/// DMA access supplied by the kernel's mapper/cache-coherency layer.
pub trait DmaMemory {
    type Error;
    fn read(&self, device_address: u64, out: &mut [u8]) -> Result<(), Self::Error>;
    fn write(&mut self, device_address: u64, data: &[u8]) -> Result<(), Self::Error>;
    fn sync_for_device(&mut self, device_address: u64, len: usize) -> Result<(), Self::Error>;
    fn sync_for_cpu(&mut self, device_address: u64, len: usize) -> Result<(), Self::Error>;
}
