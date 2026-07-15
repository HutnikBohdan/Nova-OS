//! Checked CPL3 memory transfer through the currently active x86-64 page tables.

use core::ptr;
use memory_core::{
    MemoryError, PAGE_SIZE, UserBufferRange, UserMemoryAccess, validate_user_page_permissions,
};
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{PageTable, PageTableFlags},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCopyError {
    InvalidRange,
    PhysicalMapUnavailable,
    NotPresent,
    SupervisorPage,
    ReadOnly,
    HugePage,
}

#[derive(Clone, Copy)]
enum Access {
    Read,
    Write,
}

impl From<MemoryError> for UserCopyError {
    fn from(_: MemoryError) -> Self {
        Self::InvalidRange
    }
}

/// Validates the complete source range before copying any user-controlled byte.
pub fn copy_from_user(address: u64, output: &mut [u8]) -> Result<(), UserCopyError> {
    validate(address, output.len(), Access::Read)?;
    // The interrupt gate keeps IRQs disabled and Nova is currently single-core,
    // so validated page tables cannot be replaced between validation and copy.
    unsafe {
        ptr::copy_nonoverlapping(address as *const u8, output.as_mut_ptr(), output.len());
    }
    Ok(())
}

/// Validates the complete destination range, including effective write access,
/// before modifying any userspace byte.
pub fn copy_to_user(address: u64, input: &[u8]) -> Result<(), UserCopyError> {
    validate(address, input.len(), Access::Write)?;
    unsafe {
        ptr::copy_nonoverlapping(input.as_ptr(), address as *mut u8, input.len());
    }
    Ok(())
}

/// Checks a future destination before a stateful operation such as dequeueing
/// an IPC message. The final copy still revalidates the same range.
pub fn validate_user_destination(address: u64, length: usize) -> Result<(), UserCopyError> {
    validate(address, length, Access::Write)
}

fn validate(address: u64, length: usize, access: Access) -> Result<(), UserCopyError> {
    let range = UserBufferRange::new(address, length)?;
    let physical_offset =
        crate::user_space::physical_memory_offset().ok_or(UserCopyError::PhysicalMapUnavailable)?;
    let (root, _) = Cr3::read();
    let root = root.start_address().as_u64();

    let mut page = range.first_page();
    loop {
        validate_page(root, physical_offset, page, access)?;
        if page == range.last_page() {
            break;
        }
        page = page
            .checked_add(PAGE_SIZE)
            .ok_or(UserCopyError::InvalidRange)?;
    }
    Ok(())
}

fn validate_page(
    root: u64,
    physical_offset: u64,
    virtual_address: u64,
    access: Access,
) -> Result<(), UserCopyError> {
    let indices = [
        ((virtual_address >> 39) & 0x1ff) as usize,
        ((virtual_address >> 30) & 0x1ff) as usize,
        ((virtual_address >> 21) & 0x1ff) as usize,
        ((virtual_address >> 12) & 0x1ff) as usize,
    ];
    let mut table_physical = root;
    for (level, index) in indices.into_iter().enumerate() {
        let table = physical_table(physical_offset, table_physical)?;
        let entry = &table[index];
        let flags = entry.flags();
        require_effective_flags(flags, access)?;
        if level < 3 && flags.contains(PageTableFlags::HUGE_PAGE) {
            // Nova does not create huge CPL3 mappings. Rejecting them keeps the
            // 4 KiB validation contract explicit and fail-closed.
            return Err(UserCopyError::HugePage);
        }
        table_physical = entry.addr().as_u64();
    }
    Ok(())
}

fn require_effective_flags(flags: PageTableFlags, access: Access) -> Result<(), UserCopyError> {
    let present = flags.contains(PageTableFlags::PRESENT);
    let user = flags.contains(PageTableFlags::USER_ACCESSIBLE);
    let writable = flags.contains(PageTableFlags::WRITABLE);
    let model_access = match access {
        Access::Read => UserMemoryAccess::Read,
        Access::Write => UserMemoryAccess::Write,
    };
    if validate_user_page_permissions(present, user, writable, model_access).is_ok() {
        return Ok(());
    }
    if !present {
        return Err(UserCopyError::NotPresent);
    }
    if !user {
        return Err(UserCopyError::SupervisorPage);
    }
    if matches!(access, Access::Write) && !writable {
        return Err(UserCopyError::ReadOnly);
    }
    Err(UserCopyError::InvalidRange)
}

fn physical_table(
    physical_offset: u64,
    physical_address: u64,
) -> Result<&'static PageTable, UserCopyError> {
    let mapped = physical_offset
        .checked_add(physical_address)
        .ok_or(UserCopyError::PhysicalMapUnavailable)?;
    let virtual_address =
        VirtAddr::try_new(mapped).map_err(|_| UserCopyError::PhysicalMapUnavailable)?;
    let physical_address =
        PhysAddr::try_new(physical_address).map_err(|_| UserCopyError::PhysicalMapUnavailable)?;
    if !physical_address.is_aligned(PAGE_SIZE) {
        return Err(UserCopyError::PhysicalMapUnavailable);
    }
    Ok(unsafe { &*virtual_address.as_ptr::<PageTable>() })
}
