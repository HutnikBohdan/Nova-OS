pub const MAX_MESSAGE_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub sender: u64,
    pub tag: u32,
    length: u8,
    bytes: [u8; MAX_MESSAGE_BYTES],
}

impl Message {
    pub fn new(sender: u64, tag: u32, payload: &[u8]) -> Result<Self, IpcError> {
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(IpcError::MessageTooLarge);
        }
        let mut message = Self {
            sender,
            tag,
            length: payload.len() as u8,
            bytes: [0; MAX_MESSAGE_BYTES],
        };
        message.bytes[..payload.len()].copy_from_slice(payload);
        Ok(message)
    }

    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    Full,
    Empty,
    Closed,
    MessageTooLarge,
}

/// Bounded MPMC channel state. Kernel serialization makes operations atomic;
/// the structure itself contains no locks and never allocates.
pub struct Channel<const N: usize> {
    messages: [Option<Message>; N],
    head: usize,
    length: usize,
    send_closed: bool,
    receive_closed: bool,
}

impl<const N: usize> Channel<N> {
    pub const fn new() -> Self {
        Self {
            messages: [None; N],
            head: 0,
            length: 0,
            send_closed: false,
            receive_closed: false,
        }
    }
    pub const fn len(&self) -> usize {
        self.length
    }
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn send(&mut self, message: Message) -> Result<(), IpcError> {
        if self.send_closed || self.receive_closed {
            return Err(IpcError::Closed);
        }
        if N == 0 || self.length == N {
            return Err(IpcError::Full);
        }
        let tail = (self.head + self.length) % N;
        self.messages[tail] = Some(message);
        self.length += 1;
        Ok(())
    }

    pub fn receive(&mut self) -> Result<Message, IpcError> {
        if self.length == 0 {
            return if self.send_closed {
                Err(IpcError::Closed)
            } else {
                Err(IpcError::Empty)
            };
        }
        let message = self.messages[self.head].take().ok_or(IpcError::Empty)?;
        self.head = (self.head + 1) % N;
        self.length -= 1;
        Ok(message)
    }

    pub fn close_sender(&mut self) {
        self.send_closed = true;
    }
    pub fn close_receiver(&mut self) {
        self.receive_closed = true;
    }
}

impl<const N: usize> Default for Channel<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_preserves_fifo_and_backpressure() {
        let mut channel = Channel::<2>::new();
        channel
            .send(Message::new(1, 10, "один".as_bytes()).unwrap())
            .unwrap();
        channel
            .send(Message::new(2, 11, "два".as_bytes()).unwrap())
            .unwrap();
        assert_eq!(
            channel.send(Message::new(3, 12, "три".as_bytes()).unwrap()),
            Err(IpcError::Full)
        );
        assert_eq!(channel.receive().unwrap().payload(), "один".as_bytes());
        assert_eq!(channel.receive().unwrap().sender, 2);
    }

    #[test]
    fn closing_sender_drains_then_reports_closed() {
        let mut channel = Channel::<1>::new();
        channel.send(Message::new(1, 0, b"x").unwrap()).unwrap();
        channel.close_sender();
        assert_eq!(channel.receive().unwrap().payload(), b"x");
        assert_eq!(channel.receive(), Err(IpcError::Closed));
    }
}
