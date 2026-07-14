use core::cell::UnsafeCell;
use runtime_core::{Channel, IpcError, Message};

struct KernelChannel(UnsafeCell<Channel<8>>);

// The int 0x80 interrupt gate clears IF while the channel is accessed. Nova is
// currently single-core, so no two kernel contexts can mutate this queue.
unsafe impl Sync for KernelChannel {}

static CHANNEL: KernelChannel = KernelChannel(UnsafeCell::new(Channel::new()));

pub fn send(sender: u64, tag: u32, payload: &[u8]) -> Result<(), IpcError> {
    let message = Message::new(sender, tag, payload)?;
    unsafe { (&mut *CHANNEL.0.get()).send(message) }
}

pub fn receive(output: &mut [u8]) -> Result<usize, IpcError> {
    let message = unsafe { (&mut *CHANNEL.0.get()).receive()? };
    if output.len() < message.payload().len() {
        return Err(IpcError::MessageTooLarge);
    }
    let length = message.payload().len();
    output[..length].copy_from_slice(message.payload());
    Ok(length)
}
