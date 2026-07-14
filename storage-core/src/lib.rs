#![no_std]

pub const BLOCK_SIZE: usize = 512;
pub const NOVA_FS_MAGIC: u64 = 0x4e4f_5641_4653_3031;
const JOURNAL_MAGIC: u32 = 0x4e4a_4e4c;
const PREPARED: u8 = 1;
const COMMITTED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    OutOfBounds,
    Io,
    InvalidSuperblock,
    InvalidJournal,
    JournalFull,
    InvalidLayout,
}

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, out: &mut [u8; BLOCK_SIZE]) -> Result<(), StorageError>;
    fn write_block(&mut self, block: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), StorageError>;
    fn flush(&mut self) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superblock {
    pub total_blocks: u64,
    pub journal_start: u64,
    pub journal_entries: u32,
    pub inode_start: u64,
    pub data_start: u64,
    pub generation: u64,
}

impl Superblock {
    pub fn new(
        total_blocks: u64,
        journal_entries: u32,
        inode_blocks: u32,
    ) -> Result<Self, StorageError> {
        let journal_start = 1;
        let inode_start = journal_start + journal_entries as u64 * 2;
        let data_start = inode_start + inode_blocks as u64;
        if journal_entries == 0 || inode_blocks == 0 || data_start >= total_blocks {
            return Err(StorageError::InvalidLayout);
        }
        Ok(Self {
            total_blocks,
            journal_start,
            journal_entries,
            inode_start,
            data_start,
            generation: 1,
        })
    }

    pub fn encode(&self) -> [u8; BLOCK_SIZE] {
        let mut out = [0u8; BLOCK_SIZE];
        put_u64(&mut out, 0, NOVA_FS_MAGIC);
        put_u32(&mut out, 8, 1);
        put_u64(&mut out, 16, self.total_blocks);
        put_u64(&mut out, 24, self.journal_start);
        put_u32(&mut out, 32, self.journal_entries);
        put_u64(&mut out, 40, self.inode_start);
        put_u64(&mut out, 48, self.data_start);
        put_u64(&mut out, 56, self.generation);
        let sum = crc32(&out[..BLOCK_SIZE - 4]);
        put_u32(&mut out, BLOCK_SIZE - 4, sum);
        out
    }

    pub fn decode(block: &[u8; BLOCK_SIZE]) -> Result<Self, StorageError> {
        if get_u64(block, 0) != NOVA_FS_MAGIC || get_u32(block, 8) != 1 {
            return Err(StorageError::InvalidSuperblock);
        }
        if get_u32(block, BLOCK_SIZE - 4) != crc32(&block[..BLOCK_SIZE - 4]) {
            return Err(StorageError::InvalidSuperblock);
        }
        let value = Self {
            total_blocks: get_u64(block, 16),
            journal_start: get_u64(block, 24),
            journal_entries: get_u32(block, 32),
            inode_start: get_u64(block, 40),
            data_start: get_u64(block, 48),
            generation: get_u64(block, 56),
        };
        if value.journal_entries == 0
            || value.inode_start <= value.journal_start
            || value.data_start <= value.inode_start
            || value.data_start >= value.total_blocks
        {
            return Err(StorageError::InvalidLayout);
        }
        Ok(value)
    }
}

pub fn format(
    device: &mut impl BlockDevice,
    journal_entries: u32,
    inode_blocks: u32,
) -> Result<Superblock, StorageError> {
    let superblock = Superblock::new(device.block_count(), journal_entries, inode_blocks)?;
    device.write_block(0, &superblock.encode())?;
    let zero = [0u8; BLOCK_SIZE];
    for index in 0..journal_entries as u64 * 2 + inode_blocks as u64 {
        device.write_block(1 + index, &zero)?;
    }
    device.flush()?;
    Ok(superblock)
}

#[derive(Clone, Copy)]
pub struct PendingWrite {
    pub target: u64,
    pub data: [u8; BLOCK_SIZE],
}

