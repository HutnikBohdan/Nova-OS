#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free hardware protocol building blocks for Nova OS.
//!
//! This crate deliberately performs no port I/O, MMIO or DMA itself. It owns
//! the wire formats, state machines and bounded parsers used by kernel drivers.

pub mod acpi;
pub mod ahci;
pub mod hda;
pub mod hid;
pub mod nvme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    InvalidSignature,
    InvalidChecksum,
    InvalidLength,
    InvalidValue,
    Capacity,
    Unsupported,
}
