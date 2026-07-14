#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free deployment orchestration for Nova OS.
//!
//! Policy and I/O are intentionally separated.  The state machines expose one
//! durable, idempotent step at a time, so a platform service can persist the
//! journal before acknowledging completion to the UI.

pub const SECTOR_SIZE: usize = 512;
pub const MAX_DISKS: usize = 16;
pub const UKRAINIAN_LOCALE: Locale = Locale::new(*b"uk-UA", *b"UA", *b"Europe/Kyiv____");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    Firmware,
    Journal,
    InventoryFull,
    DiskNotFound,
    DiskNotEligible,
    ConfirmationRequired,
    WrongConfirmation,
    InvalidPlan,
    InvalidImage,
    HashMismatch,
    InvalidState,
    NoInactiveSlot,
    RollbackUnavailable,
    FaultInjected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiskId(pub [u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Nvme,
    Sata,
    Virtio,
    Usb,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskInfo {
    pub id: DiskId,
    pub sectors: u64,
    pub logical_sector_size: u32,
    pub bus: Bus,
    pub removable: bool,
    pub read_only: bool,
    pub system_disk: bool,
}

impl DiskInfo {
    pub fn eligibility(self, minimum_sectors: u64) -> Eligibility {
        if self.read_only {
            Eligibility::ReadOnly
        } else if self.logical_sector_size != SECTOR_SIZE as u32 {
            Eligibility::UnsupportedSectorSize
        } else if self.sectors < minimum_sectors {
            Eligibility::TooSmall
        } else if self.removable {
            Eligibility::RemovableNeedsExplicitOptIn
        } else {
            Eligibility::Eligible
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    ReadOnly,
    UnsupportedSectorSize,
    TooSmall,
    RemovableNeedsExplicitOptIn,
}

pub trait DiskInventorySource {
    fn count(&self) -> usize;
    fn disk(&self, index: usize) -> Result<DiskInfo, Error>;
}

#[derive(Debug, Clone, Copy)]
pub struct Inventory {
    disks: [DiskInfo; MAX_DISKS],
    len: usize,
}

impl Inventory {
    pub const fn empty() -> Self {
        const EMPTY: DiskInfo = DiskInfo {
            id: DiskId([0; 16]),
            sectors: 0,
            logical_sector_size: 0,
            bus: Bus::Unknown,
            removable: false,
            read_only: true,
            system_disk: false,
        };
        Self {
            disks: [EMPTY; MAX_DISKS],
            len: 0,
        }
    }

    pub fn scan(source: &impl DiskInventorySource) -> Result<Self, Error> {
        if source.count() > MAX_DISKS {
            return Err(Error::InventoryFull);
        }
        let mut result = Self::empty();
        for index in 0..source.count() {
            result.disks[index] = source.disk(index)?;
            result.len += 1;
        }
        Ok(result)
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, index: usize) -> Option<DiskInfo> {
        if index < self.len {
            Some(self.disks[index])
        } else {
            None
        }
    }
    pub fn by_id(&self, id: DiskId) -> Option<DiskInfo> {
        self.disks[..self.len]
            .iter()
            .copied()
            .find(|disk| disk.id == id)
    }
}

/// A token bound to the exact disk, size and UI challenge nonce.  It prevents
/// stale confirmation dialogs from authorizing a different destructive write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationToken(pub [u8; 32]);

pub fn confirmation_token(disk: DiskInfo, nonce: [u8; 16]) -> ConfirmationToken {
    let mut hash = Sha256::new();
    hash.update(b"NOVA-DEPLOY-DESTRUCTIVE-V1");
    hash.update(&disk.id.0);
    hash.update(&disk.sectors.to_le_bytes());
    hash.update(&disk.logical_sector_size.to_le_bytes());
    hash.update(&nonce);
    ConfirmationToken(hash.finish())
}

pub fn verify_confirmation(expected: ConfirmationToken, actual: ConfirmationToken) -> bool {
    let mut difference = 0u8;
    for index in 0..32 {
        difference |= expected.0[index] ^ actual.0[index];
    }
    difference == 0
}

pub trait BlockDevice {
    fn info(&self) -> DiskInfo;
    fn read_sector(&mut self, lba: u64, sector: &mut [u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn write_sector(&mut self, lba: u64, sector: &[u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
}

pub trait Firmware {
    fn install_boot_entry(&mut self, disk: DiskId, esp_start: u64) -> Result<(), Error>;
    fn select_boot_slot(&mut self, slot: Slot) -> Result<(), Error>;
}

pub trait ImageSource {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn expected_sha256(&self) -> [u8; 32];
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<usize, Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
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
pub struct Locale {
    pub language: [u8; 5],
    pub keyboard: [u8; 2],
    pub timezone: [u8; 15],
}

impl Locale {
    pub const fn new(language: [u8; 5], keyboard: [u8; 2], timezone: [u8; 15]) -> Self {
        Self {
            language,
            keyboard,
            timezone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallPlan {
    pub disk: DiskId,
    pub minimum_sectors: u64,
    pub primary_gpt_lba: u64,
    pub backup_gpt_lba: u64,
    pub esp_start: u64,
    pub slot_a_start: u64,
    pub slot_a_sectors: u64,
    pub recovery_start: u64,
    pub locale: Locale,
    pub allow_removable: bool,
}

impl InstallPlan {
    pub fn validate(self, disk: DiskInfo, image_len: u64) -> Result<(), Error> {
        if self.disk != disk.id
            || self.primary_gpt_lba == self.backup_gpt_lba
            || self.backup_gpt_lba >= disk.sectors
            || self.esp_start >= disk.sectors
            || self.recovery_start >= disk.sectors
            || self.slot_a_start >= disk.sectors
            || self.slot_a_sectors == 0
            || self.slot_a_start + self.slot_a_sectors > disk.sectors
            || image_len > self.slot_a_sectors * SECTOR_SIZE as u64
        {
            return Err(Error::InvalidPlan);
        }
        match disk.eligibility(self.minimum_sectors) {
            Eligibility::Eligible => Ok(()),
            Eligibility::RemovableNeedsExplicitOptIn if self.allow_removable => Ok(()),
            _ => Err(Error::DiskNotEligible),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    Idle,
    Confirmed,
    ProtectiveMbr,
    PrimaryGpt,
    BackupGpt,
    Image,
    ImageVerified,
    BootEntry,
    Setup,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Downloading,
    Downloaded,
    Staging,
    Staged,
    Switching,
    Trial,
    Committed,
    RollingBack,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    None,
    Install,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Journal {
    pub sequence: u64,
    pub operation: Operation,
    pub install_phase: InstallPhase,
    pub update_phase: UpdatePhase,
    pub disk: DiskId,
    pub byte_offset: u64,
    pub active_slot: Slot,
    pub target_slot: Slot,
    pub trial_boots_left: u8,
    pub hash: Sha256,
}

impl Journal {
    pub const fn empty() -> Self {
        Self {
            sequence: 0,
            operation: Operation::None,
            install_phase: InstallPhase::Idle,
            update_phase: UpdatePhase::Idle,
            disk: DiskId([0; 16]),
            byte_offset: 0,
            active_slot: Slot::A,
            target_slot: Slot::B,
            trial_boots_left: 0,
            hash: Sha256::new(),
        }
    }
}

pub trait JournalStore {
    fn load(&mut self) -> Result<Journal, Error>;
    fn commit(&mut self, journal: &Journal) -> Result<(), Error>;
    fn clear(&mut self) -> Result<(), Error>;
}

fn checkpoint(store: &mut impl JournalStore, journal: &mut Journal) -> Result<(), Error> {
    journal.sequence = journal.sequence.wrapping_add(1);
    store.commit(journal)
}

pub trait GptWriter {
    fn protective_mbr(
        &mut self,
        disk: &mut impl BlockDevice,
        plan: InstallPlan,
    ) -> Result<(), Error>;
    fn primary(&mut self, disk: &mut impl BlockDevice, plan: InstallPlan) -> Result<(), Error>;
    fn backup(&mut self, disk: &mut impl BlockDevice, plan: InstallPlan) -> Result<(), Error>;
}

pub trait SetupWriter {
    fn write_locale(
        &mut self,
        disk: &mut impl BlockDevice,
        plan: InstallPlan,
        locale: Locale,
    ) -> Result<(), Error>;
}

/// Starts an installation only after an exact destructive confirmation.
pub fn begin_install(
    store: &mut impl JournalStore,
    disk: DiskInfo,
    plan: InstallPlan,
    image_len: u64,
    nonce: [u8; 16],
    token: ConfirmationToken,
) -> Result<Journal, Error> {
    plan.validate(disk, image_len)?;
    if !verify_confirmation(confirmation_token(disk, nonce), token) {
        return Err(Error::WrongConfirmation);
    }
    let mut journal = Journal::empty();
    journal.operation = Operation::Install;
    journal.install_phase = InstallPhase::Confirmed;
    journal.disk = disk.id;
    checkpoint(store, &mut journal)?;
    Ok(journal)
}

/// Performs one resumable install unit.  Every transition follows a device
/// flush barrier and a journal checkpoint.
#[allow(clippy::too_many_arguments)]
pub fn advance_install(
    journal: &mut Journal,
    store: &mut impl JournalStore,
    disk: &mut impl BlockDevice,
    firmware: &mut impl Firmware,
    gpt: &mut impl GptWriter,
    setup: &mut impl SetupWriter,
    image: &mut impl ImageSource,
    plan: InstallPlan,
) -> Result<InstallPhase, Error> {
    if journal.operation != Operation::Install || journal.disk != disk.info().id {
        return Err(Error::InvalidState);
    }
    match journal.install_phase {
        InstallPhase::Confirmed => {
            gpt.protective_mbr(disk, plan)?;
            disk.flush()?;
            journal.install_phase = InstallPhase::ProtectiveMbr;
        }
        InstallPhase::ProtectiveMbr => {
            gpt.primary(disk, plan)?;
            disk.flush()?;
            journal.install_phase = InstallPhase::PrimaryGpt;
        }
        InstallPhase::PrimaryGpt => {
            gpt.backup(disk, plan)?;
            disk.flush()?;
            journal.install_phase = InstallPhase::BackupGpt;
        }
        InstallPhase::BackupGpt => {
            journal.hash = Sha256::new();
            journal.byte_offset = 0;
            journal.install_phase = InstallPhase::Image;
        }
        InstallPhase::Image => {
            stream_one_sector(disk, image, plan.slot_a_start, journal)?;
            if journal.byte_offset == image.len() {
                disk.flush()?;
                let digest = journal.hash.finish();
                if !digest_eq(digest, image.expected_sha256()) {
                    return Err(Error::HashMismatch);
                }
                journal.install_phase = InstallPhase::ImageVerified;
            }
        }
        InstallPhase::ImageVerified => {
            firmware.install_boot_entry(plan.disk, plan.esp_start)?;
            journal.install_phase = InstallPhase::BootEntry;
        }
        InstallPhase::BootEntry => {
            setup.write_locale(disk, plan, plan.locale)?;
            disk.flush()?;
            journal.install_phase = InstallPhase::Setup;
        }
        InstallPhase::Setup => {
            firmware.select_boot_slot(Slot::A)?;
            journal.install_phase = InstallPhase::Complete;
        }
        InstallPhase::Complete => return Ok(InstallPhase::Complete),
        InstallPhase::Idle => return Err(Error::ConfirmationRequired),
    }
    checkpoint(store, journal)?;
    Ok(journal.install_phase)
}

fn stream_one_sector(
    disk: &mut impl BlockDevice,
    image: &mut impl ImageSource,
    start_lba: u64,
    journal: &mut Journal,
) -> Result<(), Error> {
    let remaining = image
        .len()
        .checked_sub(journal.byte_offset)
        .ok_or(Error::InvalidImage)?;
    if remaining == 0 {
        return Ok(());
    }
    let length = core::cmp::min(remaining, SECTOR_SIZE as u64) as usize;
    let mut sector = [0u8; SECTOR_SIZE];
    let read = image.read_at(journal.byte_offset, &mut sector[..length])?;
    if read != length {
        return Err(Error::InvalidImage);
    }
    journal.hash.update(&sector[..length]);
    disk.write_sector(
        start_lba + journal.byte_offset / SECTOR_SIZE as u64,
        &sector,
    )?;
    journal.byte_offset += length as u64;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdatePlan {
    pub disk: DiskId,
    pub active: Slot,
    pub inactive_start: u64,
    pub inactive_sectors: u64,
    pub trial_boots: u8,
}

pub fn begin_update(store: &mut impl JournalStore, plan: UpdatePlan) -> Result<Journal, Error> {
    if plan.trial_boots == 0 || plan.inactive_sectors == 0 {
        return Err(Error::InvalidPlan);
    }
    let mut journal = Journal::empty();
    journal.operation = Operation::Update;
    journal.update_phase = UpdatePhase::Downloading;
    journal.disk = plan.disk;
    journal.active_slot = plan.active;
    journal.target_slot = plan.active.other();
    journal.trial_boots_left = plan.trial_boots;
    checkpoint(store, &mut journal)?;
    Ok(journal)
}

/// Download and stage use the same verified source but remain separate durable
/// phases so a network service may cache first and expose progress safely.
pub fn mark_downloaded(journal: &mut Journal, store: &mut impl JournalStore) -> Result<(), Error> {
    if journal.operation != Operation::Update || journal.update_phase != UpdatePhase::Downloading {
        return Err(Error::InvalidState);
    }
    journal.update_phase = UpdatePhase::Downloaded;
    journal.byte_offset = 0;
    journal.hash = Sha256::new();
    checkpoint(store, journal)
}

pub fn advance_update(
    journal: &mut Journal,
    store: &mut impl JournalStore,
    disk: &mut impl BlockDevice,
    firmware: &mut impl Firmware,
    image: &mut impl ImageSource,
    plan: UpdatePlan,
) -> Result<UpdatePhase, Error> {
    if journal.operation != Operation::Update
        || disk.info().id != plan.disk
        || journal.target_slot == journal.active_slot
    {
        return Err(Error::InvalidState);
    }
    match journal.update_phase {
        UpdatePhase::Downloaded => {
            journal.update_phase = UpdatePhase::Staging;
        }
        UpdatePhase::Staging => {
            if image.len() > plan.inactive_sectors * SECTOR_SIZE as u64 {
                return Err(Error::InvalidImage);
            }
            stream_one_sector(disk, image, plan.inactive_start, journal)?;
            if journal.byte_offset == image.len() {
                disk.flush()?;
                if !digest_eq(journal.hash.finish(), image.expected_sha256()) {
                    return Err(Error::HashMismatch);
                }
                journal.update_phase = UpdatePhase::Staged;
            }
        }
        UpdatePhase::Staged => {
            journal.update_phase = UpdatePhase::Switching;
        }
        UpdatePhase::Switching => {
            firmware.select_boot_slot(journal.target_slot)?;
            journal.update_phase = UpdatePhase::Trial;
        }
        UpdatePhase::Trial | UpdatePhase::Committed | UpdatePhase::RolledBack => {
            return Ok(journal.update_phase);
        }
        _ => return Err(Error::InvalidState),
    }
    checkpoint(store, journal)?;
    Ok(journal.update_phase)
}

pub fn report_trial_boot(
    journal: &mut Journal,
    store: &mut impl JournalStore,
    healthy: bool,
) -> Result<UpdatePhase, Error> {
    if journal.update_phase != UpdatePhase::Trial {
        return Err(Error::InvalidState);
    }
    if healthy {
        journal.active_slot = journal.target_slot;
        journal.update_phase = UpdatePhase::Committed;
    } else {
        journal.trial_boots_left = journal.trial_boots_left.saturating_sub(1);
        if journal.trial_boots_left == 0 {
            journal.update_phase = UpdatePhase::RollingBack;
        }
    }
    checkpoint(store, journal)?;
    Ok(journal.update_phase)
}

pub fn rollback(
    journal: &mut Journal,
    store: &mut impl JournalStore,
    firmware: &mut impl Firmware,
) -> Result<(), Error> {
    if journal.update_phase != UpdatePhase::RollingBack
        && journal.update_phase != UpdatePhase::Trial
    {
        return Err(Error::RollbackUnavailable);
    }
    firmware.select_boot_slot(journal.active_slot)?;
    journal.update_phase = UpdatePhase::RolledBack;
    checkpoint(store, journal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    Resume,
    RollbackUpdate,
    BootA,
    BootB,
    RepairBootEntry,
    FactoryReset,
}

pub fn available_recovery_actions(journal: Journal) -> u16 {
    let mut flags = (1 << RecoveryAction::BootA as u8)
        | (1 << RecoveryAction::BootB as u8)
        | (1 << RecoveryAction::RepairBootEntry as u8);
    if journal.operation != Operation::None {
        flags |= 1 << RecoveryAction::Resume as u8;
    }
    if journal.operation == Operation::Update && journal.update_phase != UpdatePhase::Committed {
        flags |= 1 << RecoveryAction::RollbackUpdate as u8;
    }
    flags | (1 << RecoveryAction::FactoryReset as u8)
}

pub fn recover(store: &mut impl JournalStore) -> Result<Journal, Error> {
    store.load()
}

fn digest_eq(left: [u8; 32], right: [u8; 32]) -> bool {
    let mut difference = 0u8;
    for index in 0..32 {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

// Compact incremental SHA-256. State is Copy so it can live directly in the
// persistent journal without heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: u8,
    bytes: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            block_len: 0,
            bytes: 0,
        }
    }
    pub fn update(&mut self, mut data: &[u8]) {
        self.bytes = self.bytes.wrapping_add(data.len() as u64);
        if self.block_len != 0 {
            let take = core::cmp::min(64 - self.block_len as usize, data.len());
            self.block[self.block_len as usize..self.block_len as usize + take]
                .copy_from_slice(&data[..take]);
            self.block_len += take as u8;
            data = &data[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        self.block[..data.len()].copy_from_slice(data);
        self.block_len = data.len() as u8;
    }
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.bytes.wrapping_mul(8);
        let n = self.block_len as usize;
        self.block[n] = 0x80;
        if n >= 56 {
            for i in n + 1..64 {
                self.block[i] = 0;
            }
            let b = self.block;
            self.compress(&b);
            self.block = [0; 64];
        } else {
            for i in n + 1..56 {
                self.block[i] = 0;
            }
        }
        self.block[56..64].copy_from_slice(&bits.to_be_bytes());
        let b = self.block;
        self.compress(&b);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *x = x.wrapping_add(y);
        }
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    const ID: DiskId = DiskId(*b"NOVA-TEST-DISK01");
    fn info() -> DiskInfo {
        DiskInfo {
            id: ID,
            sectors: 50_000,
            logical_sector_size: 512,
            bus: Bus::Nvme,
            removable: false,
            read_only: false,
            system_disk: false,
        }
    }
    fn plan() -> InstallPlan {
        InstallPlan {
            disk: ID,
            minimum_sectors: 10_000,
            primary_gpt_lba: 1,
            backup_gpt_lba: 49_999,
            esp_start: 2048,
            slot_a_start: 4096,
            slot_a_sectors: 64,
            recovery_start: 40_000,
            locale: UKRAINIAN_LOCALE,
            allow_removable: false,
        }
    }

    struct Inv([DiskInfo; 1]);
    impl DiskInventorySource for Inv {
        fn count(&self) -> usize {
            1
        }
        fn disk(&self, i: usize) -> Result<DiskInfo, Error> {
            self.0.get(i).copied().ok_or(Error::DiskNotFound)
        }
    }
    #[test]
    fn inventory_and_eligibility() {
        let inv = Inventory::scan(&Inv([info()])).unwrap();
        assert_eq!(inv.by_id(ID), Some(info()));
        assert_eq!(info().eligibility(1), Eligibility::Eligible);
    }
    #[test]
    fn confirmation_is_bound_to_nonce() {
        let a = confirmation_token(info(), [1; 16]);
        let b = confirmation_token(info(), [2; 16]);
        assert!(verify_confirmation(a, a));
        assert!(!verify_confirmation(a, b));
    }
    #[test]
    fn sha256_vector() {
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            h.finish(),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }

    #[derive(Clone)]
    struct MemJournal {
        value: Journal,
        commits: u32,
        fail_at: Option<u32>,
    }
    impl MemJournal {
        fn new() -> Self {
            Self {
                value: Journal::empty(),
                commits: 0,
                fail_at: None,
            }
        }
    }
    impl JournalStore for MemJournal {
        fn load(&mut self) -> Result<Journal, Error> {
            Ok(self.value)
        }
        fn commit(&mut self, j: &Journal) -> Result<(), Error> {
            self.commits += 1;
            if self.fail_at == Some(self.commits) {
                return Err(Error::FaultInjected);
            }
            self.value = *j;
            Ok(())
        }
        fn clear(&mut self) -> Result<(), Error> {
            self.value = Journal::empty();
            Ok(())
        }
    }
    struct Dev {
        data: Vec<[u8; 512]>,
        flushes: u32,
        fail_write: Option<u32>,
        writes: u32,
    }
    impl Dev {
        fn new() -> Self {
            Self {
                data: std::vec![[0;512];50_000],
                flushes: 0,
                fail_write: None,
                writes: 0,
            }
        }
    }
    impl BlockDevice for Dev {
        fn info(&self) -> DiskInfo {
            info()
        }
        fn read_sector(&mut self, l: u64, s: &mut [u8; 512]) -> Result<(), Error> {
            *s = self.data[l as usize];
            Ok(())
        }
        fn write_sector(&mut self, l: u64, s: &[u8; 512]) -> Result<(), Error> {
            self.writes += 1;
            if self.fail_write == Some(self.writes) {
                return Err(Error::FaultInjected);
            }
            self.data[l as usize] = *s;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Error> {
            self.flushes += 1;
            Ok(())
        }
    }
    #[derive(Default)]
    struct Fw {
        selected: Option<Slot>,
        installed: bool,
    }
    impl Firmware for Fw {
        fn install_boot_entry(&mut self, _: DiskId, _: u64) -> Result<(), Error> {
            self.installed = true;
            Ok(())
        }
        fn select_boot_slot(&mut self, s: Slot) -> Result<(), Error> {
            self.selected = Some(s);
            Ok(())
        }
    }
    #[derive(Default)]
    struct Gpt(u8);
    impl GptWriter for Gpt {
        fn protective_mbr(
            &mut self,
            _: &mut impl BlockDevice,
            _: InstallPlan,
        ) -> Result<(), Error> {
            self.0 |= 1;
            Ok(())
        }
        fn primary(&mut self, _: &mut impl BlockDevice, _: InstallPlan) -> Result<(), Error> {
            self.0 |= 2;
            Ok(())
        }
        fn backup(&mut self, _: &mut impl BlockDevice, _: InstallPlan) -> Result<(), Error> {
            self.0 |= 4;
            Ok(())
        }
    }
    #[derive(Default)]
    struct Setup(Option<Locale>);
    impl SetupWriter for Setup {
        fn write_locale(
            &mut self,
            _: &mut impl BlockDevice,
            _: InstallPlan,
            l: Locale,
        ) -> Result<(), Error> {
            self.0 = Some(l);
            Ok(())
        }
    }
    struct Img {
        bytes: Vec<u8>,
        hash: [u8; 32],
    }
    impl Img {
        fn new(bytes: Vec<u8>) -> Self {
            let mut h = Sha256::new();
            h.update(&bytes);
            Self {
                bytes,
                hash: h.finish(),
            }
        }
    }
    impl ImageSource for Img {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn expected_sha256(&self) -> [u8; 32] {
            self.hash
        }
        fn read_at(&mut self, o: u64, out: &mut [u8]) -> Result<usize, Error> {
            let o = o as usize;
            if o >= self.bytes.len() {
                return Ok(0);
            }
            let n = core::cmp::min(out.len(), self.bytes.len() - o);
            out[..n].copy_from_slice(&self.bytes[o..o + n]);
            Ok(n)
        }
    }

    fn drive_install(
        j: &mut Journal,
        store: &mut MemJournal,
        d: &mut Dev,
        fw: &mut Fw,
        g: &mut Gpt,
        s: &mut Setup,
        img: &mut Img,
    ) {
        for _ in 0..20 {
            if advance_install(j, store, d, fw, g, s, img, plan()).unwrap()
                == InstallPhase::Complete
            {
                return;
            }
        }
        panic!("not complete")
    }
    #[test]
    fn complete_install_has_barriers_and_ukrainian_setup() {
        let mut st = MemJournal::new();
        let nonce = [7; 16];
        let mut j = begin_install(
            &mut st,
            info(),
            plan(),
            700,
            nonce,
            confirmation_token(info(), nonce),
        )
        .unwrap();
        let (mut d, mut fw, mut g, mut s, mut img) = (
            Dev::new(),
            Fw::default(),
            Gpt::default(),
            Setup::default(),
            Img::new(std::vec![0x5a;700]),
        );
        drive_install(&mut j, &mut st, &mut d, &mut fw, &mut g, &mut s, &mut img);
        assert_eq!(g.0, 7);
        assert!(fw.installed);
        assert_eq!(fw.selected, Some(Slot::A));
        assert_eq!(s.0, Some(UKRAINIAN_LOCALE));
        assert!(d.flushes >= 4);
        assert_eq!(&d.data[4096][..512], &img.bytes[..512]);
    }
    #[test]
    fn bad_hash_never_installs_boot_entry() {
        let mut st = MemJournal::new();
        let n = [1; 16];
        let mut j = begin_install(
            &mut st,
            info(),
            plan(),
            10,
            n,
            confirmation_token(info(), n),
        )
        .unwrap();
        let (mut d, mut fw, mut g, mut s, mut img) = (
            Dev::new(),
            Fw::default(),
            Gpt::default(),
            Setup::default(),
            Img::new(std::vec![1;10]),
        );
        img.hash = [9; 32];
        let mut got = false;
        for _ in 0..10 {
            if advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan(),
            ) == Err(Error::HashMismatch)
            {
                got = true;
                break;
            }
        }
        assert!(got);
        assert!(!fw.installed);
    }
    #[test]
    fn journal_power_loss_resumes_exact_offset() {
        let mut st = MemJournal::new();
        let n = [3; 16];
        let mut j = begin_install(
            &mut st,
            info(),
            plan(),
            700,
            n,
            confirmation_token(info(), n),
        )
        .unwrap();
        let (mut d, mut fw, mut g, mut s, mut img) = (
            Dev::new(),
            Fw::default(),
            Gpt::default(),
            Setup::default(),
            Img::new(std::vec![8;700]),
        );
        for _ in 0..5 {
            advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan(),
            )
            .unwrap();
        }
        assert_eq!(st.value.byte_offset, 512);
        let mut recovered = recover(&mut st).unwrap();
        drive_install(
            &mut recovered,
            &mut st,
            &mut d,
            &mut fw,
            &mut g,
            &mut s,
            &mut img,
        );
        assert_eq!(recovered.install_phase, InstallPhase::Complete);
    }
    #[test]
    fn injected_write_fault_does_not_advance_journal() {
        let mut st = MemJournal::new();
        let n = [4; 16];
        let mut j = begin_install(
            &mut st,
            info(),
            plan(),
            600,
            n,
            confirmation_token(info(), n),
        )
        .unwrap();
        let (mut d, mut fw, mut g, mut s, mut img) = (
            Dev::new(),
            Fw::default(),
            Gpt::default(),
            Setup::default(),
            Img::new(std::vec![2;600]),
        );
        for _ in 0..4 {
            advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan(),
            )
            .unwrap();
        }
        d.fail_write = Some(1);
        assert_eq!(
            advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan()
            ),
            Err(Error::FaultInjected)
        );
        assert_eq!(st.value.byte_offset, 0);
    }
    #[test]
    fn injected_journal_fault_recovers_previous_checkpoint() {
        let mut st = MemJournal::new();
        let n = [5; 16];
        let mut j = begin_install(
            &mut st,
            info(),
            plan(),
            600,
            n,
            confirmation_token(info(), n),
        )
        .unwrap();
        let (mut d, mut fw, mut g, mut s, mut img) = (
            Dev::new(),
            Fw::default(),
            Gpt::default(),
            Setup::default(),
            Img::new(std::vec![4; 600]),
        );
        for _ in 0..4 {
            advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan(),
            )
            .unwrap();
        }
        st.fail_at = Some(st.commits + 1);
        assert_eq!(
            advance_install(
                &mut j,
                &mut st,
                &mut d,
                &mut fw,
                &mut g,
                &mut s,
                &mut img,
                plan()
            ),
            Err(Error::FaultInjected)
        );
        assert_eq!(st.value.byte_offset, 0);
        st.fail_at = None;
        let mut recovered = recover(&mut st).unwrap();
        drive_install(
            &mut recovered,
            &mut st,
            &mut d,
            &mut fw,
            &mut g,
            &mut s,
            &mut img,
        );
        assert_eq!(recovered.install_phase, InstallPhase::Complete);
    }
    #[test]
    fn ab_update_switches_then_rolls_back_after_failed_trials() {
        let p = UpdatePlan {
            disk: ID,
            active: Slot::A,
            inactive_start: 8192,
            inactive_sectors: 8,
            trial_boots: 2,
        };
        let mut st = MemJournal::new();
        let mut j = begin_update(&mut st, p).unwrap();
        mark_downloaded(&mut j, &mut st).unwrap();
        let (mut d, mut fw, mut img) = (Dev::new(), Fw::default(), Img::new(std::vec![3;513]));
        for _ in 0..8 {
            if advance_update(&mut j, &mut st, &mut d, &mut fw, &mut img, p).unwrap()
                == UpdatePhase::Trial
            {
                break;
            }
        }
        assert_eq!(fw.selected, Some(Slot::B));
        assert_eq!(
            report_trial_boot(&mut j, &mut st, false).unwrap(),
            UpdatePhase::Trial
        );
        assert_eq!(
            report_trial_boot(&mut j, &mut st, false).unwrap(),
            UpdatePhase::RollingBack
        );
        rollback(&mut j, &mut st, &mut fw).unwrap();
        assert_eq!(fw.selected, Some(Slot::A));
        assert_eq!(j.update_phase, UpdatePhase::RolledBack);
    }
    #[test]
    fn healthy_update_commits_new_active_slot() {
        let p = UpdatePlan {
            disk: ID,
            active: Slot::A,
            inactive_start: 8192,
            inactive_sectors: 8,
            trial_boots: 2,
        };
        let mut st = MemJournal::new();
        let mut j = begin_update(&mut st, p).unwrap();
        j.update_phase = UpdatePhase::Trial;
        j.target_slot = Slot::B;
        assert_eq!(
            report_trial_boot(&mut j, &mut st, true).unwrap(),
            UpdatePhase::Committed
        );
        assert_eq!(j.active_slot, Slot::B);
    }
    #[test]
    fn recovery_flags_follow_journal() {
        let mut j = Journal::empty();
        j.operation = Operation::Update;
        j.update_phase = UpdatePhase::Trial;
        let f = available_recovery_actions(j);
        assert_ne!(f & (1 << RecoveryAction::Resume as u8), 0);
        assert_ne!(f & (1 << RecoveryAction::RollbackUpdate as u8), 0);
    }
}
