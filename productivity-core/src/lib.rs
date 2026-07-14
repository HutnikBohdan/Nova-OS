#![no_std]

//! Безалокаційні моделі основних програм Nova OS.
//!
//! Ядро не виконує системних викликів: застосунок передає введення, файлові
//! метадані та результати I/O, а ця бібліотека детерміновано керує станом.

#[cfg(test)]
extern crate std;

pub mod document;
pub mod files;
pub mod lifecycle;
pub mod settings;
pub mod terminal;

pub use document::{Document, DocumentError, SearchMatches, Selection};
pub use files::{ClipboardAction, FileEntry, FileKind, FileManager, SortField};
pub use lifecycle::{AppLifecycle, AppPhase, SessionCoordinator};
pub use settings::{SettingKey, SettingValue, Settings, SettingsTransaction};
pub use terminal::{Cell, Color, Terminal};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    ЗавеликийТекст,
}

impl<const N: usize> Text<N> {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, TextError> {
        if value.len() > N {
            return Err(TextError::ЗавеликийТекст);
        }
        let mut result = Self::empty();
        result.bytes[..value.len()].copy_from_slice(value.as_bytes());
        result.len = value.len();
        Ok(result)
    }

    pub fn set(&mut self, value: &str) -> Result<(), TextError> {
        *self = Self::new(value)?;
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        // Запис можливий лише з `str`, тому зайнята частина завжди UTF-8.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }

    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::empty()
    }
}
impl<const N: usize> core::fmt::Debug for Text<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}
