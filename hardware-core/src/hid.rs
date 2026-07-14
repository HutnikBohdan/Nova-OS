use crate::ParseError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

impl KeyboardReport {
    pub fn parse_boot(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < 8 {
            return Err(ParseError::Truncated);
        }
        let mut keys = [0; 6];
        keys.copy_from_slice(&bytes[2..8]);
        if keys.iter().any(|&key| (1..=3).contains(&key)) {
            return Err(ParseError::InvalidValue);
        }
        Ok(Self {
            modifiers: bytes[0],
            keys,
        })
    }

    pub fn pressed_since<const N: usize>(&self, old: &Self) -> KeyChanges<N> {
        let mut out = KeyChanges::new();
        for &key in &self.keys {
            if key != 0 && !old.keys.contains(&key) {
                let _ = out.push(key);
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseReport {
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

impl MouseReport {
    pub fn parse_boot(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < 3 {
            return Err(ParseError::Truncated);
        }
        Ok(Self {
            buttons: bytes[0],
            x: bytes[1] as i8,
            y: bytes[2] as i8,
            wheel: bytes.get(3).copied().unwrap_or(0) as i8,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyChanges<const N: usize> {
    values: [u8; N],
    len: usize,
}

impl<const N: usize> KeyChanges<N> {
    pub const fn new() -> Self {
        Self {
            values: [0; N],
            len: 0,
        }
    }
    pub fn push(&mut self, value: u8) -> Result<(), ParseError> {
        if self.len == N {
            return Err(ParseError::Capacity);
        }
        self.values[self.len] = value;
        self.len += 1;
        Ok(())
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.values[..self.len]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Main,
    Global,
    Local,
    Reserved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorItem {
    pub kind: ItemKind,
    pub tag: u8,
    pub data: u32,
    pub size: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportDescriptor<const N: usize> {
    items: [DescriptorItem; N],
    len: usize,
}

const EMPTY_ITEM: DescriptorItem = DescriptorItem {
    kind: ItemKind::Reserved,
    tag: 0,
    data: 0,
    size: 0,
};

impl<const N: usize> ReportDescriptor<N> {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let mut out = Self {
            items: [EMPTY_ITEM; N],
            len: 0,
        };
        let mut offset = 0;
        while offset < bytes.len() {
            let prefix = bytes[offset];
            offset += 1;
            if prefix == 0xfe {
                if offset + 2 > bytes.len() {
                    return Err(ParseError::Truncated);
                }
                let length = bytes[offset] as usize;
                if offset + 2 + length > bytes.len() {
                    return Err(ParseError::Truncated);
                }
                return Err(ParseError::Unsupported); // long items are reserved by HID 1.11
            }
            let size = match prefix & 3 {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 4,
            };
            if offset + size > bytes.len() {
                return Err(ParseError::Truncated);
            }
            if out.len == N {
                return Err(ParseError::Capacity);
            }
            let mut data_bytes = [0u8; 4];
            data_bytes[..size].copy_from_slice(&bytes[offset..offset + size]);
            offset += size;
            let kind = match (prefix >> 2) & 3 {
                0 => ItemKind::Main,
                1 => ItemKind::Global,
                2 => ItemKind::Local,
                _ => ItemKind::Reserved,
            };
            out.items[out.len] = DescriptorItem {
                kind,
                tag: prefix >> 4,
                data: u32::from_le_bytes(data_bytes),
                size: size as u8,
            };
            out.len += 1;
        }
        Ok(out)
    }
    pub fn items(&self) -> &[DescriptorItem] {
        &self.items[..self.len]
    }
    pub fn first_global(&self, tag: u8) -> Option<u32> {
        self.items()
            .iter()
            .find(|i| i.kind == ItemKind::Global && i.tag == tag)
            .map(|i| i.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_delta() {
        let old = KeyboardReport::parse_boot(&[0, 0, 4, 0, 0, 0, 0, 0]).unwrap();
        let new = KeyboardReport::parse_boot(&[2, 0, 4, 5, 0, 0, 0, 0]).unwrap();
        assert_eq!(new.pressed_since::<6>(&old).as_slice(), &[5]);
    }
    #[test]
    fn rejects_rollover_error() {
        assert!(KeyboardReport::parse_boot(&[0, 0, 1, 0, 0, 0, 0, 0]).is_err());
    }
    #[test]
    fn mouse_wheel_is_optional() {
        assert_eq!(
            MouseReport::parse_boot(&[1, 255, 2]).unwrap(),
            MouseReport {
                buttons: 1,
                x: -1,
                y: 2,
                wheel: 0
            }
        );
    }
    #[test]
    fn parses_short_descriptor_items() {
        // Usage Page (Generic Desktop), Usage (Keyboard), Collection (Application), End Collection.
        let d = ReportDescriptor::<8>::parse(&[0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0xc0]).unwrap();
        assert_eq!(d.items().len(), 4);
        assert_eq!(d.first_global(0), Some(1));
        assert_eq!(d.items()[1].kind, ItemKind::Local);
    }
    #[test]
    fn descriptor_capacity_is_enforced() {
        assert_eq!(
            ReportDescriptor::<1>::parse(&[0xc0, 0xc0]),
            Err(ParseError::Capacity)
        );
    }
}
