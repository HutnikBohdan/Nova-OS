use crate::{control::SetupPacket, UsbError};

pub const fn set_protocol(interface: u8, boot: bool) -> SetupPacket {
    SetupPacket {
        request_type: 0x21,
        request: 11,
        value: if boot { 0 } else { 1 },
        index: interface as u16,
        length: 0,
    }
}
pub const fn set_idle(interface: u8, duration_4ms: u8) -> SetupPacket {
    SetupPacket {
        request_type: 0x21,
        request: 10,
        value: (duration_4ms as u16) << 8,
        index: interface as u16,
        length: 0,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}
impl KeyboardReport {
    pub fn parse_boot(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 8 {
            return Err(UsbError::BufferTooSmall);
        }
        let mut keys = [0; 6];
        keys.copy_from_slice(&b[2..8]);
        Ok(Self {
            modifiers: b[0],
            keys,
        })
    }
    pub fn pressed(self, key: u8) -> bool {
        self.keys.contains(&key)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseReport {
    pub buttons: u8,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
}
impl MouseReport {
    pub fn parse_boot(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 3 {
            return Err(UsbError::BufferTooSmall);
        }
        Ok(Self {
            buttons: b[0],
            dx: b[1] as i8,
            dy: b[2] as i8,
            wheel: b.get(3).copied().unwrap_or(0) as i8,
        })
    }
}

/// Tracks rollover-safe report deltas without allocation.
pub fn newly_pressed(
    previous: KeyboardReport,
    current: KeyboardReport,
    out: &mut [u8; 6],
) -> usize {
    let mut n = 0;
    for key in current.keys {
        if key > 3 && !previous.pressed(key) && n < out.len() {
            out[n] = key;
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_delta() {
        let a = KeyboardReport::parse_boot(&[0, 0, 4, 0, 0, 0, 0, 0]).unwrap();
        let b = KeyboardReport::parse_boot(&[0, 0, 4, 5, 0, 0, 0, 0]).unwrap();
        let mut out = [0; 6];
        assert_eq!(newly_pressed(a, b, &mut out), 1);
        assert_eq!(out[0], 5);
    }
    #[test]
    fn signed_mouse() {
        let m = MouseReport::parse_boot(&[1, 255, 2, 254]).unwrap();
        assert_eq!((m.dx, m.wheel), (-1, -2));
    }
    #[test]
    fn boot_protocol_request() {
        assert_eq!(set_protocol(2, true).index, 2);
    }
}
