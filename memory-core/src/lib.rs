#![no_std]

pub const PAGE_SIZE: u64 = 4096;
pub const USER_ADDRESS_MAX: u64 = 0x0000_7fff_ffff_ffff;
pub const MAX_FRAMES: usize = 32_768;
pub const MAX_REGIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalFrame(pub u64);

/// Fixed-capacity ownership record for frames allocated to one kernel object.
/// Entries are unique and can be transferred without allocation.
pub struct FrameLedger<const N: usize> {
    frames: [Option<PhysicalFrame>; N],
    len: usize,
}

impl<const N: usize> FrameLedger<N> {
    pub const fn new() -> Self {
        Self {
            frames: [None; N],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, frame: PhysicalFrame) -> bool {
        self.frames[..self.len].contains(&Some(frame))
    }

    pub fn record(&mut self, frame: PhysicalFrame) -> Result<(), MemoryError> {
        if self.contains(frame) {
            return Err(MemoryError::InvalidFrame);
        }
        if self.len == N {
            return Err(MemoryError::TableFull);
        }
        self.frames[self.len] = Some(frame);
        self.len += 1;
        Ok(())
    }

    pub fn take_last(&mut self) -> Option<PhysicalFrame> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.frames[self.len].take()
    }
}

impl<const N: usize> Default for FrameLedger<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    Exhausted,
    InvalidFrame,
    InvalidRange,
    Overlap,
    TableFull,
    PermissionDenied,
}

/// A non-empty canonical low-half user range with an exclusive end address.
///
/// Kernel architecture code uses this model before walking the active page
/// tables. Keeping the overflow and boundary rules here makes them testable on
/// the host without weakening the hardware page-table checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserBufferRange {
    start: u64,
    end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserMemoryAccess {
    Read,
    Write,
}

/// Models the effective permission required at every x86-64 page-table level.
pub fn validate_user_page_permissions(
    present: bool,
    user_accessible: bool,
    writable: bool,
    access: UserMemoryAccess,
) -> Result<(), MemoryError> {
    if !present {
        return Err(MemoryError::InvalidRange);
    }
    if !user_accessible || matches!(access, UserMemoryAccess::Write) && !writable {
        return Err(MemoryError::PermissionDenied);
    }
    Ok(())
}

impl UserBufferRange {
    pub fn new(start: u64, length: usize) -> Result<Self, MemoryError> {
        if length == 0 || start > USER_ADDRESS_MAX {
            return Err(MemoryError::InvalidRange);
        }
        let end = start
            .checked_add(length as u64)
            .ok_or(MemoryError::InvalidRange)?;
        if end == 0 || end - 1 > USER_ADDRESS_MAX {
            return Err(MemoryError::InvalidRange);
        }
        Ok(Self { start, end })
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }

    pub const fn first_page(self) -> u64 {
        self.start & !(PAGE_SIZE - 1)
    }

    pub const fn last_page(self) -> u64 {
        (self.end - 1) & !(PAGE_SIZE - 1)
    }
}

pub struct FrameAllocator {
    base: u64,
    frame_count: usize,
    used: [u64; MAX_FRAMES / 64],
}

