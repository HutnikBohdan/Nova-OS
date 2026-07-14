use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

/// Kernel object identity. Object zero is reserved and never granted.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u64);

/// Rights attached to a capability. Unknown bits are rejected when importing
/// capabilities across a syscall boundary.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const SIGNAL: Self = Self(1 << 4);
    pub const DUPLICATE: Self = Self(1 << 5);
    pub const TRANSFER: Self = Self(1 << 6);
    pub const INSPECT: Self = Self(1 << 7);
    pub const ALL: Self = Self((1 << 8) - 1);

    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}
impl BitAndAssign for Rights {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// Unforgeable authority stored in a process-local capability slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub object: ObjectId,
    pub rights: Rights,
    /// Changes whenever a slot is revoked and reused, preventing stale handles.
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    TableFull,
    InvalidHandle,
    StaleHandle,
    MissingRights,
    RightsEscalation,
    InvalidObject,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    capability: Option<Capability>,
    generation: u32,
}

impl Slot {
    const EMPTY: Self = Self {
        capability: None,
        generation: 1,
    };
}

/// A handle encodes both the slot and its generation.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityHandle(pub u64);

impl CapabilityHandle {
    const fn new(index: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | index as u64)
    }
    pub const fn slot(self) -> usize {
        self.0 as u32 as usize
    }
    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

pub struct CapabilityTable<const N: usize> {
    slots: [Slot; N],
}

impl<const N: usize> CapabilityTable<N> {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; N],
        }
    }

    pub fn insert(
        &mut self,
        object: ObjectId,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        if object.0 == 0 {
            return Err(CapabilityError::InvalidObject);
        }
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.capability.is_none() {
                let capability = Capability {
                    object,
                    rights,
                    generation: slot.generation,
                };
                slot.capability = Some(capability);
                return Ok(CapabilityHandle::new(index, slot.generation));
            }
        }
        Err(CapabilityError::TableFull)
    }

    pub fn resolve(
        &self,
        handle: CapabilityHandle,
        required: Rights,
    ) -> Result<&Capability, CapabilityError> {
        let slot = self
            .slots
            .get(handle.slot())
            .ok_or(CapabilityError::InvalidHandle)?;
        if slot.generation != handle.generation() {
            return Err(CapabilityError::StaleHandle);
        }
        let capability = slot
            .capability
            .as_ref()
            .ok_or(CapabilityError::InvalidHandle)?;
        if !capability.rights.contains(required) {
            return Err(CapabilityError::MissingRights);
        }
        Ok(capability)
    }

    pub fn revoke(&mut self, handle: CapabilityHandle) -> Result<Capability, CapabilityError> {
        let slot = self
            .slots
            .get_mut(handle.slot())
            .ok_or(CapabilityError::InvalidHandle)?;
        if slot.generation != handle.generation() {
            return Err(CapabilityError::StaleHandle);
        }
        let old = slot
            .capability
            .take()
            .ok_or(CapabilityError::InvalidHandle)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        Ok(old)
    }

    /// Copies authority while only allowing rights to be removed.
    pub fn derive(
        &mut self,
        source: CapabilityHandle,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        let capability = *self.resolve(source, Rights::DUPLICATE)?;
        if !capability.rights.contains(rights) {
            return Err(CapabilityError::RightsEscalation);
        }
        self.insert(capability.object, rights)
    }
}

impl<const N: usize> Default for CapabilityTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_cannot_escalate_rights() {
        let mut table = CapabilityTable::<4>::new();
        let source = table
            .insert(ObjectId(7), Rights::READ | Rights::DUPLICATE)
            .unwrap();
        let child = table.derive(source, Rights::READ).unwrap();
        assert_eq!(
            table.resolve(child, Rights::READ).unwrap().object,
            ObjectId(7)
        );
        assert_eq!(
            table.derive(source, Rights::WRITE),
            Err(CapabilityError::RightsEscalation)
        );
    }

    #[test]
    fn revoked_handles_cannot_be_reused() {
        let mut table = CapabilityTable::<1>::new();
        let old = table.insert(ObjectId(1), Rights::READ).unwrap();
        table.revoke(old).unwrap();
        let new = table.insert(ObjectId(2), Rights::READ).unwrap();
        assert_ne!(old, new);
        assert_eq!(
            table.resolve(old, Rights::READ),
            Err(CapabilityError::StaleHandle)
        );
    }
}
