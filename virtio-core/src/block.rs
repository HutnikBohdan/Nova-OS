pub const T_IN: u32 = 0;
pub const T_OUT: u32 = 1;
pub const T_FLUSH: u32 = 4;
pub const T_DISCARD: u32 = 11;
pub const T_WRITE_ZEROES: u32 = 13;
pub const S_OK: u8 = 0;
pub const S_IOERR: u8 = 1;
pub const S_UNSUPP: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RequestHeader {
    pub request_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DiscardRange {
    pub sector: u64,
    pub sectors: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Read { sector: u64, bytes: u32 },
    Write { sector: u64, bytes: u32 },
    Flush,
    Discard(DiscardRange),
    WriteZeroes(DiscardRange),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestState {
    Free,
    Submitted { operation: Operation, deadline: u64 },
    Completed(Result<u32, BlockError>),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    QueueFull,
    InvalidLength,
    OutOfRange,
    Unsupported,
    Io,
    TimedOut,
    Stale,
}

pub struct RequestTable<const N: usize> {
    slots: [RequestState; N],
    generations: [u16; N],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestId {
    pub slot: u16,
    pub generation: u16,
}

impl<const N: usize> RequestTable<N> {
    pub const fn new() -> Self {
        Self {
            slots: [RequestState::Free; N],
            generations: [0; N],
        }
    }
    pub fn submit(
        &mut self,
        op: Operation,
        now: u64,
        timeout: u64,
        capacity_sectors: u64,
        sector_size: u32,
    ) -> Result<RequestId, BlockError> {
        validate(op, capacity_sectors, sector_size)?;
        let slot = self
            .slots
            .iter()
            .position(|s| *s == RequestState::Free)
            .ok_or(BlockError::QueueFull)?;
        let generation = self.generations[slot].wrapping_add(1).max(1);
        self.generations[slot] = generation;
        self.slots[slot] = RequestState::Submitted {
            operation: op,
            deadline: now.saturating_add(timeout),
        };
        Ok(RequestId {
            slot: slot as u16,
            generation,
        })
    }
    pub fn complete(&mut self, id: RequestId, status: u8, bytes: u32) -> Result<(), BlockError> {
        let slot = self.slot(id)?;
        if !matches!(self.slots[slot], RequestState::Submitted { .. }) {
            return Err(BlockError::Stale);
        }
        let result = match status {
            S_OK => Ok(bytes),
            S_UNSUPP => Err(BlockError::Unsupported),
            _ => Err(BlockError::Io),
        };
        self.slots[slot] = RequestState::Completed(result);
        Ok(())
    }
    pub fn poll(
        &mut self,
        id: RequestId,
        now: u64,
    ) -> Result<Option<Result<u32, BlockError>>, BlockError> {
        let slot = self.slot(id)?;
        match self.slots[slot] {
            RequestState::Submitted { deadline, .. } if now >= deadline => {
                self.slots[slot] = RequestState::Completed(Err(BlockError::TimedOut));
                Ok(Some(Err(BlockError::TimedOut)))
            }
            RequestState::Submitted { .. } => Ok(None),
            RequestState::Completed(value) => Ok(Some(value)),
            RequestState::Free => Err(BlockError::Stale),
        }
    }
    pub fn release(&mut self, id: RequestId) -> Result<(), BlockError> {
        let slot = self.slot(id)?;
        if !matches!(self.slots[slot], RequestState::Completed(_)) {
            return Err(BlockError::Stale);
        }
        self.slots[slot] = RequestState::Free;
        Ok(())
    }
    pub fn reset_all(&mut self) {
        for state in &mut self.slots {
            if matches!(state, RequestState::Submitted { .. }) {
                *state = RequestState::Completed(Err(BlockError::Io));
            }
        }
    }
    fn slot(&self, id: RequestId) -> Result<usize, BlockError> {
        let slot = id.slot as usize;
        if slot >= N || self.generations[slot] != id.generation {
            Err(BlockError::Stale)
        } else {
            Ok(slot)
        }
    }
}
impl<const N: usize> Default for RequestTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn header(operation: Operation) -> RequestHeader {
    match operation {
        Operation::Read { sector, .. } => RequestHeader {
            request_type: T_IN,
            reserved: 0,
            sector,
        },
        Operation::Write { sector, .. } => RequestHeader {
            request_type: T_OUT,
            reserved: 0,
            sector,
        },
        Operation::Flush => RequestHeader {
            request_type: T_FLUSH,
            reserved: 0,
            sector: 0,
        },
        Operation::Discard(range) => RequestHeader {
            request_type: T_DISCARD,
            reserved: 0,
            sector: range.sector,
        },
        Operation::WriteZeroes(range) => RequestHeader {
            request_type: T_WRITE_ZEROES,
            reserved: 0,
            sector: range.sector,
        },
    }
}

fn validate(op: Operation, capacity: u64, sector_size: u32) -> Result<(), BlockError> {
    let (sector, bytes) = match op {
        Operation::Read { sector, bytes } | Operation::Write { sector, bytes } => (sector, bytes),
        Operation::Discard(r) | Operation::WriteZeroes(r) => (
            r.sector,
            r.sectors
                .checked_mul(sector_size)
                .ok_or(BlockError::OutOfRange)?,
        ),
        Operation::Flush => return Ok(()),
    };
    if bytes == 0 || sector_size == 0 || bytes % sector_size != 0 {
        return Err(BlockError::InvalidLength);
    }
    let sectors = (bytes / sector_size) as u64;
    if sector.checked_add(sectors).is_none() || sector + sectors > capacity {
        return Err(BlockError::OutOfRange);
    }
    Ok(())
}
