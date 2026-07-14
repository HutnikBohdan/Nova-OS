pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2;
pub const DESC_F_INDIRECT: u16 = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Descriptor {
    pub address: u64,
    pub length: u32,
    pub flags: u16,
    pub next: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    Empty,
    Full,
    InvalidDescriptor,
    NotOwned,
    StaleToken,
    ChainTooLong,
    AlreadyComplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub head: u16,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Owner {
    Free,
    Driver(u16),
    Device(u16),
    Used(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Slot {
    descriptor: Descriptor,
    owner: Owner,
}
impl Slot {
    const EMPTY: Self = Self {
        descriptor: Descriptor {
            address: 0,
            length: 0,
            flags: 0,
            next: 0,
        },
        owner: Owner::Free,
    };
}

/// Driver-side split virtqueue metadata. `N` is fixed at compile time.
pub struct SplitQueue<const N: usize> {
    slots: [Slot; N],
    generation: u16,
    available: [u16; N],
    used: [(u16, u32); N],
    avail_head: u16,
    avail_tail: u16,
    used_head: u16,
    used_tail: u16,
}

impl<const N: usize> SplitQueue<N> {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; N],
            generation: 1,
            available: [0; N],
            used: [(0, 0); N],
            avail_head: 0,
            avail_tail: 0,
            used_head: 0,
            used_tail: 0,
        }
    }

    pub fn submit(&mut self, chain: &[Descriptor]) -> Result<Token, QueueError> {
        if chain.is_empty() {
            return Err(QueueError::Empty);
        }
        if chain.len() > N || N > u16::MAX as usize {
            return Err(QueueError::ChainTooLong);
        }
        if chain.iter().any(|descriptor| descriptor.length == 0) {
            return Err(QueueError::InvalidDescriptor);
        }
        if self.avail_tail.wrapping_sub(self.avail_head) as usize >= N {
            return Err(QueueError::Full);
        }
        let mut indices = [0u16; N];
        let mut found = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.owner == Owner::Free {
                indices[found] = index as u16;
                found += 1;
                if found == chain.len() {
                    break;
                }
            }
        }
        if found != chain.len() {
            return Err(QueueError::Full);
        }
        let generation = self.generation;
        self.generation = self.generation.wrapping_add(1).max(1);
        for (position, descriptor) in chain.iter().enumerate() {
            let index = indices[position];
            let mut value = *descriptor;
            if position + 1 < chain.len() {
                value.flags |= DESC_F_NEXT;
                value.next = indices[position + 1];
            } else {
                value.flags &= !DESC_F_NEXT;
                value.next = 0;
            }
            self.slots[index as usize] = Slot {
                descriptor: value,
                owner: Owner::Driver(generation),
            };
        }
        let head = indices[0];
        for index in indices[..chain.len()].iter() {
            self.slots[*index as usize].owner = Owner::Device(generation);
        }
        self.available[self.avail_tail as usize % N] = head;
        self.avail_tail = self.avail_tail.wrapping_add(1);
        Ok(Token { head, generation })
    }

    /// Testable device-side dequeue; a hardware adapter mirrors the same indices to DMA.
    pub fn device_take(&mut self) -> Result<Token, QueueError> {
        if self.avail_head == self.avail_tail {
            return Err(QueueError::Empty);
        }
        let head = self.available[self.avail_head as usize % N];
        self.avail_head = self.avail_head.wrapping_add(1);
        match self.slots[head as usize].owner {
            Owner::Device(generation) => Ok(Token { head, generation }),
            _ => Err(QueueError::NotOwned),
        }
    }

    pub fn device_complete(&mut self, token: Token, written: u32) -> Result<(), QueueError> {
        self.validate_chain(token, Owner::Device(token.generation))?;
        if self.used_tail.wrapping_sub(self.used_head) as usize >= N {
            return Err(QueueError::Full);
        }
        self.mark_chain(token, Owner::Used(token.generation))?;
        self.used[self.used_tail as usize % N] = (token.head, written);
        self.used_tail = self.used_tail.wrapping_add(1);
        Ok(())
    }

    pub fn pop_used(&mut self) -> Result<(Token, u32), QueueError> {
        if self.used_head == self.used_tail {
            return Err(QueueError::Empty);
        }
        let (head, written) = self.used[self.used_head as usize % N];
        let generation = match self.slots[head as usize].owner {
            Owner::Used(g) => g,
            _ => return Err(QueueError::NotOwned),
        };
        let token = Token { head, generation };
        self.free_chain(token)?;
        self.used_head = self.used_head.wrapping_add(1);
        Ok((token, written))
    }

    fn validate_chain(&self, token: Token, expected: Owner) -> Result<(), QueueError> {
        let mut index = token.head as usize;
        for _ in 0..N {
            let slot = self.slots.get(index).ok_or(QueueError::InvalidDescriptor)?;
            if slot.owner != expected {
                return Err(if matches!(slot.owner, Owner::Free) {
                    QueueError::StaleToken
                } else {
                    QueueError::NotOwned
                });
            }
            if slot.descriptor.flags & DESC_F_NEXT == 0 {
                return Ok(());
            }
            index = slot.descriptor.next as usize;
        }
        Err(QueueError::ChainTooLong)
    }
    fn mark_chain(&mut self, token: Token, owner: Owner) -> Result<(), QueueError> {
        let mut index = token.head as usize;
        for _ in 0..N {
            let next = self.slots[index].descriptor.next as usize;
            let last = self.slots[index].descriptor.flags & DESC_F_NEXT == 0;
            self.slots[index].owner = owner;
            if last {
                return Ok(());
            }
            index = next;
        }
        Err(QueueError::ChainTooLong)
    }
    fn free_chain(&mut self, token: Token) -> Result<(), QueueError> {
        self.validate_chain(token, Owner::Used(token.generation))?;
        let mut index = token.head as usize;
        for _ in 0..N {
            let next = self.slots[index].descriptor.next as usize;
            let last = self.slots[index].descriptor.flags & DESC_F_NEXT == 0;
            self.slots[index] = Slot::EMPTY;
            if last {
                return Ok(());
            }
            index = next;
        }
        Err(QueueError::ChainTooLong)
    }
}

