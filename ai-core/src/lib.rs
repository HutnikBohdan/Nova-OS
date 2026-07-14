#![no_std]
#![forbid(unsafe_code)]

//! Local inference primitives for Nova OS.
//!
//! The crate deliberately has no allocator, operating system or floating-point
//! library dependency. Callers own all model bytes and scratch buffers.

pub mod agent;
pub mod gguf;
pub mod inference;
pub mod journal;
pub mod quant;
pub mod sampling;
pub mod tokenizer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnexpectedEof,
    InvalidMagic,
    UnsupportedVersion(u32),
    UnsupportedValueType(u32),
    InvalidBool(u8),
    InvalidUtf8,
    IntegerOverflow,
    InvalidTensor,
    OutputTooSmall,
    UnknownToken,
    InvalidConfiguration,
    InvalidTransition,
    PermissionDenied,
    StepLimit,
    CheckpointLimit,
    CallMismatch,
    MissingTensor,
    ShapeMismatch,
    ContextFull,
    Cancelled,
    WorkBudgetExceeded,
    JournalCapacity,
    CorruptJournal,
}

pub type Result<T> = core::result::Result<T, Error>;
