use crate::{Deadline, UsbError};

pub const CBW_SIGNATURE: u32 = 0x4342_5355;
pub const CSW_SIGNATURE: u32 = 0x5342_5355;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandBlockWrapper {
    pub signature: u32,
    pub tag: u32,
    pub transfer_length: u32,
    pub flags: u8,
    pub lun: u8,
    pub command_length: u8,
    pub command: [u8; 16],
}
impl CommandBlockWrapper {
    pub fn new(
        tag: u32,
        transfer_length: u32,
        data_in: bool,
        lun: u8,
        cdb: &[u8],
    ) -> Result<Self, UsbError> {
        if cdb.is_empty() || cdb.len() > 16 || lun > 15 {
            return Err(UsbError::Protocol);
        }
        let mut command = [0; 16];
        command[..cdb.len()].copy_from_slice(cdb);
        Ok(Self {
            signature: CBW_SIGNATURE,
            tag,
            transfer_length,
            flags: if data_in { 0x80 } else { 0 },
            lun,
            command_length: cdb.len() as u8,
            command,
        })
    }
    pub const fn inquiry(tag: u32, allocation: u8) -> Self {
        let mut c = [0; 16];
        c[0] = 0x12;
        c[4] = allocation;
        Self {
            signature: CBW_SIGNATURE,
            tag,
            transfer_length: allocation as u32,
            flags: 0x80,
            lun: 0,
            command_length: 6,
            command: c,
        }
    }
    pub const fn read10(tag: u32, lba: u32, blocks: u16, block_size: u32) -> Self {
        let mut c = [0; 16];
        c[0] = 0x28;
        let l = lba.to_be_bytes();
        c[2] = l[0];
        c[3] = l[1];
        c[4] = l[2];
        c[5] = l[3];
        let b = blocks.to_be_bytes();
        c[7] = b[0];
        c[8] = b[1];
        Self {
            signature: CBW_SIGNATURE,
            tag,
            transfer_length: (blocks as u32).saturating_mul(block_size),
            flags: 0x80,
            lun: 0,
            command_length: 10,
            command: c,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandStatusWrapper {
    pub signature: u32,
    pub tag: u32,
    pub residue: u32,
    pub status: u8,
}
impl CommandStatusWrapper {
    pub fn parse(b: &[u8]) -> Result<Self, UsbError> {
        if b.len() < 13 {
            return Err(UsbError::BufferTooSmall);
        }
        let s = Self {
            signature: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            tag: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            residue: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            status: b[12],
        };
        if s.signature != CSW_SIGNATURE || s.status > 2 {
            return Err(UsbError::Protocol);
        }
        Ok(s)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BotState {
    Idle,
    Command,
    DataIn,
    DataOut,
    Status,
    Complete,
    NeedsReset,
    Failed(UsbError),
}
pub struct BotTransfer {
    pub cbw: CommandBlockWrapper,
    pub state: BotState,
    deadline: Deadline,
}
impl BotTransfer {
    pub const fn begin(cbw: CommandBlockWrapper, now: u64, timeout: u64) -> Self {
        Self {
            cbw,
            state: BotState::Command,
            deadline: Deadline::after(now, timeout),
        }
    }
    pub fn command_sent(&mut self) -> Result<(), UsbError> {
        if self.state != BotState::Command {
            return Err(UsbError::InvalidState);
        }
        self.state = if self.cbw.transfer_length == 0 {
            BotState::Status
        } else if self.cbw.flags & 0x80 != 0 {
            BotState::DataIn
        } else {
            BotState::DataOut
        };
        Ok(())
    }
    pub fn data_complete(&mut self, bytes: u32) -> Result<(), UsbError> {
        if !matches!(self.state, BotState::DataIn | BotState::DataOut)
            || bytes > self.cbw.transfer_length
        {
            return Err(UsbError::Protocol);
        }
        self.state = BotState::Status;
        Ok(())
    }
    pub fn status(&mut self, csw: CommandStatusWrapper) -> Result<(), UsbError> {
        if self.state != BotState::Status || csw.tag != self.cbw.tag {
            return Err(UsbError::Protocol);
        }
        match csw.status {
            0 => {
                self.state = BotState::Complete;
                Ok(())
            }
            1 => {
                self.state = BotState::Failed(UsbError::Transaction);
                Err(UsbError::Transaction)
            }
            _ => {
                self.state = BotState::NeedsReset;
                Err(UsbError::Protocol)
            }
        }
    }
    pub fn poll_timeout(&mut self, now: u64) -> Result<(), UsbError> {
        if !matches!(self.state, BotState::Complete | BotState::Failed(_))
            && self.deadline.expired(now)
        {
            self.state = BotState::NeedsReset;
            Err(UsbError::Timeout)
        } else {
            Ok(())
        }
    }
    pub fn reset_complete(&mut self) {
        self.state = BotState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read10_encodes_big_endian() {
        let c = CommandBlockWrapper::read10(7, 0x12345678, 2, 512);
        assert_eq!(&c.command[2..6], &[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(c.transfer_length, 1024);
    }
    #[test]
    fn happy_path() {
        let mut t = BotTransfer::begin(CommandBlockWrapper::inquiry(9, 36), 0, 10);
        t.command_sent().unwrap();
        t.data_complete(36).unwrap();
        let csw = CommandStatusWrapper {
            signature: CSW_SIGNATURE,
            tag: 9,
            residue: 0,
            status: 0,
        };
        t.status(csw).unwrap();
        assert_eq!(t.state, BotState::Complete);
    }
    #[test]
    fn phase_error_needs_reset() {
        let mut t = BotTransfer::begin(CommandBlockWrapper::inquiry(9, 1), 0, 10);
        t.command_sent().unwrap();
        t.data_complete(1).unwrap();
        assert_eq!(
            t.status(CommandStatusWrapper {
                signature: CSW_SIGNATURE,
                tag: 9,
                residue: 0,
                status: 2
            }),
            Err(UsbError::Protocol)
        );
        assert_eq!(t.state, BotState::NeedsReset);
    }
}
