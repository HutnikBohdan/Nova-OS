pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const SYN_REPORT: u16 = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    Full,
    Empty,
}

pub struct EventBuffer<const N: usize> {
    events: [InputEvent; N],
    head: u16,
    tail: u16,
    dropped: u32,
}
impl<const N: usize> EventBuffer<N> {
    pub const fn new() -> Self {
        Self {
            events: [InputEvent {
                event_type: 0,
                code: 0,
                value: 0,
            }; N],
            head: 0,
            tail: 0,
            dropped: 0,
        }
    }
    pub fn push(&mut self, event: InputEvent) -> Result<(), InputError> {
        if N == 0 || self.tail.wrapping_sub(self.head) as usize >= N {
            self.dropped = self.dropped.saturating_add(1);
            return Err(InputError::Full);
        }
        self.events[self.tail as usize % N] = event;
        self.tail = self.tail.wrapping_add(1);
        Ok(())
    }
    pub fn pop(&mut self) -> Result<InputEvent, InputError> {
        if self.head == self.tail {
            return Err(InputError::Empty);
        }
        let event = self.events[self.head as usize % N];
        self.head = self.head.wrapping_add(1);
        Ok(event)
    }
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }
    pub fn frame_ready(&self) -> bool {
        let mut cursor = self.head;
        while cursor != self.tail {
            let e = self.events[cursor as usize % N];
            if e.event_type == EV_SYN && e.code == SYN_REPORT {
                return true;
            }
            cursor = cursor.wrapping_add(1);
        }
        false
    }
}
impl<const N: usize> Default for EventBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}
