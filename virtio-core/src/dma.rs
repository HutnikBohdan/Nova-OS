#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaError {
    ZeroLength,
    InvalidAlignment,
    Overflow,
    OutOfSpace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaRegion {
    pub address: u64,
    pub length: usize,
    pub alignment: usize,
}

impl DmaRegion {
    pub fn new(address: u64, length: usize, alignment: usize) -> Result<Self, DmaError> {
        if length == 0 {
            return Err(DmaError::ZeroLength);
        }
        if alignment == 0 || !alignment.is_power_of_two() || address & (alignment as u64 - 1) != 0 {
            return Err(DmaError::InvalidAlignment);
        }
        address
            .checked_add(length as u64)
            .ok_or(DmaError::Overflow)?;
        Ok(Self {
            address,
            length,
            alignment,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SplitQueueLayout {
    pub descriptor: DmaRegion,
    pub available: DmaRegion,
    pub used: DmaRegion,
    pub total: usize,
}

pub fn plan_split(
    base: u64,
    bytes: usize,
    queue_size: u16,
    event_idx: bool,
) -> Result<SplitQueueLayout, DmaError> {
    if queue_size == 0 || !queue_size.is_power_of_two() {
        return Err(DmaError::InvalidAlignment);
    }
    let desc_len = queue_size as usize * 16;
    let avail_len = 6 + queue_size as usize * 2 + usize::from(event_idx) * 2;
    let used_len = 6 + queue_size as usize * 8 + usize::from(event_idx) * 2;
    let desc = align_up(base, 16)?;
    let avail = align_up(
        desc.checked_add(desc_len as u64)
            .ok_or(DmaError::Overflow)?,
        2,
    )?;
    let used = align_up(
        avail
            .checked_add(avail_len as u64)
            .ok_or(DmaError::Overflow)?,
        4,
    )?;
    let end = used
        .checked_add(used_len as u64)
        .ok_or(DmaError::Overflow)?;
    let total = end.checked_sub(base).ok_or(DmaError::Overflow)? as usize;
    if total > bytes {
        return Err(DmaError::OutOfSpace);
    }
    Ok(SplitQueueLayout {
        descriptor: DmaRegion::new(desc, desc_len, 16)?,
        available: DmaRegion::new(avail, avail_len, 2)?,
        used: DmaRegion::new(used, used_len, 4)?,
        total,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackedQueueLayout {
    pub descriptor: DmaRegion,
    pub driver_event: DmaRegion,
    pub device_event: DmaRegion,
    pub total: usize,
}

pub fn plan_packed(
    base: u64,
    bytes: usize,
    queue_size: u16,
) -> Result<PackedQueueLayout, DmaError> {
    if queue_size == 0 || !queue_size.is_power_of_two() {
        return Err(DmaError::InvalidAlignment);
    }
    let descriptor_len = queue_size as usize * 16;
    let descriptor = align_up(base, 16)?;
    let driver = align_up(
        descriptor
            .checked_add(descriptor_len as u64)
            .ok_or(DmaError::Overflow)?,
        4,
    )?;
    let device = driver.checked_add(4).ok_or(DmaError::Overflow)?;
    let end = device.checked_add(4).ok_or(DmaError::Overflow)?;
    let total = end.checked_sub(base).ok_or(DmaError::Overflow)? as usize;
    if total > bytes {
        return Err(DmaError::OutOfSpace);
    }
    Ok(PackedQueueLayout {
        descriptor: DmaRegion::new(descriptor, descriptor_len, 16)?,
        driver_event: DmaRegion::new(driver, 4, 4)?,
        device_event: DmaRegion::new(device, 4, 4)?,
        total,
    })
}

fn align_up(value: u64, alignment: u64) -> Result<u64, DmaError> {
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .ok_or(DmaError::Overflow)
}
