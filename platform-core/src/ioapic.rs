use crate::topology::ApicId;
use crate::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    Edge,
    Level,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryMode {
    Fixed,
    LowestPriority,
    Nmi,
    ExtInt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedirectionEntry {
    bits: u64,
}
impl RedirectionEntry {
    pub fn new(
        vector: u8,
        destination: ApicId,
        delivery: DeliveryMode,
        trigger: TriggerMode,
        polarity: Polarity,
        masked: bool,
    ) -> Result<Self, Error> {
        if vector < 32 {
            return Err(Error::InvalidValue);
        }
        let dest = destination.raw();
        if dest > 255 {
            return Err(Error::Unsupported);
        }
        let dm = match delivery {
            DeliveryMode::Fixed => 0,
            DeliveryMode::LowestPriority => 1,
            DeliveryMode::Nmi => 4,
            DeliveryMode::ExtInt => 7,
        };
        let mut bits = vector as u64 | ((dm as u64) << 8) | ((dest as u64) << 56);
        if trigger == TriggerMode::Level {
            bits |= 1 << 15;
        }
        if polarity == Polarity::ActiveLow {
            bits |= 1 << 13;
        }
        if masked {
            bits |= 1 << 16;
        }
        Ok(Self { bits })
    }
    pub const fn raw(self) -> u64 {
        self.bits
    }
    pub const fn low(self) -> u32 {
        self.bits as u32
    }
    pub const fn high(self) -> u32 {
        (self.bits >> 32) as u32
    }
    pub fn set_masked(&mut self, masked: bool) {
        if masked {
            self.bits |= 1 << 16
        } else {
            self.bits &= !(1 << 16)
        }
    }
    pub const fn is_masked(self) -> bool {
        self.bits & (1 << 16) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicWindow {
    pub gsi_base: u32,
    pub redirection_count: u8,
}
impl IoApicWindow {
    pub fn index_for(&self, gsi: u32) -> Result<u8, Error> {
        let i = gsi.checked_sub(self.gsi_base).ok_or(Error::NotFound)?;
        if i >= self.redirection_count as u32 {
            Err(Error::NotFound)
        } else {
            Ok(i as u8)
        }
    }
    pub const fn select_low(index: u8) -> u8 {
        0x10 + index * 2
    }
    pub const fn select_high(index: u8) -> u8 {
        0x11 + index * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_redirection() {
        let e = RedirectionEntry::new(
            48,
            ApicId::XApic(3),
            DeliveryMode::Fixed,
            TriggerMode::Level,
            Polarity::ActiveLow,
            false,
        )
        .unwrap();
        assert_eq!(e.low(), 48 | (1 << 15) | (1 << 13));
        assert_eq!(e.high(), 3 << 24);
    }
    #[test]
    fn mask_roundtrip() {
        let mut e = RedirectionEntry::new(
            40,
            ApicId::XApic(0),
            DeliveryMode::Fixed,
            TriggerMode::Edge,
            Polarity::ActiveHigh,
            false,
        )
        .unwrap();
        e.set_masked(true);
        assert!(e.is_masked());
        e.set_masked(false);
        assert!(!e.is_masked());
    }
    #[test]
    fn rejects_x2_destination_in_physical_ioapic() {
        assert_eq!(
            RedirectionEntry::new(
                40,
                ApicId::X2Apic(256),
                DeliveryMode::Fixed,
                TriggerMode::Edge,
                Polarity::ActiveHigh,
                false
            ),
            Err(Error::Unsupported)
        );
    }
    #[test]
    fn maps_gsi_window() {
        let w = IoApicWindow {
            gsi_base: 24,
            redirection_count: 8,
        };
        assert_eq!(w.index_for(29), Ok(5));
        assert_eq!(w.index_for(3), Err(Error::NotFound));
        assert_eq!(IoApicWindow::select_high(5), 0x1b);
    }
}
