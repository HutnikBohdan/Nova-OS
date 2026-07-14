#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

//! Allocation-free USB protocol primitives. Hardware access and DMA ownership
//! remain the responsibility of the platform driver.

pub mod bot;
pub mod control;
pub mod descriptor;
pub mod hid;
pub mod hub;
pub mod xhci;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbError {
    BufferTooSmall,
    InvalidDescriptor,
    InvalidState,
    RingFull,
    RingEmpty,
    Timeout,
    Stall,
    Babble,
    Transaction,
    Disconnected,
    Protocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deadline {
    expires_at: u64,
}

impl Deadline {
    pub const fn after(now: u64, ticks: u64) -> Self {
        Self {
            expires_at: now.saturating_add(ticks),
        }
    }

    pub const fn expired(self, now: u64) -> bool {
        now >= self.expires_at
    }
}
