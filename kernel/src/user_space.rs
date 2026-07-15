use crate::{
    memory::{self, GlobalFrames},
    process::{PageRole, UserAddressSpace},
};
use bootloader_api::BootInfo;
use core::{
    mem, ptr,
    sync::atomic::{AtomicU64, Ordering},
};
use memory_core::{FrameLedger, PhysicalFrame};
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame,
        Size4KiB, Translate,
    },
};

#[derive(Debug, Clone, Copy)]
pub enum UserSpaceError {
    NoPhysicalMap,
    NoFrames,
    Map,
    InvalidAddress,
}

static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);
static PHYSICAL_MEMORY_OFFSET: AtomicU64 = AtomicU64::new(u64::MAX);
pub const MAX_USER_SPACE_FRAMES: usize = 32;

struct TrackingFrames {
    global: GlobalFrames,
    owned: FrameLedger<MAX_USER_SPACE_FRAMES>,
}

impl TrackingFrames {
    const fn new() -> Self {
        Self {
            global: GlobalFrames,
            owned: FrameLedger::new(),
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for TrackingFrames {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let frame = self.global.allocate_frame()?;
        if self
            .owned
            .record(PhysicalFrame(frame.start_address().as_u64()))
            .is_err()
        {
            let _ = memory::release(frame);
            return None;
        }
        Some(frame)
    }
}

pub struct AddressSpaceOwner {
    level4_address: u64,
    frames: FrameLedger<MAX_USER_SPACE_FRAMES>,
}

impl AddressSpaceOwner {
    pub const fn level4_address(&self) -> u64 {
        self.level4_address
    }

    pub fn reclaim(mut self) -> usize {
        let mut reclaimed = 0;
        while let Some(frame) = self.frames.take_last() {
            let Ok(frame) = PhysFrame::from_start_address(PhysAddr::new(frame.0)) else {
                continue;
            };
            if memory::release(frame) {
                reclaimed += 1;
            }
        }
        reclaimed
    }
}

/// A process-owned level-4 table. Kernel mappings remain supervisor-only,
/// while PML4 slot zero is rebuilt exclusively from USER mappings.
pub struct CurrentUserSpace {
    mapper: OffsetPageTable<'static>,
    frames: TrackingFrames,
    level4_frame: PhysFrame<Size4KiB>,
    physical_offset: VirtAddr,
}

impl CurrentUserSpace {
    /// Creates a user mapper for new low-half pages while retaining supervisor-only kernel mappings.
    pub unsafe fn new(boot_info: &BootInfo) -> Result<Self, UserSpaceError> {
        let offset = boot_info
            .physical_memory_offset
            .into_option()
            .ok_or(UserSpaceError::NoPhysicalMap)?;
        PHYSICAL_MEMORY_OFFSET.store(offset, Ordering::Release);
        let (kernel_level4, _) = Cr3::read();
        let kernel_table_address = VirtAddr::new(offset + kernel_level4.start_address().as_u64());
        let kernel_table = unsafe { &*kernel_table_address.as_ptr::<PageTable>() };
        let mut frames = TrackingFrames::new();
        if crate::memory::available() < 16 {
            return Err(UserSpaceError::NoFrames);
        }
        let level4_frame = frames.allocate_frame().ok_or(UserSpaceError::NoFrames)?;
        let table_address = VirtAddr::new(offset + level4_frame.start_address().as_u64());
        let table = unsafe { &mut *table_address.as_mut_ptr::<PageTable>() };
        table.zero();
        for index in 1..512 {
            table[index] = kernel_table[index].clone();
        }
        Ok(Self {
            mapper: unsafe { OffsetPageTable::new(table, VirtAddr::new(offset)) },
            frames,
            level4_frame,
            physical_offset: VirtAddr::new(offset),
        })
    }

    pub fn level4_address(&self) -> u64 {
        self.level4_frame.start_address().as_u64()
    }

    pub fn physical_address(&self, virtual_address: u64) -> Option<u64> {
        self.mapper
            .translate_addr(VirtAddr::new(virtual_address))
            .map(|address| address.as_u64())
    }

    pub fn into_owner(mut self) -> AddressSpaceOwner {
        let frames = mem::take(&mut self.frames.owned);
        AddressSpaceOwner {
            level4_address: self.level4_address(),
            frames,
        }
    }
}

impl Drop for CurrentUserSpace {
    fn drop(&mut self) {
        let frames = mem::take(&mut self.frames.owned);
        let owner = AddressSpaceOwner {
            level4_address: self.level4_address(),
            frames,
        };
        let _ = owner.reclaim();
    }
}

unsafe impl UserAddressSpace for CurrentUserSpace {
    type Error = UserSpaceError;

    fn map_zeroed_page(&mut self, virtual_address: u64, role: PageRole) -> Result<(), Self::Error> {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(virtual_address))
            .map_err(|_| UserSpaceError::InvalidAddress)?;
        let frame = self
            .frames
            .allocate_frame()
            .ok_or(UserSpaceError::NoFrames)?;
        // New pages are writable while the kernel zeroes/populates them. Code is
        // sealed RX by `seal_code_page` before CPL3 starts.
        let mut flags =
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        if !role.executable() {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        unsafe { self.mapper.map_to(page, frame, flags, &mut self.frames) }
            .map_err(|_| UserSpaceError::Map)?
            .flush();
        let physical = self.physical_offset + frame.start_address().as_u64();
        unsafe {
            ptr::write_bytes(physical.as_mut_ptr::<u8>(), 0, 4096);
        }
        Ok(())
    }

    fn write_user(&mut self, virtual_address: u64, bytes: &[u8]) -> Result<(), Self::Error> {
        if bytes.len() > 4096 {
            return Err(UserSpaceError::InvalidAddress);
        }
        let physical = self
            .mapper
            .translate_addr(VirtAddr::new(virtual_address))
            .ok_or(UserSpaceError::InvalidAddress)?;
        let destination = self.physical_offset + physical.as_u64();
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), destination.as_mut_ptr::<u8>(), bytes.len());
        }
        Ok(())
    }

    fn seal_code_page(&mut self, virtual_address: u64) -> Result<(), Self::Error> {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(virtual_address))
            .map_err(|_| UserSpaceError::InvalidAddress)?;
        unsafe {
            self.mapper.update_flags(
                page,
                PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE,
            )
        }
        .map_err(|_| UserSpaceError::Map)?
        .flush();
        Ok(())
    }

    unsafe fn activate(&mut self) -> Result<(), Self::Error> {
        let (kernel, flags) = Cr3::read();
        KERNEL_CR3.store(
            kernel.start_address().as_u64() | flags.bits(),
            Ordering::Release,
        );
        unsafe {
            Cr3::write(self.level4_frame, flags);
        }
        Ok(())
    }
}

pub fn physical_memory_offset() -> Option<u64> {
    let offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::Acquire);
    (offset != u64::MAX).then_some(offset)
}

pub fn restore_kernel_address_space() -> bool {
    let saved = KERNEL_CR3.swap(0, Ordering::AcqRel);
    if saved == 0 {
        return false;
    }
    let address = saved & 0x000f_ffff_ffff_f000;
    let flags = x86_64::registers::control::Cr3Flags::from_bits_truncate(saved);
    let Ok(frame) = PhysFrame::from_start_address(PhysAddr::new(address)) else {
        return false;
    };
    unsafe {
        Cr3::write(frame, flags);
    }
    true
}
