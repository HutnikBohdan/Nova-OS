use crate::fs::RamFs;
use core::arch::asm;
use core::ptr;
use storage_core::{
    BLOCK_SIZE, BlockDevice, Journal, StorageError, Superblock, Transaction, crc32, format,
};

const DATA_SECTORS_FALLBACK: u64 = 8192;
const SNAPSHOT_BLOCKS: usize = 36;
const SNAPSHOT_BYTES: usize = SNAPSHOT_BLOCKS * BLOCK_SIZE;
const SNAPSHOT_MAGIC: &[u8; 8] = b"NVFSAB01";

static mut STORAGE: Option<PersistentDisk> = None;

struct PersistentDisk {
    disk: AtaPio,
    superblock: Superblock,
    generation: u64,
    active_slot: u8,
}

pub fn initialize_storage(fs: &mut RamFs) {
    let Some(mut disk) = AtaPio::identify_primary_slave() else {
        return;
    };
    let mut block = [0u8; BLOCK_SIZE];
    if disk.read_block(0, &mut block).is_err() {
        return;
    }
    let superblock = match Superblock::decode(&block) {
        Ok(value) => value,
        Err(_) => match format(&mut disk, 4, 8) {
            Ok(value) => value,
            Err(_) => return,
        },
    };
    let _ = Journal::recover(&mut disk, &superblock);
    if disk.read_block(superblock.data_start, &mut block).is_err() {
        return;
    }
    let previous = u64::from_le_bytes(block[..8].try_into().unwrap());
    let next = previous.wrapping_add(1).max(1);
    block[..8].copy_from_slice(&next.to_le_bytes());
    block[8..32].copy_from_slice(b"NOVA_PERSISTENCE_OK_____");
    let mut transaction = Transaction::<1>::new();
    if transaction.stage(superblock.data_start, &block).is_err() {
        return;
    }
    if Journal::commit(&mut disk, &superblock, next, &transaction).is_ok() {
        crate::serial::write_str("NOVA_STORAGE_READY\n");
        crate::serial::write_fmt(format_args!("NOVA_PERSISTENT_BOOT_COUNT={next}\n"));
    }
    let (generation, active_slot) =
        load_latest_snapshot(&mut disk, &superblock, fs).unwrap_or((0, 1));
    unsafe {
        *ptr::addr_of_mut!(STORAGE) = Some(PersistentDisk {
            disk,
            superblock,
            generation,
            active_slot,
        });
    }
    if generation == 0 {
        let _ = persist(fs);
    } else {
        crate::serial::write_str("NOVA_FILES_RESTORED\n");
    }
}

pub fn persist(fs: &RamFs) -> Result<(), StorageError> {
    let storage = unsafe { (&mut *ptr::addr_of_mut!(STORAGE)).as_mut() }.ok_or(StorageError::Io)?;
    let mut payload = [0u8; SNAPSHOT_BYTES];
    let len = fs.export(&mut payload).map_err(|_| StorageError::Io)?;
    let next_slot = 1 - storage.active_slot;
    let header_block = snapshot_header_block(&storage.superblock, next_slot);
    for index in 0..SNAPSHOT_BLOCKS {
        let mut block = [0u8; BLOCK_SIZE];
        block.copy_from_slice(&payload[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE]);
        storage
            .disk
            .write_block(header_block + 1 + index as u64, &block)?;
    }
    storage.disk.flush()?;
    let generation = storage.generation.wrapping_add(1).max(1);
    let mut header = [0u8; BLOCK_SIZE];
    header[..8].copy_from_slice(SNAPSHOT_MAGIC);
    header[8..16].copy_from_slice(&generation.to_le_bytes());
    header[16..20].copy_from_slice(&(len as u32).to_le_bytes());
    header[20..24].copy_from_slice(&crc32(&payload[..len]).to_le_bytes());
    storage.disk.write_block(header_block, &header)?;
    storage.disk.flush()?;
    storage.generation = generation;
    storage.active_slot = next_slot;
    crate::serial::write_fmt(format_args!("NOVA_FILES_SAVED_GENERATION={generation}\n"));
    Ok(())
}

fn load_latest_snapshot(
    disk: &mut AtaPio,
    superblock: &Superblock,
    fs: &mut RamFs,
) -> Option<(u64, u8)> {
    let mut best_generation = 0u64;
    let mut best_slot = 0u8;
    let mut best_payload = [0u8; SNAPSHOT_BYTES];
    let mut candidate = [0u8; SNAPSHOT_BYTES];
    for slot in 0..=1u8 {
        let header_block = snapshot_header_block(superblock, slot);
        let mut header = [0u8; BLOCK_SIZE];
        disk.read_block(header_block, &mut header).ok()?;
        if &header[..8] != SNAPSHOT_MAGIC {
            continue;
        }
        let generation = u64::from_le_bytes(header[8..16].try_into().ok()?);
        let len = u32::from_le_bytes(header[16..20].try_into().ok()?) as usize;
        let expected = u32::from_le_bytes(header[20..24].try_into().ok()?);
        if len > SNAPSHOT_BYTES {
            continue;
        }
        for index in 0..SNAPSHOT_BLOCKS {
            let mut block = [0u8; BLOCK_SIZE];
            disk.read_block(header_block + 1 + index as u64, &mut block)
                .ok()?;
            candidate[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE].copy_from_slice(&block);
        }
        if crc32(&candidate[..len]) == expected && generation > best_generation {
            best_generation = generation;
            best_slot = slot;
            best_payload.copy_from_slice(&candidate);
        }
    }
    if best_generation == 0 {
        return None;
    }
    let header_block = snapshot_header_block(superblock, best_slot);
    let mut header = [0u8; BLOCK_SIZE];
    disk.read_block(header_block, &mut header).ok()?;
    let len = u32::from_le_bytes(header[16..20].try_into().ok()?) as usize;
    fs.import(&best_payload[..len]).ok()?;
    Some((best_generation, best_slot))
}

