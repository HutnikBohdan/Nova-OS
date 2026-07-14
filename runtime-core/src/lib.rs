#![no_std]

//! Core execution model for Nova OS.
//!
//! The crate intentionally has no allocator and no host operating-system
//! dependencies. The kernel supplies storage for the fixed-capacity tables and
//! performs the architecture-specific context switch described by the values
//! returned from this module.

#[cfg(test)]
extern crate std;

pub mod capability;
pub mod elf;
pub mod ipc;
pub mod process;
pub mod scheduler;
pub mod syscall;

pub use capability::{Capability, CapabilityError, CapabilityTable, ObjectId, Rights};
pub use elf::{ElfError, LoadPlan, LoadSegment, SegmentFlags};
pub use ipc::{Channel, IpcError, Message};
pub use process::{
    CpuContext, CrashReason, ExitCode, Process, ProcessError, ProcessId, ProcessState,
    ProcessTable, Thread, ThreadId, ThreadState, ThreadTable,
};
pub use scheduler::{
    BlockError, BlockedSet, DispatchError, Quantum, RoundRobin, SchedulerCore, SchedulerError,
    WaitReason,
};
