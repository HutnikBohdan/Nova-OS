#![no_std]
#![forbid(unsafe_code)]

//! Deterministic, allocation-free x86-64 platform discovery and bring-up.
//!
//! This crate never performs MMIO, port I/O or waits itself. It parses firmware
//! data and returns explicit operations for the privileged kernel backend.

pub mod acpi_power;
pub mod cpu;
pub mod ioapic;
pub mod irq;
pub mod startup;
pub mod topology;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    InvalidLength,
    InvalidValue,
    Capacity,
    Duplicate,
    NotFound,
    Conflict,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundedVec<T: Copy, const N: usize> {
    entries: [Option<T>; N],
    len: usize,
}

impl<T: Copy, const N: usize> Default for BoundedVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> BoundedVec<T, N> {
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            len: 0,
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub const fn capacity(&self) -> usize {
        N
    }
    pub fn push(&mut self, value: T) -> Result<(), Error> {
        if self.len == N {
            return Err(Error::Capacity);
        }
        self.entries[self.len] = Some(value);
        self.len += 1;
        Ok(())
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len {
            self.entries[index].as_ref()
        } else {
            None
        }
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            self.entries[index].as_mut()
        } else {
            None
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries[..self.len].iter().filter_map(Option::as_ref)
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_vec_is_deterministic() {
        let mut v = BoundedVec::<u8, 2>::new();
        assert_eq!(v.push(4), Ok(()));
        assert_eq!(v.push(7), Ok(()));
        assert_eq!(v.push(9), Err(Error::Capacity));
        assert_eq!(v.iter().copied().collect::<std::vec::Vec<_>>(), [4, 7]);
    }
}
