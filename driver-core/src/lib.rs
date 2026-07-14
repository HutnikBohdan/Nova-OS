#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddress {
    pub const fn config_address(self, offset: u8) -> u32 {
        0x8000_0000
            | ((self.bus as u32) << 16)
            | ((self.device as u32) << 11)
            | ((self.function as u32) << 8)
            | ((offset as u32) & 0xfc)
    }
}

pub trait PciConfig {
    fn read_u32(&mut self, address: PciAddress, offset: u8) -> u32;
    fn write_u32(&mut self, address: PciAddress, offset: u8, value: u32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciDevice {
    pub address: PciAddress,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
    pub header_type: u8,
}

pub fn probe(config: &mut impl PciConfig, address: PciAddress) -> Option<PciDevice> {
    let identity = config.read_u32(address, 0);
    let vendor = identity as u16;
    if vendor == 0xffff {
        return None;
    }
    let class = config.read_u32(address, 8);
    let header = config.read_u32(address, 12);
    Some(PciDevice {
        address,
        vendor,
        device: (identity >> 16) as u16,
        class: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        interface: (class >> 8) as u8,
        header_type: (header >> 16) as u8,
    })
}

pub fn enumerate<const N: usize>(
    config: &mut impl PciConfig,
    output: &mut [Option<PciDevice>; N],
) -> usize {
    let mut found = 0;
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let first = PciAddress {
                bus,
                device,
                function: 0,
            };
            let Some(info) = probe(config, first) else {
                continue;
            };
            if found < N {
                output[found] = Some(info);
                found += 1;
            }
            let functions = if info.header_type & 0x80 != 0 { 8 } else { 1 };
            for function in 1..functions {
                if let Some(info) = probe(
                    config,
                    PciAddress {
                        bus,
                        device,
                        function,
                    },
                ) {
                    if found < N {
                        output[found] = Some(info);
                        found += 1;
                    }
                }
            }
            if found == N {
                return found;
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    Io { port: u16 },
    Memory32 { address: u32, prefetchable: bool },
    Memory64Low { low: u32, prefetchable: bool },
    Unused,
}

pub fn decode_bar(value: u32) -> Bar {
    if value == 0 {
        return Bar::Unused;
    }
    if value & 1 == 1 {
        return Bar::Io {
            port: (value & !3) as u16,
        };
    }
    let prefetchable = value & 8 != 0;
    match (value >> 1) & 3 {
        0 => Bar::Memory32 {
            address: value & !0xf,
            prefetchable,
        },
        2 => Bar::Memory64Low {
            low: value & !0xf,
            prefetchable,
        },
        _ => Bar::Unused,
    }
}

pub mod virtio {
    pub const STATUS_ACKNOWLEDGE: u8 = 1;
    pub const STATUS_DRIVER: u8 = 2;
    pub const STATUS_DRIVER_OK: u8 = 4;
    pub const STATUS_FEATURES_OK: u8 = 8;
    pub const STATUS_FAILED: u8 = 128;
    pub const DESC_F_NEXT: u16 = 1;
    pub const DESC_F_WRITE: u16 = 2;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Descriptor {
        pub address: u64,
        pub length: u32,
        pub flags: u16,
        pub next: u16,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Negotiation {
        pub offered: u64,
        pub required: u64,
        pub accepted: u64,
    }
    impl Negotiation {
        pub const fn new(offered: u64, required: u64, optional: u64) -> Option<Self> {
            if offered & required != required {
                return None;
            }
            Some(Self {
                offered,
                required,
                accepted: required | (offered & optional),
            })
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum QueueError {
        Full,
        InvalidCompletion,
    }

    pub struct SplitQueue<const N: usize> {
        descriptors: [Descriptor; N],
        free: [u16; N],
        free_len: usize,
        outstanding: [bool; N],
        pub available_index: u16,
        pub used_index: u16,
    }

    impl<const N: usize> SplitQueue<N> {
        pub const fn new() -> Self {
            let mut free = [0u16; N];
            let mut index = 0;
            while index < N {
                free[index] = index as u16;
                index += 1;
            }
            Self {
                descriptors: [Descriptor {
                    address: 0,
                    length: 0,
                    flags: 0,
                    next: 0,
                }; N],
                free,
                free_len: N,
                outstanding: [false; N],
                available_index: 0,
                used_index: 0,
            }
        }
        pub fn submit(&mut self, descriptor: Descriptor) -> Result<u16, QueueError> {
            if self.free_len == 0 {
                return Err(QueueError::Full);
            }
            self.free_len -= 1;
            let id = self.free[self.free_len];
            self.descriptors[id as usize] = descriptor;
            self.outstanding[id as usize] = true;
            self.available_index = self.available_index.wrapping_add(1);
            Ok(id)
        }
        pub fn complete(&mut self, id: u16) -> Result<Descriptor, QueueError> {
            let index = id as usize;
            if index >= N || !self.outstanding[index] {
                return Err(QueueError::InvalidCompletion);
            }
            self.outstanding[index] = false;
            self.free[self.free_len] = id;
            self.free_len += 1;
            self.used_index = self.used_index.wrapping_add(1);
            Ok(self.descriptors[index])
        }
        pub const fn free_count(&self) -> usize {
            self.free_len
        }
    }

    impl<const N: usize> Default for SplitQueue<N> {
        fn default() -> Self {
            Self::new()
        }
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct BlockRequest {
        pub request_type: u32,
        pub reserved: u32,
        pub sector: u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use virtio::*;

    struct FakePci;
    impl PciConfig for FakePci {
        fn read_u32(&mut self, address: PciAddress, offset: u8) -> u32 {
            if address.bus == 0 && address.device == 2 && address.function == 0 {
                match offset {
                    0 => 0x1000_1af4,
                    8 => 0x0200_0000,
                    12 => 0,
                    _ => 0,
                }
            } else {
                0xffff_ffff
            }
        }
        fn write_u32(&mut self, _: PciAddress, _: u8, _: u32) {}
    }

    #[test]
    fn discovers_virtio_network_device() {
        let mut devices = [None; 4];
        assert_eq!(enumerate(&mut FakePci, &mut devices), 1);
        assert_eq!(devices[0].unwrap().vendor, 0x1af4);
        assert_eq!(devices[0].unwrap().class, 2);
    }

    #[test]
    fn decodes_pci_bars() {
        assert_eq!(decode_bar(0xc001), Bar::Io { port: 0xc000 });
        assert_eq!(
            decode_bar(0xfebc_0008),
            Bar::Memory32 {
                address: 0xfebc_0000,
                prefetchable: true
            }
        );
    }

    #[test]
    fn feature_negotiation_requires_mandatory_bits() {
        assert!(Negotiation::new(0b1110, 0b0010, 0b1100).is_some());
        assert!(Negotiation::new(0b1000, 0b0010, 0).is_none());
    }

    #[test]
    fn queue_recycles_descriptors_after_completion() {
        let mut queue = SplitQueue::<2>::new();
        let id = queue
            .submit(Descriptor {
                address: 0x1000,
                length: 512,
                flags: 0,
                next: 0,
            })
            .unwrap();
        assert_eq!(queue.free_count(), 1);
        assert_eq!(queue.complete(id).unwrap().length, 512);
        assert_eq!(queue.free_count(), 2);
        assert!(queue.complete(id).is_err());
    }
}
