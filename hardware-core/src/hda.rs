use crate::ParseError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodecVerb(pub u32);

impl CodecVerb {
    pub const fn new(codec: u8, node: u8, verb: u16, payload: u8) -> Result<Self, ParseError> {
        if codec > 0x0f || node > 0x7f || verb > 0x0fff {
            return Err(ParseError::InvalidValue);
        }
        Ok(Self(
            ((codec as u32) << 28) | ((node as u32) << 20) | ((verb as u32) << 8) | payload as u32,
        ))
    }
    pub const fn codec(self) -> u8 {
        (self.0 >> 28) as u8
    }
    pub const fn node(self) -> u8 {
        ((self.0 >> 20) & 0x7f) as u8
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferDescriptor {
    pub address: u64,
    pub length: u32,
    pub flags: u32,
}

impl BufferDescriptor {
    pub const INTERRUPT_ON_COMPLETION: u32 = 1;
    pub const fn new(address: u64, length: u32, interrupt: bool) -> Result<Self, ParseError> {
        if length == 0 || length > 0x1_0000 || address & 0x7f != 0 {
            return Err(ParseError::InvalidValue);
        }
        Ok(Self {
            address,
            length,
            flags: interrupt as u32,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    Full,
    Empty,
    Capacity,
}

/// Fixed-capacity software mirror of an HDA CORB/RIRB ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring<T: Copy + Default, const N: usize> {
    entries: [T; N],
    read: usize,
    write: usize,
    len: usize,
}

impl<T: Copy + Default, const N: usize> Ring<T, N> {
    pub fn new() -> Result<Self, RingError> {
        if N < 2 || !N.is_power_of_two() {
            return Err(RingError::Capacity);
        }
        Ok(Self {
            entries: [T::default(); N],
            read: 0,
            write: 0,
            len: 0,
        })
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn hardware_write_pointer(&self) -> u16 {
        self.write as u16
    }
    pub fn push(&mut self, item: T) -> Result<u16, RingError> {
        if self.len == N {
            return Err(RingError::Full);
        }
        let slot = self.write;
        self.entries[slot] = item;
        self.write = (self.write + 1) & (N - 1);
        self.len += 1;
        Ok(slot as u16)
    }
    pub fn pop(&mut self) -> Result<T, RingError> {
        if self.len == 0 {
            return Err(RingError::Empty);
        }
        let item = self.entries[self.read];
        self.read = (self.read + 1) & (N - 1);
        self.len -= 1;
        Ok(item)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat(pub u16);
impl StreamFormat {
    /// Creates the common linear PCM HDA stream format.
    pub const fn pcm(sample_rate: u32, bits: u8, channels: u8) -> Result<Self, ParseError> {
        if channels == 0 || channels > 16 {
            return Err(ParseError::InvalidValue);
        }
        let channel_field = channels as u16 - 1;
        let bit_field = match bits {
            8 => 0,
            16 => 1,
            20 => 2,
            24 => 3,
            32 => 4,
            _ => return Err(ParseError::Unsupported),
        };
        let rate_field = match sample_rate {
            8_000 => (0u16, 1u16, 5u16),
            16_000 => (0, 1, 2),
            32_000 => (0, 2, 2),
            44_100 => (1, 1, 1),
            48_000 => (0, 1, 1),
            88_200 => (1, 2, 1),
            96_000 => (0, 2, 1),
            192_000 => (0, 4, 1),
            _ => return Err(ParseError::Unsupported),
        };
        Ok(Self(
            (rate_field.0 << 14)
                | (rate_field.1 << 11)
                | (rate_field.2 << 8)
                | (bit_field << 4)
                | channel_field,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamState {
    Reset,
    Prepared,
    Running,
    Stopping,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamMachine {
    state: StreamState,
    descriptors: u16,
}
impl StreamMachine {
    pub const fn new() -> Self {
        Self {
            state: StreamState::Reset,
            descriptors: 0,
        }
    }
    pub const fn state(&self) -> StreamState {
        self.state
    }
    pub fn prepare(&mut self, descriptors: u16) -> Result<(), ParseError> {
        if self.state != StreamState::Reset || descriptors == 0 || descriptors > 256 {
            return Err(ParseError::InvalidValue);
        }
        self.descriptors = descriptors;
        self.state = StreamState::Prepared;
        Ok(())
    }
    pub fn start(&mut self) -> Result<(), ParseError> {
        if self.state != StreamState::Prepared {
            return Err(ParseError::InvalidValue);
        }
        self.state = StreamState::Running;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), ParseError> {
        if self.state != StreamState::Running {
            return Err(ParseError::InvalidValue);
        }
        self.state = StreamState::Stopping;
        Ok(())
    }
    pub fn halted(&mut self, error: bool) {
        if self.state == StreamState::Stopping {
            self.state = if error {
                StreamState::Error
            } else {
                StreamState::Reset
            };
            self.descriptors = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bdl_entry_matches_hardware() {
        assert_eq!(core::mem::size_of::<BufferDescriptor>(), 16);
    }
    #[test]
    fn codec_verb_roundtrip() {
        let v = CodecVerb::new(2, 0x34, 0x705, 0x80).unwrap();
        assert_eq!((v.codec(), v.node()), (2, 0x34));
    }
    #[test]
    fn bdl_requires_alignment() {
        assert!(BufferDescriptor::new(1, 4096, true).is_err());
        assert!(BufferDescriptor::new(0x1000, 4096, true).is_ok());
    }
    #[test]
    fn ring_wraps_and_checks_full() {
        let mut r = Ring::<u32, 4>::new().unwrap();
        for i in 0..4 {
            r.push(i).unwrap();
        }
        assert_eq!(r.push(9), Err(RingError::Full));
        assert_eq!(r.pop().unwrap(), 0);
        r.push(4).unwrap();
        for i in 1..5 {
            assert_eq!(r.pop().unwrap(), i);
        }
    }
    #[test]
    fn pcm_formats_are_distinct() {
        assert_ne!(
            StreamFormat::pcm(44_100, 16, 2).unwrap(),
            StreamFormat::pcm(48_000, 16, 2).unwrap()
        );
    }
    #[test]
    fn stream_lifecycle() {
        let mut s = StreamMachine::new();
        s.prepare(4).unwrap();
        s.start().unwrap();
        s.stop().unwrap();
        s.halted(false);
        assert_eq!(s.state(), StreamState::Reset);
    }
}