impl<const N: usize> Default for SplitQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub const PACKED_F_AVAIL: u16 = 1 << 7;
pub const PACKED_F_USED: u16 = 1 << 15;

/// Packed ring wrap/generation tracker with explicit ownership.
pub struct PackedQueue<const N: usize> {
    owners: [Owner; N],
    next_avail: u16,
    next_used: u16,
    avail_wrap: bool,
    used_wrap: bool,
    generation: u16,
}
impl<const N: usize> PackedQueue<N> {
    pub const fn new() -> Self {
        Self {
            owners: [Owner::Free; N],
            next_avail: 0,
            next_used: 0,
            avail_wrap: true,
            used_wrap: true,
            generation: 1,
        }
    }
    pub fn submit_one(&mut self) -> Result<(Token, u16), QueueError> {
        if N == 0 || self.owners[self.next_avail as usize] != Owner::Free {
            return Err(QueueError::Full);
        }
        let token = Token {
            head: self.next_avail,
            generation: self.generation,
        };
        self.owners[token.head as usize] = Owner::Device(token.generation);
        let flags = if self.avail_wrap {
            PACKED_F_AVAIL
        } else {
            PACKED_F_USED
        };
        self.advance_avail();
        Ok((token, flags))
    }
    pub fn complete_one(&mut self, token: Token) -> Result<u16, QueueError> {
        if token.head != self.next_used {
            return Err(QueueError::NotOwned);
        }
        if self.owners[token.head as usize] != Owner::Device(token.generation) {
            return Err(QueueError::StaleToken);
        }
        self.owners[token.head as usize] = Owner::Used(token.generation);
        Ok(if self.used_wrap {
            PACKED_F_AVAIL | PACKED_F_USED
        } else {
            0
        })
    }
    pub fn reclaim_one(&mut self, token: Token) -> Result<(), QueueError> {
        if token.head != self.next_used
            || self.owners[token.head as usize] != Owner::Used(token.generation)
        {
            return Err(QueueError::StaleToken);
        }
        self.owners[token.head as usize] = Owner::Free;
        self.advance_used();
        Ok(())
    }
    fn advance_avail(&mut self) {
        self.next_avail += 1;
        if self.next_avail as usize == N {
            self.next_avail = 0;
            self.avail_wrap = !self.avail_wrap;
            self.generation = self.generation.wrapping_add(1).max(1);
        }
    }
    fn advance_used(&mut self) {
        self.next_used += 1;
        if self.next_used as usize == N {
            self.next_used = 0;
            self.used_wrap = !self.used_wrap;
        }
    }
}
impl<const N: usize> Default for PackedQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}
