#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptReason {
    None,
    Queue,
    Configuration,
    QueueAndConfiguration,
}
pub fn decode_isr(value: u8) -> InterruptReason {
    match value & 3 {
        1 => InterruptReason::Queue,
        2 => InterruptReason::Configuration,
        3 => InterruptReason::QueueAndConfiguration,
        _ => InterruptReason::None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Coalescer {
    packets: u16,
    threshold: u16,
    deadline: u64,
    delay: u64,
}
impl Coalescer {
    pub const fn new(threshold: u16, delay: u64) -> Self {
        Self {
            packets: 0,
            threshold,
            deadline: 0,
            delay,
        }
    }
    pub fn record(&mut self, now: u64) -> bool {
        if self.packets == 0 {
            self.deadline = now.saturating_add(self.delay);
        }
        self.packets = self.packets.saturating_add(1);
        self.threshold != 0 && self.packets >= self.threshold
    }
    pub fn due(&self, now: u64) -> bool {
        self.packets != 0 && now >= self.deadline
    }
    pub fn acknowledge(&mut self) -> u16 {
        let count = self.packets;
        self.packets = 0;
        self.deadline = 0;
        count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Reset,
    Initializing { deadline: u64 },
    Running,
    Quiescing { deadline: u64 },
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    InvalidTransition,
    TimedOut,
}
pub struct Lifecycle {
    state: DeviceState,
}
impl Lifecycle {
    pub const fn new() -> Self {
        Self {
            state: DeviceState::Reset,
        }
    }
    pub const fn state(&self) -> DeviceState {
        self.state
    }
    pub fn begin(&mut self, now: u64, timeout: u64) -> Result<(), LifecycleError> {
        if self.state != DeviceState::Reset {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = DeviceState::Initializing {
            deadline: now.saturating_add(timeout),
        };
        Ok(())
    }
    pub fn ready(&mut self) -> Result<(), LifecycleError> {
        if !matches!(self.state, DeviceState::Initializing { .. }) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = DeviceState::Running;
        Ok(())
    }
    pub fn quiesce(&mut self, now: u64, timeout: u64) -> Result<(), LifecycleError> {
        if self.state != DeviceState::Running {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = DeviceState::Quiescing {
            deadline: now.saturating_add(timeout),
        };
        Ok(())
    }
    pub fn reset_complete(&mut self) -> Result<(), LifecycleError> {
        if !matches!(
            self.state,
            DeviceState::Quiescing { .. } | DeviceState::Failed
        ) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = DeviceState::Reset;
        Ok(())
    }
    pub fn tick(&mut self, now: u64) -> Result<(), LifecycleError> {
        let deadline = match self.state {
            DeviceState::Initializing { deadline } | DeviceState::Quiescing { deadline } => {
                deadline
            }
            _ => return Ok(()),
        };
        if now >= deadline {
            self.state = DeviceState::Failed;
            Err(LifecycleError::TimedOut)
        } else {
            Ok(())
        }
    }
}
impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}
