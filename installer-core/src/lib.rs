#![no_std]

//! Power-loss-safe, allocation-free installation policy for Nova OS.
//! This crate plans disk changes but deliberately performs no host I/O.

pub const SECTOR_SIZE: u64 = 512;
pub const METADATA_SIZE: usize = 512;
pub const MAX_PARTITIONS: usize = 5;
pub const GPT_ENTRY_SIZE: usize = 128;
pub const GPT_ENTRY_COUNT: u32 = 128;
const GPT_ENTRY_SECTORS: u64 = (GPT_ENTRY_COUNT as u64 * GPT_ENTRY_SIZE as u64) / SECTOR_SIZE;
const METADATA_MAGIC: u64 = 0x4e4f_5641_494e_5354; // NOVAINST
const MANIFEST_MAGIC: u64 = 0x4e4f_5641_494d_4731; // NOVAIMG1

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    DiskTooSmall,
    InvalidGeometry,
    InvalidPlan,
    InvalidTransition,
    InvalidMetadata,
    MetadataConflict,
    UnsupportedMetadata,
    ImageTooLarge,
    HashMismatch,
    SignatureRejected,
    RollbackRejected,
    NoBootableSlot,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskGeometry {
    pub sectors: u64,
    pub logical_sector_size: u32,
    pub physical_sector_size: u32,
    pub alignment_sectors: u64,
}

impl DiskGeometry {
    pub fn new(
        sectors: u64,
        logical_sector_size: u32,
        physical_sector_size: u32,
    ) -> Result<Self, InstallError> {
        if sectors < 4096
            || logical_sector_size != 512
            || physical_sector_size < logical_sector_size
            || !physical_sector_size.is_power_of_two()
        {
            return Err(InstallError::InvalidGeometry);
        }
        let alignment_sectors = (1024 * 1024 / logical_sector_size).max(1) as u64;
        Ok(Self {
            sectors,
            logical_sector_size,
            physical_sector_size,
            alignment_sectors,
        })
    }