const fn snapshot_header_block(superblock: &Superblock, slot: u8) -> u64 {
    superblock.data_start + 1 + slot as u64 * (SNAPSHOT_BLOCKS as u64 + 1)
}

struct AtaPio {
    io: u16,
    control: u16,
    drive: u8,
    sectors: u64,
}

impl AtaPio {
    fn identify_primary_slave() -> Option<Self> {
        let mut disk = Self {
            io: 0x1f0,
            control: 0x3f6,
            drive: 1,
            sectors: DATA_SECTORS_FALLBACK,
        };
        unsafe {
            out8(disk.io + 6, 0xb0);
            delay400ns(disk.control);
            out8(disk.io + 2, 0);
            out8(disk.io + 3, 0);
            out8(disk.io + 4, 0);
            out8(disk.io + 5, 0);
            out8(disk.io + 7, 0xec);
        }
        let status = unsafe { in8(disk.io + 7) };
        if status == 0 || status == 0xff || !disk.wait_drq() {
            return None;
        }
        let mut words = [0u16; 256];
        for word in &mut words {
            *word = unsafe { in16(disk.io) };
        }
        let sectors = words[60] as u64 | ((words[61] as u64) << 16);
        if sectors > 0 {
            disk.sectors = sectors;
        }
        Some(disk)
    }

    fn wait_not_busy(&self) -> bool {
        for _ in 0..1_000_000 {
            let status = unsafe { in8(self.io + 7) };
            if status & 0x80 == 0 {
                return status & 0x01 == 0;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn wait_drq(&self) -> bool {
        for _ in 0..1_000_000 {
            let status = unsafe { in8(self.io + 7) };
            if status & 0x01 != 0 {
                return false;
            }
            if status & 0x80 == 0 && status & 0x08 != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn select_lba(&self, lba: u64, command: u8) -> Result<(), StorageError> {
        if lba >= self.sectors || lba > 0x0fff_ffff {
            return Err(StorageError::OutOfBounds);
        }
        if !self.wait_not_busy() {
            return Err(StorageError::Io);
        }
        unsafe {
            out8(
                self.io + 6,
                0xe0 | (self.drive << 4) | ((lba >> 24) as u8 & 0x0f),
            );
            delay400ns(self.control);
            out8(self.io + 1, 0);
            out8(self.io + 2, 1);
            out8(self.io + 3, lba as u8);
            out8(self.io + 4, (lba >> 8) as u8);
            out8(self.io + 5, (lba >> 16) as u8);
            out8(self.io + 7, command);
        }
        if self.wait_drq() {
            Ok(())
        } else {
            Err(StorageError::Io)
        }
    }
}

impl BlockDevice for AtaPio {
    fn block_count(&self) -> u64 {
        self.sectors
    }

    fn read_block(&mut self, block: u64, out: &mut [u8; BLOCK_SIZE]) -> Result<(), StorageError> {
        self.select_lba(block, 0x20)?;
        for index in 0..256 {
            let word = unsafe { in16(self.io) }.to_le_bytes();
            out[index * 2] = word[0];
            out[index * 2 + 1] = word[1];
        }
        Ok(())
    }

    fn write_block(&mut self, block: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), StorageError> {
        self.select_lba(block, 0x30)?;
        for index in 0..256 {
            unsafe {
                out16(
                    self.io,
                    u16::from_le_bytes([data[index * 2], data[index * 2 + 1]]),
                );
            }
        }
        if self.wait_not_busy() {
            Ok(())
        } else {
            Err(StorageError::Io)
        }
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        if !self.wait_not_busy() {
            return Err(StorageError::Io);
        }
        unsafe {
            out8(self.io + 6, 0xe0 | (self.drive << 4));
            out8(self.io + 7, 0xe7);
        }
        if self.wait_not_busy() {
            Ok(())
        } else {
            Err(StorageError::Io)
        }
    }
}

unsafe fn delay400ns(control: u16) {
    for _ in 0..4 {
        let _ = unsafe { in8(control) };
    }
}
unsafe fn out8(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
    }
}
unsafe fn in8(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack));
    }
    value
}
unsafe fn out16(port: u16, value: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack));
    }
}
unsafe fn in16(port: u16) -> u16 {
    let value: u16;
    unsafe {
        asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack));
    }
    value
}
