use crate::RegisterIo;

pub const PCI_CAP_ID_VENDOR: u8 = 0x09;
pub const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER: u8 = 2;
pub const STATUS_DRIVER_OK: u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_DEVICE_NEEDS_RESET: u8 = 64;
pub const STATUS_FAILED: u8 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError<E = ()> {
    MalformedCapability,
    CapabilityLoop,
    CapabilityOutOfRange,
    MissingCommonConfig,
    UnsupportedFeature,
    FeatureRejected,
    InvalidQueue,
    ConfigurationUnstable,
    InvalidState,
    DeviceNeedsReset,
    Io(E),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciCapability {
    pub kind: u8,
    pub bar: u8,
    pub id: u8,
    pub offset: u32,
    pub length: u32,
    pub notify_multiplier: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySet {
    pub common: Option<PciCapability>,
    pub notify: Option<PciCapability>,
    pub isr: Option<PciCapability>,
    pub device: Option<PciCapability>,
    pub pci: Option<PciCapability>,
}

impl CapabilitySet {
    pub const fn empty() -> Self {
        Self {
            common: None,
            notify: None,
            isr: None,
            device: None,
            pci: None,
        }
    }

    pub fn parse(config: &[u8], first: u8) -> Result<Self, TransportError> {
        let mut result = Self::empty();
        let mut current = first as usize;
        let mut visited = [false; 256];
        let mut count = 0;
        while current != 0 {
            if current < 0x40 || current + 2 > config.len() || current > 255 {
                return Err(TransportError::CapabilityOutOfRange);
            }
            if visited[current] {
                return Err(TransportError::CapabilityLoop);
            }
            visited[current] = true;
            count += 1;
            if count > 48 {
                return Err(TransportError::CapabilityLoop);
            }
            let cap_id = config[current];
            let next = config[current + 1] as usize;
            if cap_id == PCI_CAP_ID_VENDOR {
                if current + 16 > config.len() {
                    return Err(TransportError::MalformedCapability);
                }
                let cap_len = config[current + 2] as usize;
                if cap_len < 16
                    || current.checked_add(cap_len).is_none()
                    || current + cap_len > config.len()
                {
                    return Err(TransportError::MalformedCapability);
                }
                let kind = config[current + 3];
                let bar = config[current + 4];
                if bar > 5 {
                    return Err(TransportError::MalformedCapability);
                }
                let id = config[current + 5];
                let offset = le32(config, current + 8);
                let length = le32(config, current + 12);
                if length == 0 || offset.checked_add(length).is_none() {
                    return Err(TransportError::MalformedCapability);
                }
                let multiplier = if kind == VIRTIO_PCI_CAP_NOTIFY_CFG {
                    if cap_len < 20 {
                        return Err(TransportError::MalformedCapability);
                    }
                    Some(le32(config, current + 16))
                } else {
                    None
                };
                let cap = PciCapability {
                    kind,
                    bar,
                    id,
                    offset,
                    length,
                    notify_multiplier: multiplier,
                };
                match kind {
                    VIRTIO_PCI_CAP_COMMON_CFG => result.common = Some(cap),
                    VIRTIO_PCI_CAP_NOTIFY_CFG => result.notify = Some(cap),
                    VIRTIO_PCI_CAP_ISR_CFG => result.isr = Some(cap),
                    VIRTIO_PCI_CAP_DEVICE_CFG => result.device = Some(cap),
                    VIRTIO_PCI_CAP_PCI_CFG => result.pci = Some(cap),
                    _ => {}
                }
            }
            current = next;
        }
        if result.common.is_none() {
            return Err(TransportError::MissingCommonConfig);
        }
        Ok(result)
    }
}

fn le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigWindow {
    pub base: u64,
    pub length: u32,
}

impl ConfigWindow {
    pub const fn new(base: u64, length: u32) -> Self {
        Self { base, length }
    }
    pub fn at(&self, offset: u32, width: u32) -> Result<u64, TransportError> {
        let end = offset
            .checked_add(width)
            .ok_or(TransportError::CapabilityOutOfRange)?;
        if end > self.length {
            return Err(TransportError::CapabilityOutOfRange);
        }
        self.base
            .checked_add(offset as u64)
            .ok_or(TransportError::CapabilityOutOfRange)
    }
}

pub mod common_offset {
    pub const DEVICE_FEATURE_SELECT: u32 = 0x00;
    pub const DEVICE_FEATURE: u32 = 0x04;
    pub const DRIVER_FEATURE_SELECT: u32 = 0x08;
    pub const DRIVER_FEATURE: u32 = 0x0c;
    pub const MSIX_CONFIG: u32 = 0x10;
    pub const NUM_QUEUES: u32 = 0x12;
    pub const DEVICE_STATUS: u32 = 0x14;
    pub const CONFIG_GENERATION: u32 = 0x15;
    pub const QUEUE_SELECT: u32 = 0x16;
    pub const QUEUE_SIZE: u32 = 0x18;
    pub const QUEUE_MSIX_VECTOR: u32 = 0x1a;
    pub const QUEUE_ENABLE: u32 = 0x1c;
    pub const QUEUE_NOTIFY_OFF: u32 = 0x1e;
    pub const QUEUE_DESC: u32 = 0x20;
    pub const QUEUE_DRIVER: u32 = 0x28;
    pub const QUEUE_DEVICE: u32 = 0x30;
}

pub const FEATURE_VERSION_1: u32 = 32;
pub const FEATURE_RING_PACKED: u32 = 34;
pub const FEATURE_IN_ORDER: u32 = 35;
pub const FEATURE_NOTIFICATION_DATA: u32 = 38;
pub const FEATURE_RING_RESET: u32 = 40;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Features(pub u128);
impl Features {
    pub const fn bit(bit: u32) -> Self {
        Self(1u128 << bit)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

pub struct ModernTransport<R> {
    io: R,
    common: ConfigWindow,
    status: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueConfig {
    pub index: u16,
    pub size: u16,
    pub descriptor: u64,
    pub driver: u64,
    pub device: u64,
    pub msix_vector: Option<u16>,
}

impl<R: RegisterIo> ModernTransport<R> {
    pub const fn new(io: R, common: ConfigWindow) -> Self {
        Self {
            io,
            common,
            status: 0,
        }
    }
    pub fn into_inner(self) -> R {
        self.io
    }

    pub fn reset(&mut self) -> Result<(), TransportError<R::Error>> {
        self.write_status(0)?;
        self.status = 0;
        Ok(())
    }

    pub fn begin(&mut self) -> Result<(), TransportError<R::Error>> {
        if self.status != 0 {
            return Err(TransportError::InvalidState);
        }
        self.write_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        self.status = STATUS_ACKNOWLEDGE | STATUS_DRIVER;
        Ok(())
    }

    pub fn negotiate(
        &mut self,
        supported: Features,
        required: Features,
    ) -> Result<Features, TransportError<R::Error>> {
        if self.status != STATUS_ACKNOWLEDGE | STATUS_DRIVER {
            return Err(TransportError::InvalidState);
        }
        let offered = self.read_features()?;
        if !offered.contains(required) {
            self.fail()?;
            return Err(TransportError::UnsupportedFeature);
        }
        let accepted = offered.intersection(supported);
        self.write_features(accepted)?;
        let next = self.status | STATUS_FEATURES_OK;
        self.write_status(next)?;
        let actual = self.read_status()?;
        if actual & STATUS_FEATURES_OK == 0 {
            self.fail()?;
            return Err(TransportError::FeatureRejected);
        }
        self.status = actual;
        Ok(accepted)
    }

    pub fn activate(&mut self) -> Result<(), TransportError<R::Error>> {
        if self.status & STATUS_FEATURES_OK == 0 {
            return Err(TransportError::InvalidState);
        }
        let next = self.status | STATUS_DRIVER_OK;
        self.write_status(next)?;
        self.status = next;
        Ok(())
    }

    pub fn configure_queue(
        &mut self,
        config: QueueConfig,
    ) -> Result<u16, TransportError<R::Error>> {
        if config.size == 0
            || !config.size.is_power_of_two()
            || config.descriptor & 15 != 0
            || config.driver & 1 != 0
            || config.device & 3 != 0
        {
            return Err(TransportError::InvalidQueue);
        }
        self.write16(common_offset::QUEUE_SELECT, config.index)?;
        let maximum = self.read16(common_offset::QUEUE_SIZE)?;
        if config.size > maximum || maximum == 0 {
            return Err(TransportError::InvalidQueue);
        }
        self.write16(common_offset::QUEUE_SIZE, config.size)?;
        if let Some(vector) = config.msix_vector {
            self.write16(common_offset::QUEUE_MSIX_VECTOR, vector)?;
        }
        self.write64(common_offset::QUEUE_DESC, config.descriptor)?;
        self.write64(common_offset::QUEUE_DRIVER, config.driver)?;
        self.write64(common_offset::QUEUE_DEVICE, config.device)?;
        self.write16(common_offset::QUEUE_ENABLE, 1)?;
        self.read16(common_offset::QUEUE_NOTIFY_OFF)
    }

    /// Reads a 32-bit device-specific field while guarding against concurrent
    /// configuration changes as required by VirtIO 1.x.
    pub fn read_device_u32_consistent(
        &mut self,
        device: ConfigWindow,
        offset: u32,
        attempts: u8,
    ) -> Result<u32, TransportError<R::Error>> {
        let address = device.at(offset, 4).map_err(map_unit)?;
        for _ in 0..attempts {
            let before = self.read8(common_offset::CONFIG_GENERATION)?;
            let value = self.io.read_u32(address).map_err(TransportError::Io)?;
            let after = self.read8(common_offset::CONFIG_GENERATION)?;
            if before == after {
                return Ok(value);
            }
        }
        Err(TransportError::ConfigurationUnstable)
    }

    pub fn check_health(&mut self) -> Result<(), TransportError<R::Error>> {
        let status = self.read_status()?;
        self.status = status;
        if status & STATUS_DEVICE_NEEDS_RESET != 0 {
            Err(TransportError::DeviceNeedsReset)
        } else {
            Ok(())
        }
    }

    fn read_features(&mut self) -> Result<Features, TransportError<R::Error>> {
        let mut bits = 0u128;
        for select in 0..4u32 {
            self.write32(common_offset::DEVICE_FEATURE_SELECT, select)?;
            bits |= (self.read32(common_offset::DEVICE_FEATURE)? as u128) << (select * 32);
        }
        Ok(Features(bits))
    }
    fn write_features(&mut self, features: Features) -> Result<(), TransportError<R::Error>> {
        for select in 0..4u32 {
            self.write32(common_offset::DRIVER_FEATURE_SELECT, select)?;
            self.write32(
                common_offset::DRIVER_FEATURE,
                (features.0 >> (select * 32)) as u32,
            )?;
        }
        Ok(())
    }
    fn read_status(&mut self) -> Result<u8, TransportError<R::Error>> {
        let at = self
            .common
            .at(common_offset::DEVICE_STATUS, 1)
            .map_err(map_unit)?;
        self.io.read_u8(at).map_err(TransportError::Io)
    }
    fn read8(&mut self, offset: u32) -> Result<u8, TransportError<R::Error>> {
        let at = self.common.at(offset, 1).map_err(map_unit)?;
        self.io.read_u8(at).map_err(TransportError::Io)
    }
    fn read16(&mut self, offset: u32) -> Result<u16, TransportError<R::Error>> {
        let at = self.common.at(offset, 2).map_err(map_unit)?;
        self.io.read_u16(at).map_err(TransportError::Io)
    }
    fn write16(&mut self, offset: u32, value: u16) -> Result<(), TransportError<R::Error>> {
        let at = self.common.at(offset, 2).map_err(map_unit)?;
        self.io.write_u16(at, value).map_err(TransportError::Io)
    }
    fn write64(&mut self, offset: u32, value: u64) -> Result<(), TransportError<R::Error>> {
        let at = self.common.at(offset, 8).map_err(map_unit)?;
        self.io.write_u64(at, value).map_err(TransportError::Io)
    }
    fn write_status(&mut self, value: u8) -> Result<(), TransportError<R::Error>> {
        let at = self
            .common
            .at(common_offset::DEVICE_STATUS, 1)
            .map_err(map_unit)?;
        self.io.write_u8(at, value).map_err(TransportError::Io)
    }
    fn read32(&mut self, offset: u32) -> Result<u32, TransportError<R::Error>> {
        let at = self.common.at(offset, 4).map_err(map_unit)?;
        self.io.read_u32(at).map_err(TransportError::Io)
    }
    fn write32(&mut self, offset: u32, value: u32) -> Result<(), TransportError<R::Error>> {
        let at = self.common.at(offset, 4).map_err(map_unit)?;
        self.io.write_u32(at, value).map_err(TransportError::Io)
    }
    fn fail(&mut self) -> Result<(), TransportError<R::Error>> {
        let status = self.status | STATUS_FAILED;
        self.write_status(status)?;
        self.status = status;
        Ok(())
    }
}

fn map_unit<E>(error: TransportError) -> TransportError<E> {
    match error {
        TransportError::CapabilityOutOfRange => TransportError::CapabilityOutOfRange,
        _ => TransportError::InvalidState,
    }
}

pub fn notify_address(
    window: ConfigWindow,
    multiplier: u32,
    queue_offset: u16,
) -> Result<u64, TransportError> {
    let offset = (queue_offset as u32)
        .checked_mul(multiplier)
        .ok_or(TransportError::CapabilityOutOfRange)?;
    window.at(offset, 2)
}
