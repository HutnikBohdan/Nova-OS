//! Append-only, crash-recoverable action journal for Nova Guardian.
//! The filesystem owns the durable slots; this module owns validation and state transitions.

use crate::{Error, Result};

pub const RECORD_SIZE: usize = 96;
const MAGIC: [u8; 4] = *b"NVAJ";
const VERSION: u8 = 1;
const INVERSE_CAPACITY: usize = 44;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ActionKind {
    FileWrite = 1,
    FileCreate = 2,
    FileDelete = 3,
    Build = 4,
    Launch = 5,
    Setting = 6,
    Package = 7,
    SystemUpdate = 8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Capability {
    Files = 1,
    Build = 2,
    Processes = 3,
    Settings = 4,
    Packages = 5,
    System = 6,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Phase {
    Planned = 1,
    Approved = 2,
    Applied = 3,
    Verified = 4,
    Failed = 5,
    Reverted = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record {
    pub sequence: u32,
    pub action_id: u32,
    pub task_id: u32,
    pub kind: ActionKind,
    pub capability: Capability,
    pub phase: Phase,
    pub before_hash: u64,
    pub after_hash: u64,
    pub previous_digest: u64,
    inverse: [u8; INVERSE_CAPACITY],
    inverse_len: u8,
}
impl Record {
    const EMPTY: Self = Self {
        sequence: 0,
        action_id: 0,
        task_id: 0,
        kind: ActionKind::FileWrite,
        capability: Capability::Files,
        phase: Phase::Planned,
        before_hash: 0,
        after_hash: 0,
        previous_digest: 0,
        inverse: [0; INVERSE_CAPACITY],
        inverse_len: 0,
    };
    pub fn inverse(&self) -> &[u8] {
        &self.inverse[..self.inverse_len as usize]
    }
}

pub struct Journal<const N: usize> {
    records: [Record; N],
    len: usize,
    next_action: u32,
    digest: u64,
}
impl<const N: usize> Journal<N> {
    pub const fn new() -> Self {
        Self {
            records: [Record::EMPTY; N],
            len: 0,
            next_action: 1,
            digest: 0,
        }
    }
    pub fn records(&self) -> &[Record] {
        &self.records[..self.len]
    }
    pub fn plan(
        &mut self,
        task_id: u32,
        kind: ActionKind,
        capability: Capability,
        before_hash: u64,
        inverse: &[u8],
    ) -> Result<u32> {
        if inverse.len() > INVERSE_CAPACITY {
            return Err(Error::OutputTooSmall);
        }
        let mut record = Record {
            sequence: 0,
            action_id: self.next_action,
            task_id,
            kind,
            capability,
            phase: Phase::Planned,
            before_hash,
            after_hash: 0,
            previous_digest: 0,
            inverse: [0; INVERSE_CAPACITY],
            inverse_len: inverse.len() as u8,
        };
        record.inverse[..inverse.len()].copy_from_slice(inverse);
        let action = self.next_action;
        self.next_action = self
            .next_action
            .checked_add(1)
            .ok_or(Error::IntegerOverflow)?;
        self.append(record)?;
        Ok(action)
    }
    pub fn approve(&mut self, action: u32) -> Result<()> {
        self.transition(action, Phase::Approved, 0)
    }
    pub fn applied(&mut self, action: u32, after_hash: u64) -> Result<()> {
        self.transition(action, Phase::Applied, after_hash)
    }
    pub fn verify(&mut self, action: u32, observed_hash: u64) -> Result<()> {
        let latest = self.latest(action)?;
        if latest.after_hash != observed_hash {
            return Err(Error::CallMismatch);
        }
        self.transition(action, Phase::Verified, observed_hash)
    }
    pub fn fail(&mut self, action: u32) -> Result<()> {
        self.transition(action, Phase::Failed, 0)
    }
    pub fn revert(&mut self, action: u32, restored_hash: u64) -> Result<()> {
        let latest = self.latest(action)?;
        if latest.before_hash != restored_hash {
            return Err(Error::CallMismatch);
        }
        self.transition(action, Phase::Reverted, restored_hash)
    }
    pub fn recovery_action(&self) -> Option<&Record> {
        self.records().iter().rev().find(|record| {
            matches!(record.phase, Phase::Approved | Phase::Applied)
                && !self.records()[record.sequence as usize..]
                    .iter()
                    .any(|later| {
                        later.action_id == record.action_id
                            && matches!(later.phase, Phase::Verified | Phase::Reverted)
                    })
        })
    }
    pub fn encode_slot(&self, index: usize, output: &mut [u8; RECORD_SIZE]) -> Result<()> {
        let record = self.records().get(index).ok_or(Error::UnexpectedEof)?;
        encode(record, output);
        Ok(())
    }
    pub fn recover(slots: &[[u8; RECORD_SIZE]; N]) -> Result<Self> {
        let mut journal = Self::new();
        let mut empty_seen = false;
        for slot in slots {
            if slot.iter().all(|byte| *byte == 0) {
                empty_seen = true;
                continue;
            }
            if empty_seen {
                return Err(Error::CorruptJournal);
            }
            let record = decode(slot)?;
            if record.sequence != journal.len as u32 + 1 || record.previous_digest != journal.digest
            {
                return Err(Error::CorruptJournal);
            }
            journal.digest = digest(slot);
            journal.next_action = journal.next_action.max(record.action_id.saturating_add(1));
            journal.records[journal.len] = record;
            journal.len += 1;
        }
        Ok(journal)
    }
    fn transition(&mut self, action: u32, phase: Phase, after_hash: u64) -> Result<()> {
        let mut record = *self.latest(action)?;
        let valid = matches!(
            (record.phase, phase),
            (Phase::Planned, Phase::Approved | Phase::Failed)
                | (Phase::Approved, Phase::Applied | Phase::Failed)
                | (
                    Phase::Applied,
                    Phase::Verified | Phase::Failed | Phase::Reverted
                )
                | (Phase::Failed, Phase::Reverted)
        );
        if !valid {
            return Err(Error::InvalidTransition);
        }
        record.phase = phase;
        if after_hash != 0 {
            record.after_hash = after_hash;
        }
        self.append(record)
    }
    fn latest(&self, action: u32) -> Result<&Record> {
        self.records()
            .iter()
            .rev()
            .find(|record| record.action_id == action)
            .ok_or(Error::CallMismatch)
    }
    fn append(&mut self, mut record: Record) -> Result<()> {
        if self.len == N {
            return Err(Error::JournalCapacity);
        }
        record.sequence = self.len as u32 + 1;
        record.previous_digest = self.digest;
        let mut slot = [0u8; RECORD_SIZE];
        encode(&record, &mut slot);
        self.digest = digest(&slot);
        self.records[self.len] = record;
        self.len += 1;
        Ok(())
    }
}
impl<const N: usize> Default for Journal<N> {
    fn default() -> Self {
        Self::new()
    }
}

fn encode(record: &Record, out: &mut [u8; RECORD_SIZE]) {
    *out = [0; RECORD_SIZE];
    out[..4].copy_from_slice(&MAGIC);
    out[4] = VERSION;
    out[5] = record.kind as u8;
    out[6] = record.capability as u8;
    out[7] = record.phase as u8;
    out[8..12].copy_from_slice(&record.sequence.to_le_bytes());
    out[12..16].copy_from_slice(&record.action_id.to_le_bytes());
    out[16..20].copy_from_slice(&record.task_id.to_le_bytes());
    out[20] = record.inverse_len;
    out[24..32].copy_from_slice(&record.before_hash.to_le_bytes());
    out[32..40].copy_from_slice(&record.after_hash.to_le_bytes());
    out[40..48].copy_from_slice(&record.previous_digest.to_le_bytes());
    out[48..96 - 4].copy_from_slice(&record.inverse[..44]);
    let crc = crc32(&out[..92]);
    out[92..96].copy_from_slice(&crc.to_le_bytes());
}
fn decode(input: &[u8; RECORD_SIZE]) -> Result<Record> {
    if input[..4] != MAGIC
        || input[4] != VERSION
        || crc32(&input[..92])
            != u32::from_le_bytes(
                input[92..96]
                    .try_into()
                    .map_err(|_| Error::CorruptJournal)?,
            )
    {
        return Err(Error::CorruptJournal);
    }
    let inverse_len = input[20] as usize;
    if inverse_len > 44 {
        return Err(Error::CorruptJournal);
    }
    let mut inverse = [0u8; INVERSE_CAPACITY];
    inverse[..inverse_len].copy_from_slice(&input[48..48 + inverse_len]);
    Ok(Record {
        sequence: u32_at(input, 8)?,
        action_id: u32_at(input, 12)?,
        task_id: u32_at(input, 16)?,
        kind: kind(input[5])?,
        capability: capability(input[6])?,
        phase: phase(input[7])?,
        before_hash: u64_at(input, 24)?,
        after_hash: u64_at(input, 32)?,
        previous_digest: u64_at(input, 40)?,
        inverse,
        inverse_len: inverse_len as u8,
    })
}
fn digest(slot: &[u8; RECORD_SIZE]) -> u64 {
    slot[..92].iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    })
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b[at..at + 4]
            .try_into()
            .map_err(|_| Error::CorruptJournal)?,
    ))
}
fn u64_at(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b[at..at + 8]
            .try_into()
            .map_err(|_| Error::CorruptJournal)?,
    ))
}
fn kind(v: u8) -> Result<ActionKind> {
    match v {
        1 => Ok(ActionKind::FileWrite),
        2 => Ok(ActionKind::FileCreate),
        3 => Ok(ActionKind::FileDelete),
        4 => Ok(ActionKind::Build),
        5 => Ok(ActionKind::Launch),
        6 => Ok(ActionKind::Setting),
        7 => Ok(ActionKind::Package),
        8 => Ok(ActionKind::SystemUpdate),
        _ => Err(Error::CorruptJournal),
    }
}
fn capability(v: u8) -> Result<Capability> {
    match v {
        1 => Ok(Capability::Files),
        2 => Ok(Capability::Build),
        3 => Ok(Capability::Processes),
        4 => Ok(Capability::Settings),
        5 => Ok(Capability::Packages),
        6 => Ok(Capability::System),
        _ => Err(Error::CorruptJournal),
    }
}
fn phase(v: u8) -> Result<Phase> {
    match v {
        1 => Ok(Phase::Planned),
        2 => Ok(Phase::Approved),
        3 => Ok(Phase::Applied),
        4 => Ok(Phase::Verified),
        5 => Ok(Phase::Failed),
        6 => Ok(Phase::Reverted),
        _ => Err(Error::CorruptJournal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_survives_serialization() {
        let mut j = Journal::<8>::new();
        let id = j
            .plan(7, ActionKind::FileWrite, Capability::Files, 10, b"old")
            .unwrap();
        j.approve(id).unwrap();
        j.applied(id, 20).unwrap();
        j.verify(id, 20).unwrap();
        let mut slots = [[0u8; RECORD_SIZE]; 8];
        for (i, slot) in slots.iter_mut().enumerate().take(j.records().len()) {
            j.encode_slot(i, slot).unwrap();
        }
        let recovered = Journal::recover(&slots).unwrap();
        assert_eq!(recovered.records().last().unwrap().phase, Phase::Verified);
        assert!(recovered.recovery_action().is_none());
    }
    #[test]
    fn power_loss_requests_inverse_recovery() {
        let mut j = Journal::<8>::new();
        let id = j
            .plan(1, ActionKind::Setting, Capability::Settings, 55, b"restore")
            .unwrap();
        j.approve(id).unwrap();
        j.applied(id, 77).unwrap();
        assert_eq!(j.recovery_action().unwrap().inverse(), b"restore");
        j.revert(id, 55).unwrap();
        assert!(j.recovery_action().is_none());
    }
    #[test]
    fn tamper_is_rejected() {
        let mut j = Journal::<4>::new();
        j.plan(1, ActionKind::Build, Capability::Build, 1, b"")
            .unwrap();
        let mut slots = [[0u8; RECORD_SIZE]; 4];
        j.encode_slot(0, &mut slots[0]).unwrap();
        slots[0][30] ^= 1;
        assert!(matches!(
            Journal::recover(&slots),
            Err(Error::CorruptJournal)
        ));
    }
}