    pub const fn last_lba(self) -> u64 {
        self.sectors - 1
    }
    pub const fn first_usable_lba(self) -> u64 {
        2 + GPT_ENTRY_SECTORS
    }
    pub const fn last_usable_lba(self) -> u64 {
        self.sectors - GPT_ENTRY_SECTORS - 2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub const ZERO: Self = Self([0; 16]);
    pub const ESP: Self = Self([
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ]);
    pub const NOVA_SYSTEM: Self = Self(*b"NOVA_SYSTEM_TYPE");
    pub const NOVA_DATA: Self = Self(*b"NOVA_DATA_TYPE__");
    pub const NOVA_RECOVERY: Self = Self(*b"NOVA_RECOVERY___");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionRole {
    EfiSystem,
    SystemA,
    SystemB,
    UserData,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionSpec {
    pub role: PartitionRole,
    pub type_guid: Guid,
    pub unique_guid: Guid,
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
    pub name: [u16; 16],
}

impl PartitionSpec {
    pub const fn sector_count(self) -> u64 {
        self.last_lba - self.first_lba + 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionPlan {
    pub disk_guid: Guid,
    pub geometry: DiskGeometry,
    pub partitions: [PartitionSpec; MAX_PARTITIONS],
}

impl PartitionPlan {
    /// Produces ESP + equally sized A/B system slots + data + recovery.
    /// Sizes are MiB and all starts are 1 MiB aligned.
    pub fn standard(
        geometry: DiskGeometry,
        disk_guid: Guid,
        esp_mib: u64,
        system_mib: u64,
        recovery_mib: u64,
    ) -> Result<Self, InstallError> {
        if esp_mib < 64 || system_mib < 128 || recovery_mib < 64 {
            return Err(InstallError::InvalidPlan);
        }
        let a = geometry.alignment_sectors;
        let mut next = align_up(geometry.first_usable_lba(), a)?;
        let esp = allocate(
            &mut next,
            esp_mib,
            geometry,
            a,
            PartitionRole::EfiSystem,
            Guid::ESP,
            1,
        )?;
        let slot_a = allocate(
            &mut next,
            system_mib,
            geometry,
            a,
            PartitionRole::SystemA,
            Guid::NOVA_SYSTEM,
            2,
        )?;
        let slot_b = allocate(
            &mut next,
            system_mib,
            geometry,
            a,
            PartitionRole::SystemB,
            Guid::NOVA_SYSTEM,
            3,
        )?;
        let recovery_sectors = mib_to_sectors(recovery_mib)?;
        let recovery_first = align_down(
            geometry
                .last_usable_lba()
                .checked_sub(recovery_sectors - 1)
                .ok_or(InstallError::DiskTooSmall)?,
            a,
        );
        let data_first = align_up(next, a)?;
        if recovery_first <= data_first {
            return Err(InstallError::DiskTooSmall);
        }
        let data = spec(
            PartitionRole::UserData,
            Guid::NOVA_DATA,
            4,
            data_first,
            recovery_first - 1,
        );
        let recovery = spec(
            PartitionRole::Recovery,
            Guid::NOVA_RECOVERY,
            5,
            recovery_first,
            geometry.last_usable_lba(),
        );
        let plan = Self {
            disk_guid,
            geometry,
            partitions: [esp, slot_a, slot_b, data, recovery],
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), InstallError> {
        if self.disk_guid == Guid::ZERO {
            return Err(InstallError::InvalidPlan);
        }
        let mut previous_end = self.geometry.first_usable_lba() - 1;
        for p in &self.partitions {
            if p.unique_guid == Guid::ZERO
                || p.first_lba <= previous_end
                || p.first_lba % self.geometry.alignment_sectors != 0
                || p.last_lba < p.first_lba
                || p.last_lba > self.geometry.last_usable_lba()
            {
                return Err(InstallError::InvalidPlan);
            }
            previous_end = p.last_lba;
        }
        if self.partitions[1].sector_count() != self.partitions[2].sector_count() {
            return Err(InstallError::InvalidPlan);
        }
        Ok(())
    }

    pub fn protective_mbr(&self) -> [u8; 512] {
        let mut out = [0u8; 512];
        out[446 + 4] = 0xee;
        put_u32(&mut out, 446 + 8, 1);
        put_u32(
            &mut out,
            446 + 12,
            self.geometry.last_lba().min(u32::MAX as u64) as u32,
        );
        out[510] = 0x55;
        out[511] = 0xaa;
        out
    }

    pub fn encode_gpt_entry(&self, index: usize) -> Result<[u8; GPT_ENTRY_SIZE], InstallError> {
        let p = self
            .partitions
            .get(index)
            .ok_or(InstallError::InvalidPlan)?;
        let mut out = [0u8; GPT_ENTRY_SIZE];
        out[0..16].copy_from_slice(&p.type_guid.0);
        out[16..32].copy_from_slice(&p.unique_guid.0);
        put_u64(&mut out, 32, p.first_lba);
        put_u64(&mut out, 40, p.last_lba);
        put_u64(&mut out, 48, p.attributes);
        for (i, c) in p.name.iter().enumerate() {
            out[56 + i * 2..58 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        Ok(out)
    }

    pub fn primary_gpt_header(&self, entries_crc32: u32) -> [u8; 512] {
        self.gpt_header(1, self.geometry.last_lba(), 2, entries_crc32)
    }

    pub fn backup_gpt_header(&self, entries_crc32: u32) -> [u8; 512] {
        self.gpt_header(
            self.geometry.last_lba(),
            1,
            self.geometry.last_lba() - GPT_ENTRY_SECTORS,
            entries_crc32,
        )
    }

    fn gpt_header(&self, current: u64, backup: u64, entries: u64, entries_crc32: u32) -> [u8; 512] {
        let mut out = [0u8; 512];
        out[0..8].copy_from_slice(b"EFI PART");
        put_u32(&mut out, 8, 0x0001_0000);
        put_u32(&mut out, 12, 92);
        put_u64(&mut out, 24, current);
        put_u64(&mut out, 32, backup);
        put_u64(&mut out, 40, self.geometry.first_usable_lba());
        put_u64(&mut out, 48, self.geometry.last_usable_lba());
        out[56..72].copy_from_slice(&self.disk_guid.0);
        put_u64(&mut out, 72, entries);
        put_u32(&mut out, 80, GPT_ENTRY_COUNT);
        put_u32(&mut out, 84, GPT_ENTRY_SIZE as u32);
        put_u32(&mut out, 88, entries_crc32);
        let crc = crc32(&out[..92]);
        put_u32(&mut out, 16, crc);
        out
    }
}

fn allocate(
    next: &mut u64,
    mib: u64,
    g: DiskGeometry,
    align: u64,
    role: PartitionRole,
    ty: Guid,
    id: u8,
) -> Result<PartitionSpec, InstallError> {
    let first = align_up(*next, align)?;
    let count = mib_to_sectors(mib)?;
    let last = first
        .checked_add(count - 1)
        .ok_or(InstallError::ArithmeticOverflow)?;
    if last > g.last_usable_lba() {
        return Err(InstallError::DiskTooSmall);
    }
    *next = last
        .checked_add(1)
        .ok_or(InstallError::ArithmeticOverflow)?;
    Ok(spec(role, ty, id, first, last))
}

fn spec(role: PartitionRole, ty: Guid, id: u8, first: u64, last: u64) -> PartitionSpec {
    let mut guid = [0u8; 16];
    guid[0..4].copy_from_slice(b"NOVA");
    guid[15] = id;
    let mut name = [0u16; 16];
    let src: &[u16] = match role {
        PartitionRole::EfiSystem => &[
            0x004e, 0x006f, 0x0076, 0x0061, 0x0020, 0x0045, 0x0046, 0x0049,
        ],
        PartitionRole::SystemA => &[0x004e, 0x006f, 0x0076, 0x0061, 0x0020, 0x0041],
        PartitionRole::SystemB => &[0x004e, 0x006f, 0x0076, 0x0061, 0x0020, 0x0042],
        PartitionRole::UserData => &[0x0414, 0x0430, 0x043d, 0x0456], // Дані
        PartitionRole::Recovery => &[
            0x0412, 0x0456, 0x0434, 0x043d, 0x043e, 0x0432, 0x043b, 0x0435, 0x043d, 0x043d, 0x044f,
        ], // Відновлення
    };
    name[..src.len()].copy_from_slice(src);
    PartitionSpec {
        role,
        type_guid: ty,
        unique_guid: Guid(guid),
        first_lba: first,
        last_lba: last,
        attributes: 0,
        name,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Slot {
    A = 0,
    B = 1,
}
impl Slot {
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SlotStatus {
    Empty = 0,
    Ready = 1,
    Pending = 2,
    Good = 3,
    Bad = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRecord {
    pub status: SlotStatus,
    pub tries_remaining: u8,
    pub version: u64,
    pub rollback_index: u64,
    pub image_hash: [u8; 32],
}
impl SlotRecord {
    pub const fn empty() -> Self {
        Self {
            status: SlotStatus::Empty,
            tries_remaining: 0,
            version: 0,
            rollback_index: 0,
            image_hash: [0; 32],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InstallPhase {
    Idle = 0,
    Planned = 1,
    Writing = 2,
    Verifying = 3,
    Switching = 4,
    AwaitingBoot = 5,
    Committed = 6,
    RollingBack = 7,
    Failed = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallMetadata {
    pub generation: u64,
    pub phase: InstallPhase,
    pub active: Slot,
    pub target: Slot,
    pub slots: [SlotRecord; 2],
    pub minimum_rollback_index: u64,
    pub transaction_id: u64,
}

impl InstallMetadata {
    pub const fn fresh() -> Self {
        Self {
            generation: 1,
            phase: InstallPhase::Idle,
            active: Slot::A,
            target: Slot::B,
            slots: [SlotRecord::empty(), SlotRecord::empty()],
            minimum_rollback_index: 0,
            transaction_id: 0,
        }
    }
    pub const fn slot(&self, s: Slot) -> &SlotRecord {
        &self.slots[s as usize]
    }
    pub fn slot_mut(&mut self, s: Slot) -> &mut SlotRecord {
        &mut self.slots[s as usize]
    }

    pub fn begin(
        &mut self,
        transaction_id: u64,
        version: u64,
        rollback_index: u64,
        image_hash: [u8; 32],
    ) -> Result<(), InstallError> {
        if !matches!(
            self.phase,
            InstallPhase::Idle | InstallPhase::Committed | InstallPhase::Failed
        ) || transaction_id == 0
        {
            return Err(InstallError::InvalidTransition);
        }
        if rollback_index < self.minimum_rollback_index {
            return Err(InstallError::RollbackRejected);
        }
        self.target = self.active.other();
        self.transaction_id = transaction_id;
        self.slots[self.target as usize] = SlotRecord {
            status: SlotStatus::Empty,
            tries_remaining: 0,
            version,
            rollback_index,
            image_hash,
        };
        self.phase = InstallPhase::Planned;
        self.bump()
    }
    pub fn start_writing(&mut self) -> Result<(), InstallError> {
        self.transition(InstallPhase::Planned, InstallPhase::Writing)
    }
    pub fn finish_writing(&mut self) -> Result<(), InstallError> {
        self.transition(InstallPhase::Writing, InstallPhase::Verifying)
    }
    pub fn image_verified(&mut self) -> Result<(), InstallError> {
        if self.phase != InstallPhase::Verifying {
            return Err(InstallError::InvalidTransition);
        }
        self.slot_mut(self.target).status = SlotStatus::Ready;
        self.phase = InstallPhase::Switching;
        self.bump()
    }
    pub fn activate(&mut self, boot_attempts: u8) -> Result<(), InstallError> {
        if self.phase != InstallPhase::Switching || boot_attempts == 0 {
            return Err(InstallError::InvalidTransition);
        }
        let target = self.target;
        let rec = self.slot_mut(target);
        rec.status = SlotStatus::Pending;
        rec.tries_remaining = boot_attempts;
        self.active = target;
        self.phase = InstallPhase::AwaitingBoot;
        self.bump()
    }
    pub fn mark_boot_successful(&mut self) -> Result<(), InstallError> {
        if self.phase != InstallPhase::AwaitingBoot
            || self.slot(self.active).status != SlotStatus::Pending
        {
            return Err(InstallError::InvalidTransition);
        }
        let active = self.active;
        let rollback = self.slot(active).rollback_index;
        let rec = self.slot_mut(active);
        rec.status = SlotStatus::Good;
        rec.tries_remaining = 0;
        self.minimum_rollback_index = self.minimum_rollback_index.max(rollback);
        self.phase = InstallPhase::Committed;
        self.bump()
    }
    /// Must be persisted before booting the returned slot, so a reset consumes an attempt.
    pub fn prepare_boot(&mut self) -> Result<BootDecision, InstallError> {
        let active = self.active;
        let rec = self.slot_mut(active);
        match rec.status {
            SlotStatus::Good | SlotStatus::Ready => {
                self.bump()?;
                Ok(BootDecision::Boot(active))
            }
            SlotStatus::Pending if rec.tries_remaining > 0 => {
                rec.tries_remaining -= 1;
                self.bump()?;
                Ok(BootDecision::Boot(active))
            }
            SlotStatus::Pending | SlotStatus::Bad | SlotStatus::Empty => self.rollback(),
        }
    }
    pub fn fail_transaction(&mut self) -> Result<(), InstallError> {
        if matches!(self.phase, InstallPhase::Idle | InstallPhase::Committed) {
            return Err(InstallError::InvalidTransition);
        }
        let target = self.target;
        self.slot_mut(target).status = SlotStatus::Bad;
        self.phase = InstallPhase::Failed;
        self.bump()
    }
    pub fn recover_after_power_loss(&mut self) -> Result<RecoveryDecision, InstallError> {
        match self.phase {
            InstallPhase::Idle | InstallPhase::Committed => Ok(RecoveryDecision::ContinueBoot),
            InstallPhase::AwaitingBoot => Ok(RecoveryDecision::TryPendingBoot),
            InstallPhase::Switching if self.slot(self.target).status == SlotStatus::Ready => {
                Ok(RecoveryDecision::ResumeActivation)
            }
            InstallPhase::Planned
            | InstallPhase::Writing
            | InstallPhase::Verifying
            | InstallPhase::Switching => {
                let target = self.target;
                self.slot_mut(target).status = SlotStatus::Bad;
                self.phase = InstallPhase::Failed;
                self.bump()?;
                Ok(RecoveryDecision::DiscardIncomplete)
            }
            InstallPhase::RollingBack => {
                if matches!(
                    self.slot(self.active).status,
                    SlotStatus::Good | SlotStatus::Ready
                ) {
                    self.phase = InstallPhase::Committed;
                    self.bump()?;
                    Ok(RecoveryDecision::RolledBack(self.active))
                } else {
                    self.rollback()?;
                    Ok(RecoveryDecision::RolledBack(self.active))
                }
            }
            InstallPhase::Failed => {
                // A write/verification failure before activation must never poison
                // the still-running slot. Only roll back if active itself is bad.
                if matches!(
                    self.slot(self.active).status,
                    SlotStatus::Good | SlotStatus::Ready
                ) {
                    Ok(RecoveryDecision::ContinueBoot)
                } else {
                    self.rollback()?;
                    Ok(RecoveryDecision::RolledBack(self.active))
                }
            }
        }
    }
    fn rollback(&mut self) -> Result<BootDecision, InstallError> {
        let failed = self.active;
        self.slot_mut(failed).status = SlotStatus::Bad;
        let fallback = failed.other();
        if !matches!(
            self.slot(fallback).status,
            SlotStatus::Good | SlotStatus::Ready
        ) {
            self.phase = InstallPhase::Failed;
            self.bump()?;
            return Err(InstallError::NoBootableSlot);
        }
        self.phase = InstallPhase::RollingBack;
        self.active = fallback;
        self.target = failed;
        self.bump()?;
        self.phase = InstallPhase::Committed;
        self.bump()?;
        Ok(BootDecision::Rollback(fallback))
    }
    fn transition(&mut self, from: InstallPhase, to: InstallPhase) -> Result<(), InstallError> {
        if self.phase != from {
            return Err(InstallError::InvalidTransition);
        }
        self.phase = to;
        self.bump()
    }
    fn bump(&mut self) -> Result<(), InstallError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(InstallError::ArithmeticOverflow)?;
        Ok(())
    }

    pub fn encode(&self) -> [u8; METADATA_SIZE] {
        let mut out = [0u8; METADATA_SIZE];
        put_u64(&mut out, 0, METADATA_MAGIC);
        put_u32(&mut out, 8, 1);
        put_u64(&mut out, 16, self.generation);
        out[24] = self.phase as u8;
        out[25] = self.active as u8;
        out[26] = self.target as u8;
        put_u64(&mut out, 32, self.minimum_rollback_index);
        put_u64(&mut out, 40, self.transaction_id);
        encode_slot(&mut out, 64, &self.slots[0]);
        encode_slot(&mut out, 128, &self.slots[1]);
        let sum = crc32(&out[..METADATA_SIZE - 4]);
        put_u32(&mut out, METADATA_SIZE - 4, sum);
        out
    }
    pub fn decode(raw: &[u8; METADATA_SIZE]) -> Result<Self, InstallError> {
        if get_u64(raw, 0) != METADATA_MAGIC {
            return Err(InstallError::InvalidMetadata);
        }
        if get_u32(raw, 8) != 1 {
            return Err(InstallError::UnsupportedMetadata);
        }
        if get_u32(raw, METADATA_SIZE - 4) != crc32(&raw[..METADATA_SIZE - 4]) {
            return Err(InstallError::InvalidMetadata);
        }
        let phase = decode_phase(raw[24])?;
        let active = decode_slot_id(raw[25])?;
        let target = decode_slot_id(raw[26])?;
        if active == target {
            return Err(InstallError::InvalidMetadata);
        }
        Ok(Self {
            generation: get_u64(raw, 16),
            phase,
            active,
            target,
            slots: [decode_slot(raw, 64)?, decode_slot(raw, 128)?],
            minimum_rollback_index: get_u64(raw, 32),
            transaction_id: get_u64(raw, 40),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootDecision {
    Boot(Slot),
    Rollback(Slot),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDecision {
    ContinueBoot,
    TryPendingBoot,
    ResumeActivation,
    DiscardIncomplete,
    RolledBack(Slot),
}

/// Chooses the newest intact copy. Callers alternate writes between copy 0 and 1,
/// flush the selected copy, and never overwrite both in one transaction.
pub fn select_metadata(
    a: &[u8; METADATA_SIZE],
    b: &[u8; METADATA_SIZE],
) -> Result<(InstallMetadata, usize), InstallError> {
    match (InstallMetadata::decode(a), InstallMetadata::decode(b)) {
        (Ok(x), Ok(y)) => {
            if y.generation > x.generation {
                Ok((y, 1))
            } else if x.generation > y.generation {
                Ok((x, 0))
            } else if a == b {
                Ok((x, 0))
            } else {
                Err(InstallError::MetadataConflict)
            }
        }
        (Ok(x), Err(_)) => Ok((x, 0)),
        (Err(_), Ok(y)) => Ok((y, 1)),
        _ => Err(InstallError::InvalidMetadata),
    }
}
pub const fn next_metadata_copy(current_copy: usize) -> usize {
    (current_copy + 1) & 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageManifest {
    pub version: u64,
    pub rollback_index: u64,
    pub payload_bytes: u64,
    pub payload_hash: [u8; 32],
    pub key_id: [u8; 16],
    pub signature: [u8; 64],
}
impl ImageManifest {
    pub fn signed_message(&self) -> [u8; 72] {
        let mut out = [0u8; 72];
        put_u64(&mut out, 0, self.version);
        put_u64(&mut out, 8, self.rollback_index);
        put_u64(&mut out, 16, self.payload_bytes);
        out[24..56].copy_from_slice(&self.payload_hash);
        out[56..72].copy_from_slice(&self.key_id);
        out
    }
    pub fn encode(&self) -> [u8; 160] {
        let mut out = [0u8; 160];
        put_u64(&mut out, 0, MANIFEST_MAGIC);
        put_u32(&mut out, 8, 1);
        put_u64(&mut out, 16, self.version);
        put_u64(&mut out, 24, self.rollback_index);
        put_u64(&mut out, 32, self.payload_bytes);
        out[40..72].copy_from_slice(&self.payload_hash);
        out[72..88].copy_from_slice(&self.key_id);
        out[88..152].copy_from_slice(&self.signature);
        let c = crc32(&out[..156]);
        put_u32(&mut out, 156, c);
        out
    }
    pub fn decode(raw: &[u8; 160]) -> Result<Self, InstallError> {
        if get_u64(raw, 0) != MANIFEST_MAGIC
            || get_u32(raw, 8) != 1
            || get_u32(raw, 156) != crc32(&raw[..156])
        {
            return Err(InstallError::InvalidMetadata);
        }
        let mut hash = [0; 32];
        hash.copy_from_slice(&raw[40..72]);
        let mut key = [0; 16];
        key.copy_from_slice(&raw[72..88]);
        let mut sig = [0; 64];
        sig.copy_from_slice(&raw[88..152]);
        Ok(Self {
            version: get_u64(raw, 16),
            rollback_index: get_u64(raw, 24),
            payload_bytes: get_u64(raw, 32),
            payload_hash: hash,
            key_id: key,
            signature: sig,
        })
    }
}
pub trait ImageVerifier {
    fn verify_signature(&self, key_id: &[u8; 16], message: &[u8], signature: &[u8; 64]) -> bool;
}
pub fn verify_image(
    manifest: &ImageManifest,
    observed_bytes: u64,
    observed_hash: &[u8; 32],
    minimum_rollback_index: u64,
    verifier: &impl ImageVerifier,
) -> Result<(), InstallError> {
    if observed_bytes != manifest.payload_bytes {
        return Err(InstallError::ImageTooLarge);
    }
    if !constant_time_eq(observed_hash, &manifest.payload_hash) {
        return Err(InstallError::HashMismatch);
    }
    if manifest.rollback_index < minimum_rollback_index {
        return Err(InstallError::RollbackRejected);
    }
    if !verifier.verify_signature(
        &manifest.key_id,
        &manifest.signed_message(),
        &manifest.signature,
    ) {
        return Err(InstallError::SignatureRejected);
    }
    Ok(())
}

pub const fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    let mut i = 0;
    while i < bytes.len() {
        crc ^= bytes[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ ((0u32.wrapping_sub(crc & 1)) & 0xedb8_8320);
            bit += 1
        }
        i += 1
    }
    !crc
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for i in 0..a.len() {
        d |= a[i] ^ b[i]
    }
    d == 0
}
fn mib_to_sectors(m: u64) -> Result<u64, InstallError> {
    m.checked_mul(1024 * 1024 / SECTOR_SIZE)
        .ok_or(InstallError::ArithmeticOverflow)
}
fn align_up(v: u64, a: u64) -> Result<u64, InstallError> {
    v.checked_add(a - 1)
        .map(|n| n / a * a)
        .ok_or(InstallError::ArithmeticOverflow)
}
const fn align_down(v: u64, a: u64) -> u64 {
    v / a * a
}
fn put_u32(o: &mut [u8], p: usize, v: u32) {
    o[p..p + 4].copy_from_slice(&v.to_le_bytes())
}
fn put_u64(o: &mut [u8], p: usize, v: u64) {
    o[p..p + 8].copy_from_slice(&v.to_le_bytes())
}
fn get_u32(i: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(i[p..p + 4].try_into().unwrap())
}
fn get_u64(i: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(i[p..p + 8].try_into().unwrap())
}
fn encode_slot(o: &mut [u8], p: usize, s: &SlotRecord) {
    o[p] = s.status as u8;
    o[p + 1] = s.tries_remaining;
    put_u64(o, p + 8, s.version);
    put_u64(o, p + 16, s.rollback_index);
    o[p + 24..p + 56].copy_from_slice(&s.image_hash)
}
fn decode_slot(i: &[u8], p: usize) -> Result<SlotRecord, InstallError> {
    let status = match i[p] {
        0 => SlotStatus::Empty,
        1 => SlotStatus::Ready,
        2 => SlotStatus::Pending,
        3 => SlotStatus::Good,
        4 => SlotStatus::Bad,
        _ => return Err(InstallError::InvalidMetadata),
    };
    let mut h = [0; 32];
    h.copy_from_slice(&i[p + 24..p + 56]);
    Ok(SlotRecord {
        status,
        tries_remaining: i[p + 1],
        version: get_u64(i, p + 8),
        rollback_index: get_u64(i, p + 16),
        image_hash: h,
    })
}
fn decode_phase(v: u8) -> Result<InstallPhase, InstallError> {
    match v {
        0 => Ok(InstallPhase::Idle),
        1 => Ok(InstallPhase::Planned),
        2 => Ok(InstallPhase::Writing),
        3 => Ok(InstallPhase::Verifying),
        4 => Ok(InstallPhase::Switching),
        5 => Ok(InstallPhase::AwaitingBoot),
        6 => Ok(InstallPhase::Committed),
        7 => Ok(InstallPhase::RollingBack),
        8 => Ok(InstallPhase::Failed),
        _ => Err(InstallError::InvalidMetadata),
    }
}
fn decode_slot_id(v: u8) -> Result<Slot, InstallError> {
    match v {
        0 => Ok(Slot::A),
        1 => Ok(Slot::B),
        _ => Err(InstallError::InvalidMetadata),
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests {
    use super::*;
    fn guid() -> Guid {
        Guid(*b"NOVA_DISK_GUID__")
    }
    fn geometry() -> DiskGeometry {
        DiskGeometry::new(2 * 1024 * 1024 * 1024 / 512, 512, 4096).unwrap()
    }
    fn base() -> InstallMetadata {
        let mut m = InstallMetadata::fresh();
        m.slots[0].status = SlotStatus::Good;
        m.slots[0].version = 1;
        m
    }
    struct Accept(bool);
    impl ImageVerifier for Accept {
        fn verify_signature(&self, _: &[u8; 16], _: &[u8], _: &[u8; 64]) -> bool {
            self.0
        }
    }
    fn manifest() -> ImageManifest {
        ImageManifest {
            version: 2,
            rollback_index: 4,
            payload_bytes: 4096,
            payload_hash: [7; 32],
            key_id: [8; 16],
            signature: [9; 64],
        }
    }

    #[test]
    fn plans_aligned_non_overlapping_gpt() {
        let p = PartitionPlan::standard(geometry(), guid(), 64, 128, 64).unwrap();
        p.validate().unwrap();
        assert_eq!(
            p.partitions[1].sector_count(),
            p.partitions[2].sector_count()
        );
        for x in p.partitions {
            assert_eq!(x.first_lba % 2048, 0)
        }
        assert_eq!(&p.protective_mbr()[510..], &[0x55, 0xaa]);
    }
    #[test]
    fn rejects_too_small_disk() {
        let g = DiskGeometry::new(4096, 512, 512).unwrap();
        assert_eq!(
            PartitionPlan::standard(g, guid(), 64, 128, 64),
            Err(InstallError::DiskTooSmall)
        );
    }
    #[test]
    fn gpt_header_has_valid_crc() {
        let p = PartitionPlan::standard(geometry(), guid(), 64, 128, 64).unwrap();
        let h = p.primary_gpt_header(123);
        let mut clean = h;
        clean[16..20].fill(0);
        assert_eq!(get_u32(&h, 16), crc32(&clean[..92]));
        assert_eq!(&h[..8], b"EFI PART");
    }
    #[test]
    fn gpt_entry_encodes_utf16_and_lbas() {
        let p = PartitionPlan::standard(geometry(), guid(), 64, 128, 64).unwrap();
        let e = p.encode_gpt_entry(4).unwrap();
        assert_eq!(get_u64(&e, 32), p.partitions[4].first_lba);
        assert_eq!(u16::from_le_bytes([e[56], e[57]]), 0x0412);
    }
    #[test]
    fn full_ab_update_commits() {
        let mut m = base();
        m.begin(44, 2, 5, [3; 32]).unwrap();
        m.start_writing().unwrap();
        m.finish_writing().unwrap();
        m.image_verified().unwrap();
        m.activate(3).unwrap();
        assert_eq!(m.prepare_boot(), Ok(BootDecision::Boot(Slot::B)));
        m.mark_boot_successful().unwrap();
        assert_eq!(m.active, Slot::B);
        assert_eq!(m.slot(Slot::B).status, SlotStatus::Good);
        assert_eq!(m.minimum_rollback_index, 5);
    }
    #[test]
    fn exhausted_pending_slot_rolls_back() {
        let mut m = base();
        m.active = Slot::B;
        m.target = Slot::A;
        m.phase = InstallPhase::AwaitingBoot;
        m.slots[1] = SlotRecord {
            status: SlotStatus::Pending,
            tries_remaining: 0,
            version: 2,
            rollback_index: 2,
            image_hash: [1; 32],
        };
        assert_eq!(m.prepare_boot(), Ok(BootDecision::Rollback(Slot::A)));
        assert_eq!(m.slot(Slot::B).status, SlotStatus::Bad);
    }
    #[test]
    fn refuses_rollback_below_fuse() {
        let mut m = base();
        m.minimum_rollback_index = 10;
        assert_eq!(
            m.begin(1, 2, 9, [0; 32]),
            Err(InstallError::RollbackRejected)
        );
    }
    #[test]
    fn metadata_roundtrip_and_corruption_detection() {
        let mut m = base();
        m.begin(7, 8, 9, [4; 32]).unwrap();
        let mut raw = m.encode();
        assert_eq!(InstallMetadata::decode(&raw).unwrap(), m);
        raw[70] ^= 1;
        assert_eq!(
            InstallMetadata::decode(&raw),
            Err(InstallError::InvalidMetadata)
        );
    }
    #[test]
    fn dual_copy_selects_newest_valid() {
        let old = base();
        let mut new = old;
        new.generation = 9;
        let a = old.encode();
        let mut b = new.encode();
        assert_eq!(select_metadata(&a, &b).unwrap().1, 1);
        b[20] ^= 0x80;
        assert_eq!(select_metadata(&a, &b).unwrap().1, 0);
    }
    #[test]
    fn power_loss_while_writing_discards_target() {
        let mut m = base();
        m.begin(1, 2, 1, [2; 32]).unwrap();
        m.start_writing().unwrap();
        assert_eq!(
            m.recover_after_power_loss(),
            Ok(RecoveryDecision::DiscardIncomplete)
        );
        assert_eq!(m.slot(Slot::B).status, SlotStatus::Bad);
    }
    #[test]
    fn power_loss_after_verify_resumes_activation() {
        let mut m = base();
        m.begin(1, 2, 1, [2; 32]).unwrap();
        m.start_writing().unwrap();
        m.finish_writing().unwrap();
        m.image_verified().unwrap();
        assert_eq!(
            m.recover_after_power_loss(),
            Ok(RecoveryDecision::ResumeActivation)
        );
    }
    #[test]
    fn failed_inactive_write_preserves_active_slot() {
        let mut m = base();
        m.begin(1, 2, 1, [2; 32]).unwrap();
        m.start_writing().unwrap();
        m.fail_transaction().unwrap();
        assert_eq!(
            m.recover_after_power_loss(),
            Ok(RecoveryDecision::ContinueBoot)
        );
        assert_eq!(m.slot(Slot::A).status, SlotStatus::Good);
        assert_eq!(m.slot(Slot::B).status, SlotStatus::Bad);
    }
    #[test]
    fn equal_generation_split_brain_is_rejected() {
        let a = base();
        let mut b = a;
        b.transaction_id = 99;
        assert_eq!(
            select_metadata(&a.encode(), &b.encode()),
            Err(InstallError::MetadataConflict)
        );
    }
    #[test]
    fn manifest_roundtrip_verifies_hash_signature_and_floor() {
        let x = manifest();
        let raw = x.encode();
        let d = ImageManifest::decode(&raw).unwrap();
        assert_eq!(d, x);
        assert_eq!(verify_image(&d, 4096, &[7; 32], 4, &Accept(true)), Ok(()));
        assert_eq!(
            verify_image(&d, 4096, &[6; 32], 4, &Accept(true)),
            Err(InstallError::HashMismatch)
        );
        assert_eq!(
            verify_image(&d, 4096, &[7; 32], 5, &Accept(true)),
            Err(InstallError::RollbackRejected)
        );
        assert_eq!(
            verify_image(&d, 4096, &[7; 32], 4, &Accept(false)),
            Err(InstallError::SignatureRejected)
        );
    }
    #[test]
    fn invalid_state_transition_rejected() {
        let mut m = base();
        assert_eq!(m.start_writing(), Err(InstallError::InvalidTransition));
    }
    #[test]
    fn no_bootable_slot_is_reported() {
        let mut m = InstallMetadata::fresh();
        m.phase = InstallPhase::AwaitingBoot;
        m.slots[0].status = SlotStatus::Pending;
        assert_eq!(m.prepare_boot(), Err(InstallError::NoBootableSlot));
    }
    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
