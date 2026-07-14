use crate::UsbError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route {
    raw: u32,
    depth: u8,
}
impl Route {
    pub const fn root() -> Self {
        Self { raw: 0, depth: 0 }
    }
    pub fn child(self, port: u8) -> Result<Self, UsbError> {
        if self.depth >= 5 || port == 0 || port > 15 {
            return Err(UsbError::Protocol);
        }
        Ok(Self {
            raw: self.raw | ((port as u32) << (self.depth * 4)),
            depth: self.depth + 1,
        })
    }
    pub const fn route_string(self) -> u32 {
        self.raw
    }
    pub const fn depth(self) -> u8 {
        self.depth
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortState {
    Disconnected,
    Debouncing,
    Resetting,
    Enabled,
    Suspended,
    Fault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HubPort {
    pub state: PortState,
    pub changed_at: u64,
    pub attempts: u8,
}
impl HubPort {
    pub const fn new() -> Self {
        Self {
            state: PortState::Disconnected,
            changed_at: 0,
            attempts: 0,
        }
    }
    pub fn connect(&mut self, now: u64) {
        self.state = PortState::Debouncing;
        self.changed_at = now;
        self.attempts = 0;
    }
    pub fn poll(&mut self, now: u64, connected: bool, enabled: bool) -> Result<bool, UsbError> {
        if !connected {
            self.state = PortState::Disconnected;
            return Ok(false);
        }
        match self.state {
            PortState::Debouncing if now.saturating_sub(self.changed_at) >= 100 => {
                self.state = PortState::Resetting;
                self.changed_at = now;
                self.attempts += 1;
                Ok(false)
            }
            PortState::Resetting if enabled => {
                self.state = PortState::Enabled;
                Ok(true)
            }
            PortState::Resetting if now.saturating_sub(self.changed_at) >= 100 => {
                if self.attempts >= 3 {
                    self.state = PortState::Fault;
                    Err(UsbError::Timeout)
                } else {
                    self.state = PortState::Debouncing;
                    self.changed_at = now;
                    Ok(false)
                }
            }
            PortState::Enabled => Ok(true),
            PortState::Fault => Err(UsbError::Timeout),
            _ => Ok(false),
        }
    }
}
impl Default for HubPort {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Hub<const N: usize> {
    ports: [HubPort; N],
}
impl<const N: usize> Hub<N> {
    pub const fn new() -> Self {
        Self {
            ports: [HubPort::new(); N],
        }
    }
    pub fn port_mut(&mut self, one_based: u8) -> Result<&mut HubPort, UsbError> {
        if one_based == 0 {
            return Err(UsbError::Protocol);
        }
        self.ports
            .get_mut(one_based as usize - 1)
            .ok_or(UsbError::Protocol)
    }
    pub const fn port_count(&self) -> usize {
        N
    }
}
impl<const N: usize> Default for Hub<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_depth_is_bounded() {
        let r = Route::root().child(2).unwrap().child(3).unwrap();
        assert_eq!(r.route_string(), 0x32);
        assert_eq!(r.depth(), 2);
    }
    #[test]
    fn port_debounce_reset() {
        let mut p = HubPort::new();
        p.connect(0);
        assert!(!p.poll(99, true, false).unwrap());
        assert!(!p.poll(100, true, false).unwrap());
        assert!(p.poll(101, true, true).unwrap());
    }
}
