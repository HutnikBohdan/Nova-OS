use crate::{Deadline, UsbError};

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetupPacket {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl SetupPacket {
    pub const fn get_descriptor(kind: u8, index: u8, language: u16, length: u16) -> Self {
        Self {
            request_type: 0x80,
            request: 6,
            value: ((kind as u16) << 8) | index as u16,
            index: language,
            length,
        }
    }
    pub const fn set_configuration(value: u8) -> Self {
        Self {
            request_type: 0,
            request: 9,
            value: value as u16,
            index: 0,
            length: 0,
        }
    }
    pub const fn direction_in(self) -> bool {
        self.request_type & 0x80 != 0
    }
    pub const fn bytes(self) -> [u8; 8] {
        let v = self.value.to_le_bytes();
        let i = self.index.to_le_bytes();
        let l = self.length.to_le_bytes();
        [
            self.request_type,
            self.request,
            v[0],
            v[1],
            i[0],
            i[1],
            l[0],
            l[1],
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlState {
    Idle,
    Setup,
    Data,
    Status,
    Complete,
    Failed(UsbError),
}

pub struct ControlTransfer {
    pub setup: SetupPacket,
    pub state: ControlState,
    deadline: Deadline,
    transferred: u16,
}
impl ControlTransfer {
    pub const fn begin(setup: SetupPacket, now: u64, timeout: u64) -> Self {
        Self {
            setup,
            state: ControlState::Setup,
            deadline: Deadline::after(now, timeout),
            transferred: 0,
        }
    }
    pub fn setup_complete(&mut self) -> Result<(), UsbError> {
        if self.state != ControlState::Setup {
            return Err(UsbError::InvalidState);
        }
        self.state = if self.setup.length == 0 {
            ControlState::Status
        } else {
            ControlState::Data
        };
        Ok(())
    }
    pub fn data_complete(&mut self, bytes: u16) -> Result<(), UsbError> {
        if self.state != ControlState::Data || bytes > self.setup.length {
            return Err(UsbError::Protocol);
        }
        self.transferred = bytes;
        self.state = ControlState::Status;
        Ok(())
    }
    pub fn status_complete(&mut self) -> Result<u16, UsbError> {
        if self.state != ControlState::Status {
            return Err(UsbError::InvalidState);
        }
        self.state = ControlState::Complete;
        Ok(self.transferred)
    }
    pub fn poll_timeout(&mut self, now: u64) -> Result<(), UsbError> {
        if !matches!(self.state, ControlState::Complete | ControlState::Failed(_))
            && self.deadline.expired(now)
        {
            self.state = ControlState::Failed(UsbError::Timeout);
            Err(UsbError::Timeout)
        } else {
            Ok(())
        }
    }
    pub fn fail(&mut self, error: UsbError) {
        self.state = ControlState::Failed(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_data_flow() {
        let mut t = ControlTransfer::begin(SetupPacket::set_configuration(1), 0, 5);
        t.setup_complete().unwrap();
        assert_eq!(t.status_complete(), Ok(0));
    }
    #[test]
    fn timeout_bounded() {
        let mut t = ControlTransfer::begin(SetupPacket::get_descriptor(1, 0, 0, 18), 10, 3);
        assert_eq!(t.poll_timeout(13), Err(UsbError::Timeout));
    }
    #[test]
    fn setup_encoding() {
        assert_eq!(
            SetupPacket::set_configuration(2).bytes(),
            [0, 9, 2, 0, 0, 0, 0, 0]
        );
    }
}
