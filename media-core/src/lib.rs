#![no_std]
#![forbid(unsafe_code)]

//! Deterministic, allocation-free media primitives for Nova OS.
//!
//! Hardware access stays in the kernel drivers. This crate validates display
//! metadata and prepares bounded scanout/audio work for those drivers.

pub mod audio;
pub mod display;
pub mod edid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaError {
    InvalidValue,
    InvalidLength,
    InvalidChecksum,
    Unsupported,
    Capacity,
    Empty,
}

/// Stable Ukrainian messages suitable for the settings UI and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    DisplayReady,
    InvalidDisplayData,
    AudioReady,
    AudioUnderrun,
}

impl Diagnostic {
    pub const fn ukrainian(self) -> &'static str {
        match self {
            Self::DisplayReady => "Екран готовий",
            Self::InvalidDisplayData => "Некоректні дані дисплея",
            Self::AudioReady => "Звук готовий",
            Self::AudioUnderrun => "Переривання звуку: бракує даних",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_native_ukrainian() {
        assert_eq!(Diagnostic::DisplayReady.ukrainian(), "Екран готовий");
        assert!(Diagnostic::AudioUnderrun.ukrainian().contains("звуку"));
    }
}
