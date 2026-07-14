use crate::capability::CapabilityHandle;

/// Stable x86-64 syscall numbers. Arguments are passed in the native Nova ABI
/// registers; pointer-bearing requests use explicit C-layout structures.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallNumber {
    ProcessExit = 1,
    ThreadYield = 2,
    ObjectRead = 3,
    ObjectWrite = 4,
    MemoryMap = 5,
    CapabilityClone = 6,
    CapabilityClose = 7,
    ChannelSend = 8,
    ChannelReceive = 9,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserSlice {
    pub address: u64,
    pub length: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoRequest {
    pub capability: CapabilityHandle,
    pub offset: u64,
    pub buffer: UserSlice,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapRequest {
    pub capability: CapabilityHandle,
    pub object_offset: u64,
    pub length: u64,
    pub virtual_address: u64,
    pub protection: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyscallResult {
    pub value: u64,
    pub status: Status,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = -1,
    BadCapability = -2,
    AccessDenied = -3,
    NotFound = -4,
    WouldBlock = -5,
    NoMemory = -6,
    BufferTooSmall = -7,
}