pub struct Transaction<const N: usize> {
    writes: [Option<PendingWrite>; N],
    len: usize,
}

impl<const N: usize> Transaction<N> {
    pub const fn new() -> Self {
        Self {
            writes: [None; N],
            len: 0,
        }
    }
    pub fn stage(&mut self, target: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), StorageError> {
        if self.len == N {
            return Err(StorageError::JournalFull);
        }
        self.writes[self.len] = Some(PendingWrite {
            target,
            data: *data,
        });
        self.len += 1;
        Ok(())
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for Transaction<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Journal;

impl Journal {
    pub fn commit<const N: usize>(
        device: &mut impl BlockDevice,
        superblock: &Superblock,
        sequence: u64,
        transaction: &Transaction<N>,
    ) -> Result<(), StorageError> {
        if transaction.len() > superblock.journal_entries as usize {
            return Err(StorageError::JournalFull);
        }
        for (slot, write) in transaction.writes[..transaction.len].iter().enumerate() {
            let write = write.as_ref().ok_or(StorageError::InvalidJournal)?;
            if write.target >= superblock.total_blocks || write.target < superblock.data_start {
                return Err(StorageError::OutOfBounds);
            }
            let meta_block = superblock.journal_start + slot as u64 * 2;
            device.write_block(meta_block + 1, &write.data)?;
            device.flush()?;
            device.write_block(
                meta_block,
                &journal_meta(sequence, write.target, crc32(&write.data), PREPARED),
            )?;
            device.flush()?;
            device.write_block(
                meta_block,
                &journal_meta(sequence, write.target, crc32(&write.data), COMMITTED),
            )?;
            device.flush()?;
        }
        for write in transaction.writes[..transaction.len].iter().flatten() {
            device.write_block(write.target, &write.data)?;
        }
        device.flush()?;
        let zero = [0u8; BLOCK_SIZE];
        for slot in 0..transaction.len {
            device.write_block(superblock.journal_start + slot as u64 * 2, &zero)?;
        }
        device.flush()
    }

    /// Replays only records carrying a durable COMMITTED marker and valid payload checksum.
    pub fn recover(
        device: &mut impl BlockDevice,
        superblock: &Superblock,
    ) -> Result<usize, StorageError> {
        let mut replayed = 0;
        let mut meta = [0u8; BLOCK_SIZE];
        let mut payload = [0u8; BLOCK_SIZE];
        let zero = [0u8; BLOCK_SIZE];
        for slot in 0..superblock.journal_entries as u64 {
            let meta_block = superblock.journal_start + slot * 2;
            device.read_block(meta_block, &mut meta)?;
            if meta.iter().all(|byte| *byte == 0) {
                continue;
            }
            let Some((state, target, payload_sum)) = parse_journal_meta(&meta) else {
                device.write_block(meta_block, &zero)?;
                continue;
            };
            if state == COMMITTED
                && target >= superblock.data_start
                && target < superblock.total_blocks
            {
                device.read_block(meta_block + 1, &mut payload)?;
                if crc32(&payload) == payload_sum {
                    device.write_block(target, &payload)?;
                    replayed += 1;
                }
            }
            device.write_block(meta_block, &zero)?;
        }
        device.flush()?;
        Ok(replayed)
    }
}

fn journal_meta(sequence: u64, target: u64, payload_sum: u32, state: u8) -> [u8; BLOCK_SIZE] {
    let mut out = [0u8; BLOCK_SIZE];
    put_u32(&mut out, 0, JOURNAL_MAGIC);
    out[4] = state;
    put_u64(&mut out, 8, sequence);
    put_u64(&mut out, 16, target);
    put_u32(&mut out, 24, payload_sum);
    let header_sum = crc32(&out[..28]);
    put_u32(&mut out, 28, header_sum);
    out
}

fn parse_journal_meta(block: &[u8; BLOCK_SIZE]) -> Option<(u8, u64, u32)> {
    if get_u32(block, 0) != JOURNAL_MAGIC || get_u32(block, 28) != crc32(&block[..28]) {
        return None;
    }
    let state = block[4];
    if state != PREPARED && state != COMMITTED {
        return None;
    }
    Some((state, get_u64(block, 16), get_u32(block, 24)))
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn put_u32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut [u8], at: usize, value: u64) {
    out[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn get_u32(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(input[at..at + 4].try_into().unwrap())
}
fn get_u64(input: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(input[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    struct MemoryDevice {
        blocks: Vec<[u8; BLOCK_SIZE]>,
        writes_before_failure: Option<usize>,
    }
    impl MemoryDevice {
        fn new(count: usize) -> Self {
            Self {
                blocks: vec![[0; BLOCK_SIZE]; count],
                writes_before_failure: None,
            }
        }
    }
    impl BlockDevice for MemoryDevice {
        fn block_count(&self) -> u64 {
            self.blocks.len() as u64
        }
        fn read_block(
            &mut self,
            block: u64,
            out: &mut [u8; BLOCK_SIZE],
        ) -> Result<(), StorageError> {
            *out = *self
                .blocks
                .get(block as usize)
                .ok_or(StorageError::OutOfBounds)?;
            Ok(())
        }
        fn write_block(&mut self, block: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), StorageError> {
            if let Some(remaining) = &mut self.writes_before_failure {
                if *remaining == 0 {
                    return Err(StorageError::Io);
                }
                *remaining -= 1;
            }
            *self
                .blocks
                .get_mut(block as usize)
                .ok_or(StorageError::OutOfBounds)? = *data;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), StorageError> {
            Ok(())
        }
    }

    #[test]
    fn formats_and_validates_superblock() {
        let mut disk = MemoryDevice::new(128);
        let expected = format(&mut disk, 4, 8).unwrap();
        let decoded = Superblock::decode(&disk.blocks[0]).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn journal_commit_persists_full_block() {
        let mut disk = MemoryDevice::new(128);
        let superblock = format(&mut disk, 4, 8).unwrap();
        let mut transaction = Transaction::<2>::new();
        let data = [0x5a; BLOCK_SIZE];
        transaction.stage(superblock.data_start + 3, &data).unwrap();
        Journal::commit(&mut disk, &superblock, 7, &transaction).unwrap();
        assert_eq!(disk.blocks[(superblock.data_start + 3) as usize], data);
        assert!(
            disk.blocks[superblock.journal_start as usize]
                .iter()
                .all(|byte| *byte == 0)
        );
    }

    #[test]
    fn recovery_replays_committed_record_after_crash() {
        let mut disk = MemoryDevice::new(128);
        let superblock = format(&mut disk, 4, 8).unwrap();
        let target = superblock.data_start + 2;
        let payload = [0xa5; BLOCK_SIZE];
        disk.blocks[(superblock.journal_start + 1) as usize] = payload;
        disk.blocks[superblock.journal_start as usize] =
            journal_meta(9, target, crc32(&payload), COMMITTED);
        assert_eq!(Journal::recover(&mut disk, &superblock).unwrap(), 1);
        assert_eq!(disk.blocks[target as usize], payload);
    }

    #[test]
    fn recovery_drops_prepared_but_uncommitted_record() {
        let mut disk = MemoryDevice::new(128);
        let superblock = format(&mut disk, 4, 8).unwrap();
        let target = superblock.data_start + 1;
        let payload = [7; BLOCK_SIZE];
        disk.blocks[(superblock.journal_start + 1) as usize] = payload;
        disk.blocks[superblock.journal_start as usize] =
            journal_meta(1, target, crc32(&payload), PREPARED);
        assert_eq!(Journal::recover(&mut disk, &superblock).unwrap(), 0);
        assert_eq!(disk.blocks[target as usize], [0; BLOCK_SIZE]);
    }
}
