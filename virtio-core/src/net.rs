pub const GSO_NONE: u8 = 0;
pub const F_NEEDS_CSUM: u8 = 1;
pub const F_DATA_VALID: u8 = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct NetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub header_len: u16,
    pub gso_size: u16,
    pub checksum_start: u16,
    pub checksum_offset: u16,
    pub buffers: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetError {
    EmptyFrame,
    FrameTooLarge,
    QueueFull,
    Stale,
    TimedOut,
    Device,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PacketState {
    Free,
    DeviceRx { capacity: u32 },
    DeviceTx { length: u32, deadline: u64 },
    Ready { length: u32 },
    Failed(NetError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketId {
    pub slot: u16,
    pub generation: u16,
}

pub struct PacketQueue<const N: usize> {
    states: [PacketState; N],
    generations: [u16; N],
}
impl<const N: usize> PacketQueue<N> {
    pub const fn new() -> Self {
        Self {
            states: [PacketState::Free; N],
            generations: [0; N],
        }
    }
    pub fn provide_rx(&mut self, capacity: u32) -> Result<PacketId, NetError> {
        if capacity == 0 {
            return Err(NetError::EmptyFrame);
        }
        let (id, slot) = self.alloc()?;
        self.states[slot] = PacketState::DeviceRx { capacity };
        Ok(id)
    }
    pub fn submit_tx(
        &mut self,
        length: u32,
        mtu: u32,
        now: u64,
        timeout: u64,
    ) -> Result<PacketId, NetError> {
        if length == 0 {
            return Err(NetError::EmptyFrame);
        }
        if length > mtu {
            return Err(NetError::FrameTooLarge);
        }
        let (id, slot) = self.alloc()?;
        self.states[slot] = PacketState::DeviceTx {
            length,
            deadline: now.saturating_add(timeout),
        };
        Ok(id)
    }
    pub fn complete(&mut self, id: PacketId, length: u32) -> Result<(), NetError> {
        let slot = self.slot(id)?;
        match self.states[slot] {
            PacketState::DeviceRx { capacity } if length <= capacity => {
                self.states[slot] = PacketState::Ready { length }
            }
            PacketState::DeviceTx { length: sent, .. } if length <= sent => {
                self.states[slot] = PacketState::Ready { length }
            }
            PacketState::DeviceRx { .. } => {
                self.states[slot] = PacketState::Failed(NetError::FrameTooLarge)
            }
            _ => return Err(NetError::Stale),
        };
        Ok(())
    }
    pub fn poll(&mut self, id: PacketId, now: u64) -> Result<Option<u32>, NetError> {
        let slot = self.slot(id)?;
        match self.states[slot] {
            PacketState::Ready { length } => Ok(Some(length)),
            PacketState::Failed(error) => Err(error),
            PacketState::DeviceTx { deadline, .. } if now >= deadline => {
                self.states[slot] = PacketState::Failed(NetError::TimedOut);
                Err(NetError::TimedOut)
            }
            PacketState::DeviceRx { .. } | PacketState::DeviceTx { .. } => Ok(None),
            PacketState::Free => Err(NetError::Stale),
        }
    }
    pub fn release(&mut self, id: PacketId) -> Result<(), NetError> {
        let slot = self.slot(id)?;
        if !matches!(
            self.states[slot],
            PacketState::Ready { .. } | PacketState::Failed(_)
        ) {
            return Err(NetError::Stale);
        }
        self.states[slot] = PacketState::Free;
        Ok(())
    }
    pub fn reset_all(&mut self) {
        for state in &mut self.states {
            if !matches!(state, PacketState::Free) {
                *state = PacketState::Failed(NetError::Device);
            }
        }
    }
    fn alloc(&mut self) -> Result<(PacketId, usize), NetError> {
        let slot = self
            .states
            .iter()
            .position(|s| *s == PacketState::Free)
            .ok_or(NetError::QueueFull)?;
        let generation = self.generations[slot].wrapping_add(1).max(1);
        self.generations[slot] = generation;
        Ok((
            PacketId {
                slot: slot as u16,
                generation,
            },
            slot,
        ))
    }
    fn slot(&self, id: PacketId) -> Result<usize, NetError> {
        let slot = id.slot as usize;
        if slot >= N || self.generations[slot] != id.generation {
            Err(NetError::Stale)
        } else {
            Ok(slot)
        }
    }
}
impl<const N: usize> Default for PacketQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}
