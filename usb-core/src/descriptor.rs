use crate::UsbError;

pub const DEVICE: u8 = 1;
pub const CONFIGURATION: u8 = 2;
pub const INTERFACE: u8 = 4;
pub const ENDPOINT: u8 = 5;
pub const HID: u8 = 0x21;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Descriptor<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}

pub struct Descriptors<'a> {
    bytes: &'a [u8],
    offset: usize,
    failed: bool,
}

impl<'a> Descriptors<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            failed: false,
        }
    }
}

impl<'a> Iterator for Descriptors<'a> {
    type Item = Result<Descriptor<'a>, UsbError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.offset == self.bytes.len() {
            return None;
        }
        if self.bytes.len() - self.offset < 2 {
            self.failed = true;
            return Some(Err(UsbError::InvalidDescriptor));
        }
        let len = self.bytes[self.offset] as usize;
        if len < 2 || self.offset + len > self.bytes.len() {
            self.failed = true;
            return Some(Err(UsbError::InvalidDescriptor));
        }
        let out = Descriptor {
            kind: self.bytes[self.offset + 1],
            bytes: &self.bytes[self.offset..self.offset + len],
        };
        self.offset += len;
        Some(Ok(out))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceDescriptor {
    pub usb_bcd: u16,
    pub class: u8,
    pub max_packet_0: u8,
    pub vendor: u16,
    pub product: u16,
    pub configurations: u8,
}
impl DeviceDescriptor {
    pub fn parse(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 18 || b[0] != 18 || b[1] != DEVICE {
            return Err(UsbError::InvalidDescriptor);
        }
        Ok(Self {
            usb_bcd: le16(b, 2),
            class: b[4],
            max_packet_0: b[7],
            vendor: le16(b, 8),
            product: le16(b, 10),
            configurations: b[17],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceDescriptor {
    pub number: u8,
    pub alternate: u8,
    pub endpoints: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}
impl InterfaceDescriptor {
    pub fn parse(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 9 || b[1] != INTERFACE {
            return Err(UsbError::InvalidDescriptor);
        }
        Ok(Self {
            number: b[2],
            alternate: b[3],
            endpoints: b[4],
            class: b[5],
            subclass: b[6],
            protocol: b[7],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet: u16,
    pub interval: u8,
}
impl EndpointDescriptor {
    pub fn parse(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 7 || b[1] != ENDPOINT {
            return Err(UsbError::InvalidDescriptor);
        }
        Ok(Self {
            address: b[2],
            attributes: b[3],
            max_packet: le16(b, 4) & 0x7ff,
            interval: b[6],
        })
    }
    pub const fn direction_in(self) -> bool {
        self.address & 0x80 != 0
    }
    pub const fn transfer_type(self) -> u8 {
        self.attributes & 3
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationChoice {
    pub value: u8,
    pub interface: InterfaceDescriptor,
    pub endpoint: EndpointDescriptor,
}

/// Select the first alternate-setting-zero interface and endpoint matching a
/// class/subclass/protocol tuple. Wildcards use `None`.
pub fn select_configuration(
    bytes: &[u8],
    class: u8,
    subclass: Option<u8>,
    protocol: Option<u8>,
) -> Result<ConfigurationChoice, UsbError> {
    let mut config = 0;
    let mut selected = None;
    for d in Descriptors::new(bytes) {
        let d = d?;
        match d.kind {
            CONFIGURATION if d.bytes.len() >= 9 => config = d.bytes[5],
            INTERFACE => {
                let i = InterfaceDescriptor::parse(d.bytes)?;
                selected = (i.alternate == 0
                    && i.class == class
                    && subclass.is_none_or(|v| i.subclass == v)
                    && protocol.is_none_or(|v| i.protocol == v))
                .then_some(i);
            }
            ENDPOINT if selected.is_some() => {
                return Ok(ConfigurationChoice {
                    value: config,
                    interface: selected.unwrap(),
                    endpoint: EndpointDescriptor::parse(d.bytes)?,
                })
            }
            _ => {}
        }
    }
    Err(UsbError::InvalidDescriptor)
}

const fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;
    const CFG: &[u8] = &[
        9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10,
    ];
    #[test]
    fn iter_rejects_zero_length() {
        assert_eq!(
            Descriptors::new(&[0, 1]).next().unwrap(),
            Err(UsbError::InvalidDescriptor)
        );
    }
    #[test]
    fn selects_hid_keyboard() {
        let c = select_configuration(CFG, 3, Some(1), Some(1)).unwrap();
        assert_eq!(c.value, 1);
        assert!(c.endpoint.direction_in());
    }
    #[test]
    fn device_fields() {
        let d = DeviceDescriptor::parse(&[
            18, 1, 0, 2, 0, 0, 0, 64, 0x34, 0x12, 0x78, 0x56, 0, 1, 1, 2, 3, 1,
        ])
        .unwrap();
        assert_eq!((d.vendor, d.product), (0x1234, 0x5678));
    }
}
