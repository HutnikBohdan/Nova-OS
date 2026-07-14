use crate::ParseError;

pub const FIS_TYPE_REG_H2D: u8 = 0x27;
pub const ATA_READ_DMA_EXT: u8 = 0x25;
pub const ATA_WRITE_DMA_EXT: u8 = 0x35;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandHeader {
    pub flags_and_prdt_length: u32,
    pub transferred_bytes: u32,
    pub command_table_base: u64,
    pub reserved: [u32; 4],
}

impl CommandHeader {
    pub const fn new(
        fis_dwords: u8,
        write: bool,
        prdt_entries: u16,
        table: u64,
    ) -> Result<Self, ParseError> {
        if fis_dwords < 2 || fis_dwords > 16 || prdt_entries == 0 {
            return Err(ParseError::InvalidValue);
        }
        let flags = fis_dwords as u32 | ((write as u32) << 6) | ((prdt_entries as u32) << 16);
        Ok(Self {
            flags_and_prdt_length: flags,
            transferred_bytes: 0,
            command_table_base: table,
            reserved: [0; 4],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrdtEntry {
    pub data_base: u64,
    pub reserved: u32,
    pub byte_count_and_interrupt: u32,
}

impl PrdtEntry {
    pub const fn new(base: u64, bytes: u32, interrupt: bool) -> Result<Self, ParseError> {
        if bytes == 0 || bytes > 4 * 1024 * 1024 {
            return Err(ParseError::InvalidLength);
        }
        Ok(Self {
            data_base: base,
            reserved: 0,
            byte_count_and_interrupt: (bytes - 1) | ((interrupt as u32) << 31),
        })
    }
    pub const fn byte_count(self) -> u32 {
        (self.byte_count_and_interrupt & 0x3f_ffff) + 1
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisterHostToDeviceFis {
    pub fis_type: u8,
    pub flags: u8,
    pub command: u8,
    pub feature_low: u8,
    pub lba0: u8,
    pub lba1: u8,
    pub lba2: u8,
    pub device: u8,
    pub lba3: u8,
    pub lba4: u8,
    pub lba5: u8,
    pub feature_high: u8,
    pub count_low: u8,
    pub count_high: u8,
    pub iso_command_completion: u8,
    pub control: u8,
    pub reserved: [u8; 4],
}

impl RegisterHostToDeviceFis {
    pub const fn dma(command: u8, lba: u64, sectors: u16) -> Result<Self, ParseError> {
        if lba >= (1u64 << 48) || sectors == 0 {
            return Err(ParseError::InvalidValue);
        }
        Ok(Self {
            fis_type: FIS_TYPE_REG_H2D,
            flags: 1 << 7,
            command,
            feature_low: 0,
            lba0: lba as u8,
            lba1: (lba >> 8) as u8,
            lba2: (lba >> 16) as u8,
            device: 1 << 6,
            lba3: (lba >> 24) as u8,
            lba4: (lba >> 32) as u8,
            lba5: (lba >> 40) as u8,
            feature_high: 0,
            count_low: sectors as u8,
            count_high: (sectors >> 8) as u8,
            iso_command_completion: 0,
            control: 0,
            reserved: [0; 4],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortMachine {
    state: PortState,
    timeout_ticks: u16,
}

impl PortMachine {
    pub const fn new() -> Self {
        Self {
            state: PortState::Stopped,
            timeout_ticks: 0,
        }
    }
    pub const fn state(&self) -> PortState {
        self.state
    }
    pub fn request_start(&mut self, timeout_ticks: u16) -> Result<(), ParseError> {
        if self.state != PortState::Stopped || timeout_ticks == 0 {
            return Err(ParseError::InvalidValue);
        }
        self.state = PortState::Starting;
        self.timeout_ticks = timeout_ticks;
        Ok(())
    }
    pub fn observe(&mut self, command_running: bool, fis_running: bool, fatal: bool) {
        if fatal {
            self.state = PortState::Failed;
            return;
        }
        match self.state {
            PortState::Starting if command_running && fis_running => {
                self.state = PortState::Running
            }
            PortState::Stopping if !command_running && !fis_running => {
                self.state = PortState::Stopped
            }
            _ => {}
        }
    }
    pub fn request_stop(&mut self, timeout_ticks: u16) -> Result<(), ParseError> {
        if self.state != PortState::Running || timeout_ticks == 0 {
            return Err(ParseError::InvalidValue);
        }
        self.state = PortState::Stopping;
        self.timeout_ticks = timeout_ticks;
        Ok(())
    }
    pub fn tick(&mut self) {
        if matches!(self.state, PortState::Starting | PortState::Stopping) {
            self.timeout_ticks = self.timeout_ticks.saturating_sub(1);
            if self.timeout_ticks == 0 {
                self.state = PortState::Failed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_layouts() {
        assert_eq!(core::mem::size_of::<CommandHeader>(), 32);
        assert_eq!(core::mem::size_of::<PrdtEntry>(), 16);
        assert_eq!(core::mem::size_of::<RegisterHostToDeviceFis>(), 20);
    }
    #[test]
    fn fis_encodes_48bit_lba() {
        let f = RegisterHostToDeviceFis::dma(ATA_READ_DMA_EXT, 0x0102_0304_0506, 257).unwrap();
        assert_eq!(
            [f.lba5, f.lba4, f.lba3, f.lba2, f.lba1, f.lba0],
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!((f.count_high, f.count_low), (1, 1));
    }
    #[test]
    fn prdt_limits_and_decodes() {
        assert!(PrdtEntry::new(0, 0, false).is_err());
        assert_eq!(
            PrdtEntry::new(0x1000, 4096, true).unwrap().byte_count(),
            4096
        );
    }
    #[test]
    fn port_start_stop_machine() {
        let mut p = PortMachine::new();
        p.request_start(2).unwrap();
        p.observe(true, true, false);
        assert_eq!(p.state(), PortState::Running);
        p.request_stop(2).unwrap();
        p.observe(false, false, false);
        assert_eq!(p.state(), PortState::Stopped);
    }
}