impl FrameAllocator {
    pub fn new(base: u64, length: u64) -> Result<Self, MemoryError> {
        if base % PAGE_SIZE != 0 || length < PAGE_SIZE {
            return Err(MemoryError::InvalidRange);
        }
        let frame_count = (length / PAGE_SIZE) as usize;
        if frame_count > MAX_FRAMES {
            return Err(MemoryError::InvalidRange);
        }
        Ok(Self {
            base,
            frame_count,
            used: [0; MAX_FRAMES / 64],
        })
    }
    pub fn allocate(&mut self) -> Result<PhysicalFrame, MemoryError> {
        for index in 0..self.frame_count {
            let word = index / 64;
            let bit = index % 64;
            if self.used[word] & (1u64 << bit) == 0 {
                self.used[word] |= 1u64 << bit;
                return Ok(PhysicalFrame(self.base + index as u64 * PAGE_SIZE));
            }
        }
        Err(MemoryError::Exhausted)
    }
    pub fn free(&mut self, frame: PhysicalFrame) -> Result<(), MemoryError> {
        if frame.0 < self.base || (frame.0 - self.base) % PAGE_SIZE != 0 {
            return Err(MemoryError::InvalidFrame);
        }
        let index = ((frame.0 - self.base) / PAGE_SIZE) as usize;
        if index >= self.frame_count {
            return Err(MemoryError::InvalidFrame);
        }
        let mask = 1u64 << (index % 64);
        if self.used[index / 64] & mask == 0 {
            return Err(MemoryError::InvalidFrame);
        }
        self.used[index / 64] &= !mask;
        Ok(())
    }
    pub fn reserve(&mut self, start: u64, length: u64) -> Result<(), MemoryError> {
        if start < self.base || start % PAGE_SIZE != 0 || length % PAGE_SIZE != 0 {
            return Err(MemoryError::InvalidRange);
        }
        let first = ((start - self.base) / PAGE_SIZE) as usize;
        let count = (length / PAGE_SIZE) as usize;
        if first
            .checked_add(count)
            .is_none_or(|end| end > self.frame_count)
        {
            return Err(MemoryError::InvalidRange);
        }
        for index in first..first + count {
            self.used[index / 64] |= 1u64 << (index % 64);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions(u8);
impl Permissions {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXECUTE: Self = Self(4);
    pub const USER: Self = Self(8);
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub virtual_start: u64,
    pub physical_start: u64,
    pub page_count: u32,
    pub permissions: Permissions,
}

impl Region {
    pub const fn virtual_end(self) -> u64 {
        self.virtual_start + self.page_count as u64 * PAGE_SIZE
    }
}

pub struct AddressSpace<const N: usize = MAX_REGIONS> {
    regions: [Option<Region>; N],
    len: usize,
}

impl<const N: usize> AddressSpace<N> {
    pub const fn new() -> Self {
        Self {
            regions: [None; N],
            len: 0,
        }
    }
    pub fn map(&mut self, region: Region) -> Result<(), MemoryError> {
        if region.page_count == 0
            || region.virtual_start % PAGE_SIZE != 0
            || region.physical_start % PAGE_SIZE != 0
            || region
                .virtual_start
                .checked_add(region.page_count as u64 * PAGE_SIZE)
                .is_none()
        {
            return Err(MemoryError::InvalidRange);
        }
        if self.len == N {
            return Err(MemoryError::TableFull);
        }
        for current in self.regions[..self.len].iter().flatten() {
            if region.virtual_start < current.virtual_end()
                && current.virtual_start < region.virtual_end()
            {
                return Err(MemoryError::Overlap);
            }
        }
        self.regions[self.len] = Some(region);
        self.len += 1;
        Ok(())
    }
    pub fn translate(&self, address: u64, required: Permissions) -> Result<u64, MemoryError> {
        let region = self.regions[..self.len]
            .iter()
            .flatten()
            .find(|region| address >= region.virtual_start && address < region.virtual_end())
            .ok_or(MemoryError::InvalidRange)?;
        if !region.permissions.contains(required) {
            return Err(MemoryError::PermissionDenied);
        }
        Ok(region.physical_start + address - region.virtual_start)
    }
    pub fn unmap(&mut self, virtual_start: u64) -> Option<Region> {
        let index = self.regions[..self.len]
            .iter()
            .position(|entry| entry.is_some_and(|r| r.virtual_start == virtual_start))?;
        let value = self.regions[index].take();
        for cursor in index..self.len - 1 {
            self.regions[cursor] = self.regions[cursor + 1];
        }
        self.len -= 1;
        self.regions[self.len] = None;
        value
    }
    pub const fn len(&self) -> usize {
        self.len
    }
}

impl<const N: usize> Default for AddressSpace<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_buffer_range_rejects_empty_overflow_and_kernel_addresses() {
        assert_eq!(
            UserBufferRange::new(0x1000, 0),
            Err(MemoryError::InvalidRange)
        );
        assert_eq!(
            UserBufferRange::new(u64::MAX - 1, 4),
            Err(MemoryError::InvalidRange)
        );
        assert_eq!(
            UserBufferRange::new(USER_ADDRESS_MAX + 1, 1),
            Err(MemoryError::InvalidRange)
        );
        assert_eq!(
            UserBufferRange::new(USER_ADDRESS_MAX, 2),
            Err(MemoryError::InvalidRange)
        );
    }

    #[test]
    fn user_buffer_range_identifies_every_touched_page() {
        let range = UserBufferRange::new(0x1fff, PAGE_SIZE as usize + 2).unwrap();
        assert_eq!(range.start(), 0x1fff);
        assert_eq!(range.end(), 0x3001);
        assert_eq!(range.first_page(), 0x1000);
        assert_eq!(range.last_page(), 0x3000);

        let top = UserBufferRange::new(USER_ADDRESS_MAX, 1).unwrap();
        assert_eq!(top.last_page(), USER_ADDRESS_MAX & !(PAGE_SIZE - 1));
    }

    #[test]
    fn user_page_permissions_fail_closed_for_each_required_bit() {
        assert_eq!(
            validate_user_page_permissions(false, true, true, UserMemoryAccess::Read),
            Err(MemoryError::InvalidRange)
        );
        assert_eq!(
            validate_user_page_permissions(true, false, true, UserMemoryAccess::Read),
            Err(MemoryError::PermissionDenied)
        );
        assert_eq!(
            validate_user_page_permissions(true, true, false, UserMemoryAccess::Write),
            Err(MemoryError::PermissionDenied)
        );
        assert!(validate_user_page_permissions(true, true, false, UserMemoryAccess::Read).is_ok());
        assert!(validate_user_page_permissions(true, true, true, UserMemoryAccess::Write).is_ok());
    }
    #[test]
    fn frames_allocate_free_and_reuse() {
        let mut allocator = FrameAllocator::new(0x100000, PAGE_SIZE * 2).unwrap();
        let first = allocator.allocate().unwrap();
        let second = allocator.allocate().unwrap();
        assert_eq!(allocator.allocate(), Err(MemoryError::Exhausted));
        allocator.free(first).unwrap();
        assert_eq!(allocator.free(first), Err(MemoryError::InvalidFrame));
        assert_eq!(allocator.allocate().unwrap(), first);
        assert_ne!(first, second);
    }
    #[test]
    fn frame_ledger_is_unique_bounded_and_drains_once() {
        let mut ledger = FrameLedger::<2>::new();
        let first = PhysicalFrame(0x1000);
        let second = PhysicalFrame(0x2000);
        ledger.record(first).unwrap();
        assert_eq!(ledger.record(first), Err(MemoryError::InvalidFrame));
        ledger.record(second).unwrap();
        assert_eq!(
            ledger.record(PhysicalFrame(0x3000)),
            Err(MemoryError::TableFull)
        );
        assert_eq!(ledger.take_last(), Some(second));
        assert_eq!(ledger.take_last(), Some(first));
        assert_eq!(ledger.take_last(), None);
        assert!(ledger.is_empty());
    }
    #[test]
    fn address_space_enforces_write_and_user_permissions() {
        let mut space = AddressSpace::<4>::new();
        space
            .map(Region {
                virtual_start: 0x400000,
                physical_start: 0x200000,
                page_count: 2,
                permissions: Permissions::READ.union(Permissions::USER),
            })
            .unwrap();
        assert_eq!(
            space.translate(0x400123, Permissions::READ).unwrap(),
            0x200123
        );
        assert_eq!(
            space.translate(0x400123, Permissions::WRITE),
            Err(MemoryError::PermissionDenied)
        );
    }
    #[test]
    fn overlapping_regions_are_rejected() {
        let mut space = AddressSpace::<4>::new();
        space
            .map(Region {
                virtual_start: 0x1000,
                physical_start: 0x9000,
                page_count: 2,
                permissions: Permissions::READ,
            })
            .unwrap();
        assert_eq!(
            space.map(Region {
                virtual_start: 0x2000,
                physical_start: 0xb000,
                page_count: 1,
                permissions: Permissions::READ
            }),
            Err(MemoryError::Overlap)
        );
    }
}
