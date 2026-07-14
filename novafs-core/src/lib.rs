#![no_std]
#![forbid(unsafe_code)]

pub const BLOCK_SIZE: usize = 4096;
pub const MAGIC: u64 = 0x3246_5341_564f_4e;
pub const VERSION: u16 = 2;
pub const MAX_EXTENTS: usize = 6;
pub const MAX_REPAIRS: usize = 32;
const JOURNAL_MAGIC: u32 = 0x324a_564e;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    OutOfBounds,
    Corrupt,
    InvalidLayout,
    NoSpace,
    Full,
    Conflict,
    Unsupported,
}

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK_SIZE]) -> Result<(), Error>;
    fn write(&mut self, block: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
}

pub fn checksum(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb8_8320 & (c & 1).wrapping_neg());
        }
    }
    !c
}
fn p16(o: &mut [u8], n: usize, v: u16) {
    o[n..n + 2].copy_from_slice(&v.to_le_bytes());
}
fn p32(o: &mut [u8], n: usize, v: u32) {
    o[n..n + 4].copy_from_slice(&v.to_le_bytes());
}
fn p64(o: &mut [u8], n: usize, v: u64) {
    o[n..n + 8].copy_from_slice(&v.to_le_bytes());
}
fn g16(i: &[u8], n: usize) -> u16 {
    u16::from_le_bytes(i[n..n + 2].try_into().unwrap())
}
fn g32(i: &[u8], n: usize) -> u32 {
    u32::from_le_bytes(i[n..n + 4].try_into().unwrap())
}
fn g64(i: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(i[n..n + 8].try_into().unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub total_blocks: u64,
    pub bitmap_start: u64,
    pub bitmap_blocks: u32,
    pub inode_start: u64,
    pub inode_blocks: u32,
    pub journal_start: u64,
    pub journal_blocks: u32,
    pub refcount_start: u64,
    pub refcount_blocks: u32,
    pub data_start: u64,
}
impl Layout {
    pub fn validate(&self) -> Result<(), Error> {
        let ranges = [
            (self.bitmap_start, self.bitmap_blocks as u64),
            (self.inode_start, self.inode_blocks as u64),
            (self.journal_start, self.journal_blocks as u64),
            (self.refcount_start, self.refcount_blocks as u64),
        ];
        if self.total_blocks < 32 || self.data_start >= self.total_blocks || self.data_start < 3 {
            return Err(Error::InvalidLayout);
        }
        for (s, n) in ranges {
            if n == 0 || s < 2 || s.checked_add(n).ok_or(Error::InvalidLayout)? > self.total_blocks
            {
                return Err(Error::InvalidLayout);
            }
        }
        for a in 0..ranges.len() {
            for b in a + 1..ranges.len() {
                let (as_, an) = ranges[a];
                let (bs, bn) = ranges[b];
                if as_ < bs + bn && bs < as_ + an {
                    return Err(Error::InvalidLayout);
                }
            }
        }
        if ranges.iter().any(|(s, n)| self.data_start < s + n) {
            return Err(Error::InvalidLayout);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanState {
    Clean = 1,
    Mounted = 2,
    Recovering = 3,
    NeedsFsck = 4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Superblock {
    pub generation: u64,
    pub uuid: [u8; 16],
    pub layout: Layout,
    pub root_inode: u64,
    pub journal_sequence: u64,
    pub state: CleanState,
}
impl Superblock {
    pub fn transition(&mut self, event: MountEvent) -> Result<RecoveryAction, Error> {
        let action = match (self.state, event) {
            (CleanState::Clean, MountEvent::Mount) => {
                self.state = CleanState::Mounted;
                RecoveryAction::Continue
            }
            (CleanState::Mounted, MountEvent::Mount) => {
                self.state = CleanState::Recovering;
                RecoveryAction::ReplayJournal
            }
            (CleanState::Recovering, MountEvent::Mount) => RecoveryAction::ReplayJournal,
            (CleanState::Recovering, MountEvent::ReplayOk) => {
                self.state = CleanState::Mounted;
                RecoveryAction::Continue
            }
            (_, MountEvent::ReplayFailed) => {
                self.state = CleanState::NeedsFsck;
                RecoveryAction::RunFsck
            }
            (CleanState::Mounted, MountEvent::Unmount) => {
                self.state = CleanState::Clean;
                RecoveryAction::Continue
            }
            (CleanState::NeedsFsck, MountEvent::FsckRepaired) => {
                self.state = CleanState::Clean;
                RecoveryAction::Continue
            }
            _ => return Err(Error::Conflict),
        };
        Ok(action)
    }

    pub fn encode(&self) -> [u8; BLOCK_SIZE] {
        let mut o = [0; BLOCK_SIZE];
        p64(&mut o, 0, MAGIC);
        p16(&mut o, 8, VERSION);
        p16(&mut o, 10, BLOCK_SIZE as u16);
        p64(&mut o, 16, self.generation);
        o[24..40].copy_from_slice(&self.uuid);
        p64(&mut o, 40, self.layout.total_blocks);
        p64(&mut o, 48, self.layout.bitmap_start);
        p32(&mut o, 56, self.layout.bitmap_blocks);
        p64(&mut o, 64, self.layout.inode_start);
        p32(&mut o, 72, self.layout.inode_blocks);
        p64(&mut o, 80, self.layout.journal_start);
        p32(&mut o, 88, self.layout.journal_blocks);
        p64(&mut o, 96, self.layout.refcount_start);
        p32(&mut o, 104, self.layout.refcount_blocks);
        p64(&mut o, 112, self.layout.data_start);
        p64(&mut o, 120, self.root_inode);
        p64(&mut o, 128, self.journal_sequence);
        o[136] = self.state as u8;
        let c = checksum(&o[..BLOCK_SIZE - 4]);
        p32(&mut o, BLOCK_SIZE - 4, c);
        o
    }
    pub fn decode(i: &[u8; BLOCK_SIZE]) -> Result<Self, Error> {
        if g64(i, 0) != MAGIC
            || g16(i, 8) != VERSION
            || g16(i, 10) as usize != BLOCK_SIZE
            || g32(i, BLOCK_SIZE - 4) != checksum(&i[..BLOCK_SIZE - 4])
        {
            return Err(Error::Corrupt);
        }
        let state = match i[136] {
            1 => CleanState::Clean,
            2 => CleanState::Mounted,
            3 => CleanState::Recovering,
            4 => CleanState::NeedsFsck,
            _ => return Err(Error::Corrupt),
        };
        let layout = Layout {
            total_blocks: g64(i, 40),
            bitmap_start: g64(i, 48),
            bitmap_blocks: g32(i, 56),
            inode_start: g64(i, 64),
            inode_blocks: g32(i, 72),
            journal_start: g64(i, 80),
            journal_blocks: g32(i, 88),
            refcount_start: g64(i, 96),
            refcount_blocks: g32(i, 104),
            data_start: g64(i, 112),
        };
        layout.validate()?;
        let mut uuid = [0; 16];
        uuid.copy_from_slice(&i[24..40]);
        let root_inode = g64(i, 120);
        if root_inode == 0 {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            generation: g64(i, 16),
            uuid,
            layout,
            root_inode,
            journal_sequence: g64(i, 128),
            state,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountEvent {
    Mount,
    Unmount,
    ReplayOk,
    ReplayFailed,
    FsckRepaired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Continue,
    ReplayJournal,
    RunFsck,
}

fn select_superblock(device: &mut impl BlockDevice) -> Result<(Superblock, u64, bool), Error> {
    let mut b = [0; BLOCK_SIZE];
    let mut valid = [None, None];
    let mut saw_io_error = false;
    for slot in [0, 1] {
        match device.read(slot as u64, &mut b) {
            Ok(()) => {
                valid[slot] = Superblock::decode(&b).ok();
            }
            Err(_) => saw_io_error = true,
        }
    }
    match (valid[0], valid[1]) {
        (Some(a), Some(b)) => {
            if a.uuid != b.uuid || a.layout != b.layout || a.root_inode != b.root_inode {
                return Err(Error::Conflict);
            }
            if a.generation == b.generation {
                if a != b {
                    return Err(Error::Conflict);
                }
                Ok((a, 0, false))
            } else if a.generation > b.generation {
                Ok((a, 0, false))
            } else {
                Ok((b, 1, false))
            }
        }
        (Some(a), None) => Ok((a, 0, true)),
        (None, Some(b)) => Ok((b, 1, true)),
        (None, None) if saw_io_error => Err(Error::Io),
        (None, None) => Err(Error::Corrupt),
    }
}

pub fn read_superblock(device: &mut impl BlockDevice) -> Result<(Superblock, u64), Error> {
    let (superblock, slot, _) = select_superblock(device)?;
    if superblock.layout.total_blocks > device.block_count() {
        return Err(Error::InvalidLayout);
    }
    Ok((superblock, slot))
}
pub fn write_superblock(
    device: &mut impl BlockDevice,
    current_slot: u64,
    mut sb: Superblock,
) -> Result<u64, Error> {
    sb.generation = sb.generation.checked_add(1).ok_or(Error::Conflict)?;
    let target = 1 - (current_slot & 1);
    device.write(target, &sb.encode())?;
    device.flush()?;
    Ok(target)
}

fn persist_superblock(
    device: &mut impl BlockDevice,
    current_slot: u64,
    mut superblock: Superblock,
) -> Result<(Superblock, u64), Error> {
    superblock.generation = superblock
        .generation
        .checked_add(1)
        .ok_or(Error::Conflict)?;
    let target = 1 - (current_slot & 1);
    device.write(target, &superblock.encode())?;
    device.flush()?;
    Ok((superblock, target))
}

/// Result of selecting and transitioning NovaFS mount metadata. This does not
/// represent a mounted filesystem or provide inode/path operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountMetadata {
    pub superblock: Superblock,
    pub active_slot: u64,
    pub replayed_writes: usize,
    pub degraded_mirror: bool,
}

/// Selects consistent mirrored metadata, records the dirty mounted state and,
/// after an unclean prior mount, replays the existing journal before returning.
/// A replay failure is persisted as `NeedsFsck` and returned to the caller.
pub fn prepare_mount(device: &mut impl BlockDevice) -> Result<MountMetadata, Error> {
    let (mut superblock, mut active_slot, degraded_mirror) = select_superblock(device)?;
    if superblock.layout.total_blocks > device.block_count() {
        return Err(Error::InvalidLayout);
    }
    if superblock.state == CleanState::NeedsFsck {
        return Err(Error::Corrupt);
    }
    let action = superblock.transition(MountEvent::Mount)?;
    (superblock, active_slot) = persist_superblock(device, active_slot, superblock)?;
    let mut replayed_writes = 0;
    if action == RecoveryAction::ReplayJournal {
        match Journal::replay(device, &superblock) {
            Ok(count) => {
                replayed_writes = count;
                superblock.transition(MountEvent::ReplayOk)?;
                (superblock, active_slot) = persist_superblock(device, active_slot, superblock)?;
            }
            Err(error) => {
                superblock.transition(MountEvent::ReplayFailed)?;
                persist_superblock(device, active_slot, superblock)?;
                return Err(error);
            }
        }
    }
    Ok(MountMetadata {
        superblock,
        active_slot,
        replayed_writes,
        degraded_mirror,
    })
}

/// Persists a clean-unmount transition for metadata returned by
/// [`prepare_mount`].
pub fn mark_clean_unmount(
    device: &mut impl BlockDevice,
    mut mounted: MountMetadata,
) -> Result<MountMetadata, Error> {
    mounted.superblock.transition(MountEvent::Unmount)?;
    (mounted.superblock, mounted.active_slot) =
        persist_superblock(device, mounted.active_slot, mounted.superblock)?;
    Ok(mounted)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extent {
    pub logical: u64,
    pub physical: u64,
    pub blocks: u32,
    pub flags: u16,
}
impl Extent {
    pub fn valid(&self, total: u64) -> bool {
        self.blocks > 0
            && self
                .physical
                .checked_add(self.blocks as u64)
                .is_some_and(|x| x <= total)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InodeKind {
    File = 1,
    Directory = 2,
    Symlink = 3,
    Device = 4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inode {
    pub number: u64,
    pub generation: u64,
    pub kind: InodeKind,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub links: u32,
    pub extent_count: u8,
    pub extents: [Extent; MAX_EXTENTS],
    pub xattr_block: u64,
    pub quota_id: u32,
}
impl Inode {
    pub fn validate(&self, total: u64) -> Result<(), Error> {
        if self.number == 0
            || self.links == 0
            || self.mode & !0o777 != 0
            || self.extent_count as usize > MAX_EXTENTS
            || (self.xattr_block != 0 && self.xattr_block >= total)
        {
            return Err(Error::Corrupt);
        };
        let mut last = 0;
        for (n, e) in self.extents[..self.extent_count as usize]
            .iter()
            .enumerate()
        {
            if !e.valid(total) || (n > 0 && e.logical < last) {
                return Err(Error::Corrupt);
            }
            last = e
                .logical
                .checked_add(e.blocks as u64)
                .ok_or(Error::Corrupt)?;
        }
        Ok(())
    }
    pub fn encode(&self) -> [u8; 256] {
        let mut o = [0; 256];
        p64(&mut o, 0, self.number);
        p64(&mut o, 8, self.generation);
        o[16] = self.kind as u8;
        o[17] = self.extent_count;
        p16(&mut o, 18, self.mode);
        p32(&mut o, 20, self.uid);
        p32(&mut o, 24, self.gid);
        p64(&mut o, 32, self.size);
        p32(&mut o, 40, self.links);
        p64(&mut o, 48, self.xattr_block);
        p32(&mut o, 56, self.quota_id);
        for (n, e) in self.extents.iter().enumerate() {
            let a = 64 + n * 28;
            p64(&mut o, a, e.logical);
            p64(&mut o, a + 8, e.physical);
            p32(&mut o, a + 16, e.blocks);
            p16(&mut o, a + 20, e.flags);
        }
        let c = checksum(&o[..252]);
        p32(&mut o, 252, c);
        o
    }

    pub fn decode(i: &[u8; 256], total_blocks: u64) -> Result<Self, Error> {
        if g32(i, 252) != checksum(&i[..252]) {
            return Err(Error::Corrupt);
        }
        let kind = match i[16] {
            1 => InodeKind::File,
            2 => InodeKind::Directory,
            3 => InodeKind::Symlink,
            4 => InodeKind::Device,
            _ => return Err(Error::Corrupt),
        };
        let mut extents = [Extent::default(); MAX_EXTENTS];
        for (n, extent) in extents.iter_mut().enumerate() {
            let at = 64 + n * 28;
            *extent = Extent {
                logical: g64(i, at),
                physical: g64(i, at + 8),
                blocks: g32(i, at + 16),
                flags: g16(i, at + 20),
            };
        }
        let inode = Self {
            number: g64(i, 0),
            generation: g64(i, 8),
            kind,
            mode: g16(i, 18),
            uid: g32(i, 20),
            gid: g32(i, 24),
            size: g64(i, 32),
            links: g32(i, 40),
            extent_count: i[17],
            extents,
            xattr_block: g64(i, 48),
            quota_id: g32(i, 56),
        };
        inode.validate(total_blocks)?;
        Ok(inode)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub inode: u64,
    pub kind: u8,
    pub name_len: u8,
    pub name: [u8; 238],
}
impl DirectoryEntry {
    pub fn new(inode: u64, kind: u8, name: &[u8]) -> Result<Self, Error> {
        if inode == 0
            || name.is_empty()
            || name.len() > 238
            || name.iter().any(|b| *b == 0 || *b == b'/')
        {
            return Err(Error::Corrupt);
        }
        let mut n = [0; 238];
        n[..name.len()].copy_from_slice(name);
        Ok(Self {
            inode,
            kind,
            name_len: name.len() as u8,
            name: n,
        })
    }
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
}

pub struct Bitmap<'a> {
    bits: &'a mut [u8],
}
impl<'a> Bitmap<'a> {
    pub fn new(bits: &'a mut [u8]) -> Self {
        Self { bits }
    }
    pub fn is_set(&self, b: u64) -> bool {
        let n = b as usize;
        self.bits
            .get(n / 8)
            .is_some_and(|x| x & (1 << (n % 8)) != 0)
    }
    pub fn set(&mut self, b: u64, value: bool) -> Result<(), Error> {
        let n = b as usize;
        let x = self.bits.get_mut(n / 8).ok_or(Error::OutOfBounds)?;
        if value {
            *x |= 1 << (n % 8)
        } else {
            *x &= !(1 << (n % 8))
        }
        Ok(())
    }
    pub fn allocate_run(&mut self, start: u64, end: u64, count: u32) -> Result<u64, Error> {
        if count == 0 {
            return Err(Error::NoSpace);
        }
        let mut run = 0;
        let mut first = start;
        for b in start..end {
            if !self.is_set(b) {
                if run == 0 {
                    first = b
                }
                run += 1;
                if run == count {
                    for x in first..first + count as u64 {
                        self.set(x, true)?
                    }
                    return Ok(first);
                }
            } else {
                run = 0
            }
        }
        Err(Error::NoSpace)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaRecord {
    pub id: u32,
    pub used_blocks: u64,
    pub hard_blocks: u64,
    pub used_inodes: u64,
    pub hard_inodes: u64,
}
impl QuotaRecord {
    pub fn charge(&mut self, blocks: u64, inodes: u64) -> Result<(), Error> {
        let b = self.used_blocks.checked_add(blocks).ok_or(Error::NoSpace)?;
        let i = self.used_inodes.checked_add(inodes).ok_or(Error::NoSpace)?;
        if (self.hard_blocks != 0 && b > self.hard_blocks)
            || (self.hard_inodes != 0 && i > self.hard_inodes)
        {
            return Err(Error::NoSpace);
        }
        self.used_blocks = b;
        self.used_inodes = i;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AclEntry {
    pub principal: u32,
    pub kind: u8,
    pub allow: u16,
    pub deny: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XattrRecord {
    pub namespace: u8,
    pub key_len: u8,
    pub value_len: u16,
    pub key: [u8; 32],
    pub value: [u8; 92],
}
impl XattrRecord {
    pub fn new(namespace: u8, key: &[u8], value: &[u8]) -> Result<Self, Error> {
        if key.is_empty() || key.len() > 32 || value.len() > 92 {
            return Err(Error::Full);
        }
        let mut k = [0; 32];
        let mut v = [0; 92];
        k[..key.len()].copy_from_slice(key);
        v[..value.len()].copy_from_slice(value);
        Ok(Self {
            namespace,
            key_len: key.len() as u8,
            value_len: value.len() as u16,
            key: k,
            value: v,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refcount {
    pub block: u64,
    pub blocks: u32,
    pub refs: u32,
}
impl Refcount {
    pub fn acquire(&mut self) -> Result<(), Error> {
        self.refs = self.refs.checked_add(1).ok_or(Error::Full)?;
        Ok(())
    }
    pub fn release(&mut self) -> Result<bool, Error> {
        if self.refs == 0 {
            return Err(Error::Corrupt);
        }
        self.refs -= 1;
        Ok(self.refs == 0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub id: u64,
    pub generation: u64,
    pub root_inode: u64,
    pub created_ns: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxState {
    Empty,
    Preparing,
    Committed,
    Applied,
}
#[derive(Clone, Copy)]
pub struct JournalWrite {
    pub target: u64,
    pub data: [u8; BLOCK_SIZE],
}
pub struct Transaction<const N: usize> {
    pub sequence: u64,
    state: TxState,
    writes: [Option<JournalWrite>; N],
    len: usize,
}
impl<const N: usize> Transaction<N> {
    pub const fn new(sequence: u64) -> Self {
        Self {
            sequence,
            state: TxState::Empty,
            writes: [None; N],
            len: 0,
        }
    }
    pub fn stage(&mut self, target: u64, data: &[u8; BLOCK_SIZE]) -> Result<(), Error> {
        if self.state == TxState::Committed || self.state == TxState::Applied {
            return Err(Error::Conflict);
        }
        if self.len == N {
            return Err(Error::Full);
        }
        self.writes[self.len] = Some(JournalWrite {
            target,
            data: *data,
        });
        self.len += 1;
        self.state = TxState::Preparing;
        Ok(())
    }
    pub fn state(&self) -> TxState {
        self.state
    }
}

fn journal_header(sequence: u64, count: u16, state: TxState) -> [u8; BLOCK_SIZE] {
    let mut o = [0; BLOCK_SIZE];
    p32(&mut o, 0, JOURNAL_MAGIC);
    p64(&mut o, 8, sequence);
    p16(&mut o, 16, count);
    o[18] = state as u8;
    let c = checksum(&o[..32]);
    p32(&mut o, 32, c);
    o
}
fn parse_header(i: &[u8; BLOCK_SIZE]) -> Option<(u64, u16, TxState)> {
    if g32(i, 0) != JOURNAL_MAGIC || g32(i, 32) != checksum(&i[..32]) {
        return None;
    }
    let s = match i[18] {
        1 => TxState::Preparing,
        2 => TxState::Committed,
        3 => TxState::Applied,
        _ => return None,
    };
    Some((g64(i, 8), g16(i, 16), s))
}
fn journal_record(target: u64, data: &[u8; BLOCK_SIZE]) -> [u8; BLOCK_SIZE] {
    let mut o = [0; BLOCK_SIZE];
    p64(&mut o, 0, target);
    let c = checksum(data);
    p32(&mut o, 8, c);
    o
}

pub struct Journal;
impl Journal {
    pub fn commit<const N: usize>(
        dev: &mut impl BlockDevice,
        sb: &Superblock,
        tx: &mut Transaction<N>,
    ) -> Result<(), Error> {
        if tx.len == 0 {
            return Ok(());
        }
        if 1 + tx.len as u32 * 2 > sb.layout.journal_blocks {
            return Err(Error::Full);
        }
        for (n, w) in tx.writes[..tx.len].iter().flatten().enumerate() {
            if w.target < sb.layout.data_start || w.target >= sb.layout.total_blocks {
                return Err(Error::OutOfBounds);
            }
            let metadata = sb.layout.journal_start + 1 + n as u64 * 2;
            dev.write(metadata, &journal_record(w.target, &w.data))?;
            dev.write(metadata + 1, &w.data)?;
        }
        dev.flush()?;
        dev.write(
            sb.layout.journal_start,
            &journal_header(tx.sequence, tx.len as u16, TxState::Preparing),
        )?;
        dev.flush()?;
        dev.write(
            sb.layout.journal_start,
            &journal_header(tx.sequence, tx.len as u16, TxState::Committed),
        )?;
        dev.flush()?;
        tx.state = TxState::Committed;
        for w in tx.writes[..tx.len].iter().flatten() {
            dev.write(w.target, &w.data)?;
        }
        dev.flush()?;
        dev.write(
            sb.layout.journal_start,
            &journal_header(tx.sequence, tx.len as u16, TxState::Applied),
        )?;
        dev.flush()?;
        tx.state = TxState::Applied;
        Ok(())
    }
    pub fn replay(dev: &mut impl BlockDevice, sb: &Superblock) -> Result<usize, Error> {
        let mut h = [0; BLOCK_SIZE];
        dev.read(sb.layout.journal_start, &mut h)?;
        let Some((_seq, count, state)) = parse_header(&h) else {
            return Ok(0);
        };
        if count as u32 * 2 + 1 > sb.layout.journal_blocks {
            return Err(Error::Corrupt);
        }
        if state == TxState::Preparing || state == TxState::Applied {
            return Ok(0);
        }
        let mut r = [0; BLOCK_SIZE];
        let mut data = [0; BLOCK_SIZE];
        for n in 0..count as u64 {
            let metadata = sb.layout.journal_start + 1 + n * 2;
            dev.read(metadata, &mut r)?;
            dev.read(metadata + 1, &mut data)?;
            let target = g64(&r, 0);
            if target < sb.layout.data_start
                || target >= sb.layout.total_blocks
                || g32(&r, 8) != checksum(&data)
            {
                return Err(Error::Corrupt);
            }
            dev.write(target, &data)?;
        }
        dev.flush()?;
        dev.write(
            sb.layout.journal_start,
            &journal_header(_seq, count, TxState::Applied),
        )?;
        dev.flush()?;
        Ok(count as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    BadSuperblockMirror,
    AllocatedMetadataMissing,
    ExtentOutOfRange,
    DuplicateAllocation,
    RefcountMismatch,
    OrphanInode,
    JournalCorrupt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    RewriteMirror,
    MarkAllocated(u64),
    ClearInode(u64),
    RebuildRefcounts,
    ReplayJournal,
    SetNeedsFsck,
}
pub struct RepairPlan {
    pub repairs: [Option<Repair>; MAX_REPAIRS],
    pub len: usize,
    pub unrepairable: bool,
}
impl RepairPlan {
    pub const fn new() -> Self {
        Self {
            repairs: [None; MAX_REPAIRS],
            len: 0,
            unrepairable: false,
        }
    }
    pub fn push(&mut self, r: Repair) {
        if self.len < MAX_REPAIRS {
            self.repairs[self.len] = Some(r);
            self.len += 1
        } else {
            self.unrepairable = true
        }
    }
}
impl Default for RepairPlan {
    fn default() -> Self {
        Self::new()
    }
}
pub struct Fsck;
impl Fsck {
    pub fn verify_superblocks(dev: &mut impl BlockDevice) -> RepairPlan {
        let mut p = RepairPlan::new();
        let mut a = [0; BLOCK_SIZE];
        let mut b = [0; BLOCK_SIZE];
        let va = dev
            .read(0, &mut a)
            .ok()
            .and_then(|_| Superblock::decode(&a).ok());
        let vb = dev
            .read(1, &mut b)
            .ok()
            .and_then(|_| Superblock::decode(&b).ok());
        match (va, vb) {
            (Some(x), Some(y)) if x.uuid == y.uuid => {}
            (Some(_), None) | (None, Some(_)) => p.push(Repair::RewriteMirror),
            _ => {
                p.push(Repair::SetNeedsFsck);
                p.unrepairable = true
            }
        }
        p
    }
    pub fn verify_inode(
        inode: &Inode,
        sb: &Superblock,
        bitmap: &Bitmap<'_>,
        plan: &mut RepairPlan,
    ) {
        if inode.validate(sb.layout.total_blocks).is_err() {
            plan.push(Repair::ClearInode(inode.number));
            return;
        }
        for e in &inode.extents[..inode.extent_count as usize] {
            for b in e.physical..e.physical + e.blocks as u64 {
                if !bitmap.is_set(b) {
                    plan.push(Repair::MarkAllocated(b));
                }
            }
        }
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;
    #[derive(Clone)]
    struct Mem {
        b: Vec<[u8; BLOCK_SIZE]>,
        fail: Option<usize>,
    }
    impl Mem {
        fn new(n: usize) -> Self {
            Self {
                b: vec![[0; BLOCK_SIZE]; n],
                fail: None,
            }
        }
    }
    impl BlockDevice for Mem {
        fn block_count(&self) -> u64 {
            self.b.len() as u64
        }
        fn read(&mut self, n: u64, o: &mut [u8; BLOCK_SIZE]) -> Result<(), Error> {
            *o = *self.b.get(n as usize).ok_or(Error::OutOfBounds)?;
            Ok(())
        }
        fn write(&mut self, n: u64, d: &[u8; BLOCK_SIZE]) -> Result<(), Error> {
            if let Some(x) = &mut self.fail {
                if *x == 0 {
                    return Err(Error::Io);
                }
                *x -= 1;
            }
            *self.b.get_mut(n as usize).ok_or(Error::OutOfBounds)? = *d;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
    fn sb() -> Superblock {
        Superblock {
            generation: 4,
            uuid: [7; 16],
            layout: Layout {
                total_blocks: 128,
                bitmap_start: 2,
                bitmap_blocks: 1,
                inode_start: 3,
                inode_blocks: 2,
                journal_start: 5,
                journal_blocks: 8,
                refcount_start: 13,
                refcount_blocks: 1,
                data_start: 14,
            },
            root_inode: 1,
            journal_sequence: 3,
            state: CleanState::Clean,
        }
    }
    #[test]
    fn superblock_mirror_selects_newest() {
        let mut d = Mem::new(128);
        let a = sb();
        let mut b = a;
        b.generation = 9;
        d.write(0, &a.encode()).unwrap();
        d.write(1, &b.encode()).unwrap();
        assert_eq!(read_superblock(&mut d).unwrap().0.generation, 9)
    }
    #[test]
    fn clean_mount_is_persisted_dirty_then_clean_across_mirrors() {
        let mut d = Mem::new(128);
        let a = sb();
        let mut b = a;
        b.generation = 5;
        d.write(0, &a.encode()).unwrap();
        d.write(1, &b.encode()).unwrap();

        let mounted = prepare_mount(&mut d).unwrap();
        assert_eq!(mounted.superblock.state, CleanState::Mounted);
        assert_eq!(mounted.superblock.generation, 6);
        assert_eq!(mounted.active_slot, 0);
        assert_eq!(mounted.replayed_writes, 0);
        assert!(!mounted.degraded_mirror);

        let unmounted = mark_clean_unmount(&mut d, mounted).unwrap();
        assert_eq!(unmounted.superblock.state, CleanState::Clean);
        assert_eq!(unmounted.superblock.generation, 7);
        assert_eq!(unmounted.active_slot, 1);
        assert_eq!(read_superblock(&mut d).unwrap(), (unmounted.superblock, 1));
    }
    #[test]
    fn dirty_mount_replays_committed_journal_before_returning() {
        let mut d = Mem::new(128);
        let mut s = sb();
        s.state = CleanState::Mounted;
        d.write(0, &s.encode()).unwrap();
        let mut payload = [0u8; BLOCK_SIZE];
        payload[19] = 77;
        d.b[s.layout.journal_start as usize] = journal_header(9, 1, TxState::Committed);
        d.b[s.layout.journal_start as usize + 1] = journal_record(30, &payload);
        d.b[s.layout.journal_start as usize + 2] = payload;

        let mounted = prepare_mount(&mut d).unwrap();
        assert_eq!(mounted.superblock.state, CleanState::Mounted);
        assert_eq!(mounted.replayed_writes, 1);
        assert_eq!(d.b[30][19], 77);
        assert_eq!(
            parse_header(&d.b[s.layout.journal_start as usize])
                .unwrap()
                .2,
            TxState::Applied
        );
    }
    #[test]
    fn one_corrupt_mirror_is_degraded_but_both_corrupt_fail_closed() {
        let mut degraded = Mem::new(128);
        degraded.write(0, &sb().encode()).unwrap();
        degraded.b[1][0] = 0xff;
        let mounted = prepare_mount(&mut degraded).unwrap();
        assert!(mounted.degraded_mirror);

        let mut corrupt = Mem::new(128);
        let before = corrupt.b.clone();
        assert_eq!(prepare_mount(&mut corrupt), Err(Error::Corrupt));
        assert_eq!(corrupt.b, before);
    }
    #[test]
    fn inconsistent_valid_mirrors_fail_closed() {
        let mut d = Mem::new(128);
        let a = sb();
        let mut split_brain = a;
        split_brain.journal_sequence += 1;
        d.write(0, &a.encode()).unwrap();
        d.write(1, &split_brain.encode()).unwrap();
        assert_eq!(prepare_mount(&mut d), Err(Error::Conflict));

        split_brain.generation += 1;
        split_brain.uuid = [9; 16];
        d.write(1, &split_brain.encode()).unwrap();
        assert_eq!(prepare_mount(&mut d), Err(Error::Conflict));
    }
    #[test]
    fn corrupt_replay_marks_volume_needs_fsck_and_stays_closed() {
        let mut d = Mem::new(128);
        let mut s = sb();
        s.state = CleanState::Mounted;
        d.write(0, &s.encode()).unwrap();
        d.b[s.layout.journal_start as usize] = journal_header(9, 1, TxState::Committed);
        // The zero record targets metadata block zero, which replay must reject.
        assert_eq!(prepare_mount(&mut d), Err(Error::Corrupt));
        let (persisted, _) = read_superblock(&mut d).unwrap();
        assert_eq!(persisted.state, CleanState::NeedsFsck);
        assert_eq!(prepare_mount(&mut d), Err(Error::Corrupt));
    }
    #[test]
    fn corrupt_mirror_is_repairable() {
        let mut d = Mem::new(128);
        d.write(0, &sb().encode()).unwrap();
        let p = Fsck::verify_superblocks(&mut d);
        assert_eq!(p.repairs[0], Some(Repair::RewriteMirror));
        assert!(!p.unrepairable)
    }
    #[test]
    fn bitmap_first_fit_and_free() {
        let mut x = [0u8; 4];
        let mut b = Bitmap::new(&mut x);
        b.set(2, true).unwrap();
        assert_eq!(b.allocate_run(0, 32, 3).unwrap(), 3);
        for n in 3..6 {
            assert!(b.is_set(n))
        }
    }
    #[test]
    fn quotas_are_atomic() {
        let mut q = QuotaRecord {
            id: 1,
            used_blocks: 8,
            hard_blocks: 10,
            used_inodes: 1,
            hard_inodes: 2,
        };
        assert_eq!(q.charge(3, 0), Err(Error::NoSpace));
        assert_eq!(q.used_blocks, 8);
        q.charge(2, 1).unwrap()
    }
    #[test]
    fn inode_and_fsck_find_missing_allocations() {
        let s = sb();
        let mut bits = [0u8; 32];
        let bm = Bitmap::new(&mut bits);
        let mut ex = [Extent::default(); MAX_EXTENTS];
        ex[0] = Extent {
            logical: 0,
            physical: 20,
            blocks: 2,
            flags: 0,
        };
        let i = Inode {
            number: 2,
            generation: 1,
            kind: InodeKind::File,
            mode: 0o644,
            uid: 1,
            gid: 1,
            size: 8192,
            links: 1,
            extent_count: 1,
            extents: ex,
            xattr_block: 0,
            quota_id: 1,
        };
        let mut p = RepairPlan::new();
        Fsck::verify_inode(&i, &s, &bm, &mut p);
        assert_eq!(p.len, 2)
    }
    #[test]
    fn inode_decode_roundtrips_and_rejects_corruption() {
        let mut extents = [Extent::default(); MAX_EXTENTS];
        extents[0] = Extent {
            logical: 0,
            physical: 20,
            blocks: 2,
            flags: 3,
        };
        let inode = Inode {
            number: 2,
            generation: 7,
            kind: InodeKind::File,
            mode: 0o640,
            uid: 1000,
            gid: 100,
            size: 5000,
            links: 1,
            extent_count: 1,
            extents,
            xattr_block: 40,
            quota_id: 1000,
        };
        let encoded = inode.encode();
        assert_eq!(Inode::decode(&encoded, 128), Ok(inode));

        let mut bad_checksum = encoded;
        bad_checksum[20] ^= 1;
        assert_eq!(Inode::decode(&bad_checksum, 128), Err(Error::Corrupt));

        let mut bad_kind = encoded;
        bad_kind[16] = 99;
        let sum = checksum(&bad_kind[..252]);
        p32(&mut bad_kind, 252, sum);
        assert_eq!(Inode::decode(&bad_kind, 128), Err(Error::Corrupt));

        let mut bad_extent = inode;
        bad_extent.extents[0].physical = 127;
        assert_eq!(
            Inode::decode(&bad_extent.encode(), 128),
            Err(Error::Corrupt)
        );

        let mut overflowing_logical_extent = inode;
        overflowing_logical_extent.extents[0].logical = u64::MAX;
        assert_eq!(
            Inode::decode(&overflowing_logical_extent.encode(), 128),
            Err(Error::Corrupt)
        );
    }
    #[test]
    fn directory_rejects_invalid_names() {
        assert!(DirectoryEntry::new(1, 1, b"valid").is_ok());
        assert_eq!(DirectoryEntry::new(1, 1, b"a/b"), Err(Error::Corrupt))
    }
    #[test]
    fn xattrs_and_refcounts() {
        let x = XattrRecord::new(1, b"nova.label", b"trusted").unwrap();
        assert_eq!(x.key_len, 10);
        let mut r = Refcount {
            block: 20,
            blocks: 2,
            refs: 1,
        };
        r.acquire().unwrap();
        assert!(!r.release().unwrap());
        assert!(r.release().unwrap())
    }
    #[test]
    fn committed_transaction_replays() {
        let mut d = Mem::new(128);
        let s = sb();
        let mut tx = Transaction::<3>::new(8);
        let mut data = [0u8; BLOCK_SIZE];
        data[0] = 17;
        data[15] = 23;
        data[20] = 42;
        tx.stage(30, &data).unwrap();
        Journal::commit(&mut d, &s, &mut tx).unwrap();
        d.b[30] = [0; BLOCK_SIZE];
        d.b[s.layout.journal_start as usize] = journal_header(8, 1, TxState::Committed);
        assert_eq!(Journal::replay(&mut d, &s).unwrap(), 1);
        assert_eq!(d.b[30][0], 17);
        assert_eq!(d.b[30][15], 23);
        assert_eq!(d.b[30][20], 42)
    }
    #[test]
    fn preparing_transaction_is_never_replayed() {
        let mut d = Mem::new(128);
        let s = sb();
        d.b[s.layout.journal_start as usize] = journal_header(1, 1, TxState::Preparing);
        assert_eq!(Journal::replay(&mut d, &s).unwrap(), 0)
    }
    #[test]
    fn deterministic_power_loss_is_recoverable() {
        for cut in 0..7 {
            let mut d = Mem::new(128);
            let s = sb();
            let mut tx = Transaction::<1>::new(10);
            let mut data = [0u8; BLOCK_SIZE];
            data[100] = 99;
            tx.stage(40, &data).unwrap();
            d.fail = Some(cut);
            let _ = Journal::commit(&mut d, &s, &mut tx);
            d.fail = None;
            let _ = Journal::replay(&mut d, &s);
            let v = d.b[40][100];
            assert!(v == 0 || v == 99, "cut {cut}: {v}");
        }
    }
    #[test]
    fn layout_rejects_overlap() {
        let mut s = sb();
        s.layout.inode_start = 2;
        assert_eq!(s.layout.validate(), Err(Error::InvalidLayout))
    }
    #[test]
    fn crash_state_machine_requires_replay() {
        let mut s = sb();
        assert_eq!(
            s.transition(MountEvent::Mount),
            Ok(RecoveryAction::Continue)
        );
        assert_eq!(
            s.transition(MountEvent::Mount),
            Ok(RecoveryAction::ReplayJournal)
        );
        assert_eq!(
            s.transition(MountEvent::ReplayOk),
            Ok(RecoveryAction::Continue)
        );
        assert_eq!(
            s.transition(MountEvent::Unmount),
            Ok(RecoveryAction::Continue)
        );
        assert_eq!(s.state, CleanState::Clean);
    }
}
