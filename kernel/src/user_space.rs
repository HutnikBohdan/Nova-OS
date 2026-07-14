use crate::{
    memory::GlobalFrames,
    process::{PageRole, UserAddressSpace},
};
use bootloader_api::BootInfo;
use core::{
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};
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

/// A process-owned level-4 table. Kernel mappings remain supervisor-only,
/// while PML4 slot zero is rebuilt exclusively from USER mappings.
pub struct CurrentUserSpace {
    mapper: OffsetPageTable<'static>,
    frames: GlobalFrames,
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
        let (kernel_level4, _) = Cr3::read();
        let kernel_table_address = VirtAddr::new(offset + kernel_level4.start_address().as_u64());
        let kernel_table = unsafe { &*kernel_table_address.as_ptr::<PageTable>() };
        let mut frames = GlobalFrames;
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
