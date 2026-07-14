use crate::UsbError;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrbType {
    Normal = 1,
    SetupStage = 2,
    DataStage = 3,
    StatusStage = 4,
    Link = 6,
    EnableSlotCommand = 9,
    AddressDeviceCommand = 11,
    ConfigureEndpointCommand = 12,
    TransferEvent = 32,
    CommandCompletionEvent = 33,
    PortStatusChangeEvent = 34,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Trb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    const CYCLE: u32 = 1;
    pub const fn new(kind: TrbType, parameter: u64, status: u32, flags: u16) -> Self {
        Self {
            parameter,
            status,
            control: ((kind as u32) << 10) | ((flags as u32) << 16),
        }
    }
    pub const fn kind(self) -> u8 {
        ((self.control >> 10) & 0x3f) as u8
    }
    pub const fn cycle(self) -> bool {
        self.control & Self::CYCLE != 0
    }
    pub const fn with_cycle(mut self, cycle: bool) -> Self {
        self.control = (self.control & !Self::CYCLE) | cycle as u32;
        self
    }
    pub const fn completion_code(self) -> u8 {
        (self.status >> 24) as u8
    }
    pub const fn slot_id(self) -> u8 {
        (self.control >> 24) as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RingTicket {
    pub index: u16,
    pub cycle: bool,
}

/// Software image of a producer ring. A platform layer may copy submitted TRBs
/// into DMA-coherent storage and ring the doorbell.
pub struct ProducerRing<const N: usize> {
    entries: [Trb; N],
    producer: usize,
    consumer: usize,
    count: usize,
    cycle: bool,
}

impl<const N: usize> ProducerRing<N> {
    pub const fn new() -> Self {
        Self {
            entries: [Trb {
                parameter: 0,
                status: 0,
                control: 0,
            }; N],
            producer: 0,
            consumer: 0,
            count: 0,
            cycle: true,
        }
    }
    pub fn submit(&mut self, trb: Trb) -> Result<RingTicket, UsbError> {
        if N == 0 || self.count == N {
            return Err(UsbError::RingFull);
        }
        let ticket = RingTicket {
            index: self.producer as u16,
            cycle: self.cycle,
        };
        self.entries[self.producer] = trb.with_cycle(self.cycle);
        self.producer += 1;
        if self.producer == N {
            self.producer = 0;
            self.cycle = !self.cycle;
        }
        self.count += 1;
        Ok(ticket)
    }
    pub fn complete_one(&mut self) -> Result<(), UsbError> {
        if self.count == 0 {
            return Err(UsbError::RingEmpty);
        }
        self.consumer = (self.consumer + 1) % N;
        self.count -= 1;
        Ok(())
    }
    pub const fn get(&self, index: usize) -> Option<&Trb> {
        if index < N {
            Some(&self.entries[index])
        } else {
            None
        }
    }
    pub const fn len(&self) -> usize {
        self.count
    }
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl<const N: usize> Default for ProducerRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EventRing<const N: usize> {
    entries: [Trb; N],
    dequeue: usize,
    cycle: bool,
}

impl<const N: usize> EventRing<N> {
    pub const fn new() -> Self {
        Self {
            entries: [Trb {
                parameter: 0,
                status: 0,
                control: 0,
            }; N],
            dequeue: 0,
            cycle: true,
        }
    }
    pub fn inject_for_driver(
        &mut self,
        index: usize,
        trb: Trb,
        cycle: bool,
    ) -> Result<(), UsbError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(UsbError::BufferTooSmall)?;
        *entry = trb.with_cycle(cycle);
        Ok(())
    }
    pub fn pop(&mut self) -> Result<Trb, UsbError> {
        if N == 0 || self.entries[self.dequeue].cycle() != self.cycle {
            return Err(UsbError::RingEmpty);
        }
        let event = self.entries[self.dequeue];
        self.dequeue += 1;
        if self.dequeue == N {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
        Ok(event)
    }
    pub const fn dequeue_index(&self) -> usize {
        self.dequeue
    }
}

impl<const N: usize> Default for EventRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotState {
    Disabled,
    Enabled,
    Addressed,
    Configured,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointState {
    Disabled,
    Running,
    Halted,
    Stopped,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub id: u8,
    pub state: EndpointState,
}

impl Endpoint {
    pub const fn disabled(id: u8) -> Self {
        Self {
            id,
            state: EndpointState::Disabled,
        }
    }

    pub fn start(&mut self) -> Result<(), UsbError> {
        match self.state {
            EndpointState::Disabled | EndpointState::Stopped => {
                self.state = EndpointState::Running;
                Ok(())
            }
            _ => Err(UsbError::InvalidState),
        }
    }

    pub fn halt(&mut self) -> Result<(), UsbError> {
        if self.state != EndpointState::Running {
            return Err(UsbError::InvalidState);
        }
        self.state = EndpointState::Halted;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), UsbError> {
        if !matches!(self.state, EndpointState::Halted | EndpointState::Error) {
            return Err(UsbError::InvalidState);
        }
        self.state = EndpointState::Stopped;
        Ok(())
    }

    pub fn fail(&mut self) {
        self.state = EndpointState::Error;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceSlot {
    pub id: u8,
    pub port: u8,
    pub state: SlotState,
}

impl DeviceSlot {
    pub const fn disabled() -> Self {
        Self {
            id: 0,
            port: 0,
            state: SlotState::Disabled,
        }
    }
    pub fn enabled(&mut self, id: u8, port: u8) -> Result<(), UsbError> {
        if self.state != SlotState::Disabled || id == 0 || port == 0 {
            return Err(UsbError::InvalidState);
        }
        self.id = id;
        self.port = port;
        self.state = SlotState::Enabled;
        Ok(())
    }
    pub fn addressed(&mut self) -> Result<(), UsbError> {
        self.transition(SlotState::Enabled, SlotState::Addressed)
    }
    pub fn configured(&mut self) -> Result<(), UsbError> {
        self.transition(SlotState::Addressed, SlotState::Configured)
    }
    fn transition(&mut self, from: SlotState, to: SlotState) -> Result<(), UsbError> {
        if self.state != from {
            return Err(UsbError::InvalidState);
        }
        self.state = to;
        Ok(())
    }
    pub fn disconnect(&mut self) {
        *self = Self::disabled();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_cycle_and_capacity() {
        let mut r = ProducerRing::<2>::new();
        assert!(r.submit(Trb::new(TrbType::Normal, 1, 2, 0)).unwrap().cycle);
        r.submit(Trb::new(TrbType::Normal, 2, 3, 0)).unwrap();
        assert_eq!(r.submit(Trb::default()), Err(UsbError::RingFull));
        r.complete_one().unwrap();
        assert!(!r.submit(Trb::default()).unwrap().cycle);
    }
    #[test]
    fn event_cycle() {
        let mut r = EventRing::<1>::new();
        let e = Trb::new(TrbType::TransferEvent, 7, 1 << 24, 0);
        r.inject_for_driver(0, e, true).unwrap();
        assert_eq!(r.pop().unwrap().kind(), TrbType::TransferEvent as u8);
        assert_eq!(r.pop(), Err(UsbError::RingEmpty));
    }
    #[test]
    fn slot_transitions_are_strict() {
        let mut s = DeviceSlot::disabled();
        assert_eq!(s.addressed(), Err(UsbError::InvalidState));
        s.enabled(2, 1).unwrap();
        s.addressed().unwrap();
        s.configured().unwrap();
        assert_eq!(s.state, SlotState::Configured);
    }

    #[test]
    fn endpoint_halt_reset_restart() {
        let mut endpoint = Endpoint::disabled(3);
        endpoint.start().unwrap();
        endpoint.halt().unwrap();
        endpoint.reset().unwrap();
        endpoint.start().unwrap();
        assert_eq!(endpoint.state, EndpointState::Running);
    }
}
