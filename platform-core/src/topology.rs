use crate::{BoundedVec, Error};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApicId {
    XApic(u8),
    X2Apic(u32),
}

impl ApicId {
    pub const fn raw(self) -> u32 {
        match self {
            Self::XApic(v) => v as u32,
            Self::X2Apic(v) => v,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Processor {
    pub firmware_id: u32,
    pub apic_id: ApicId,
    pub enabled: bool,
    pub online_capable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApic {
    pub id: u8,
    pub address: u32,
    pub gsi_base: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptOverride {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topology<const C: usize, const I: usize, const O: usize> {
    pub local_apic_address: u64,
    pub processors: BoundedVec<Processor, C>,
    pub io_apics: BoundedVec<IoApic, I>,
    pub overrides: BoundedVec<InterruptOverride, O>,
}

impl<const C: usize, const I: usize, const O: usize> Topology<C, I, O> {
    pub fn parse_madt(body: &[u8]) -> Result<Self, Error> {
        if body.len() < 8 {
            return Err(Error::Truncated);
        }
        let mut out = Self {
            local_apic_address: u32le(body, 0)? as u64,
            processors: BoundedVec::new(),
            io_apics: BoundedVec::new(),
            overrides: BoundedVec::new(),
        };
        let mut p = 8;
        while p < body.len() {
            if p + 2 > body.len() {
                return Err(Error::Truncated);
            }
            let kind = body[p];
            let len = body[p + 1] as usize;
            if len < 2 || p + len > body.len() {
                return Err(Error::InvalidLength);
            }
            let e = &body[p..p + len];
            match kind {
                0 if len >= 8 => out.add_processor(Processor {
                    firmware_id: e[2] as u32,
                    apic_id: ApicId::XApic(e[3]),
                    enabled: u32le(e, 4)? & 1 != 0,
                    online_capable: u32le(e, 4)? & 2 != 0,
                })?,
                1 if len >= 12 => out.io_apics.push(IoApic {
                    id: e[2],
                    address: u32le(e, 4)?,
                    gsi_base: u32le(e, 8)?,
                })?,
                2 if len >= 10 => out.overrides.push(InterruptOverride {
                    bus: e[2],
                    source: e[3],
                    gsi: u32le(e, 4)?,
                    flags: u16le(e, 8)?,
                })?,
                5 if len >= 12 => out.local_apic_address = u64le(e, 4)?,
                9 if len >= 16 => out.add_processor(Processor {
                    firmware_id: u32le(e, 12)?,
                    apic_id: ApicId::X2Apic(u32le(e, 4)?),
                    enabled: u32le(e, 8)? & 1 != 0,
                    online_capable: u32le(e, 8)? & 2 != 0,
                })?,
                _ => {}
            }
            p += len;
        }
        Ok(out)
    }
    fn add_processor(&mut self, cpu: Processor) -> Result<(), Error> {
        if self
            .processors
            .iter()
            .any(|x| x.apic_id.raw() == cpu.apic_id.raw())
        {
            return Err(Error::Duplicate);
        }
        self.processors.push(cpu)
    }
    pub fn enabled_processors(&self) -> impl Iterator<Item = &Processor> {
        self.processors.iter().filter(|p| p.enabled)
    }
}

fn u16le(b: &[u8], o: usize) -> Result<u16, Error> {
    let s = b.get(o..o + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(b: &[u8], o: usize) -> Result<u32, Error> {
    let s = b.get(o..o + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64le(b: &[u8], o: usize) -> Result<u64, Error> {
    let s = b.get(o..o + 8).ok_or(Error::Truncated)?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_mixed_apics_and_override() {
        let mut b = [0u8; 8 + 8 + 16 + 12 + 10 + 12];
        b[0..4].copy_from_slice(&0xfee00000u32.to_le_bytes());
        let mut p = 8;
        b[p..p + 8].copy_from_slice(&[0, 8, 0, 2, 1, 0, 0, 0]);
        p += 8;
        b[p] = 9;
        b[p + 1] = 16;
        b[p + 4..p + 8].copy_from_slice(&0x123u32.to_le_bytes());
        b[p + 8..p + 12].copy_from_slice(&1u32.to_le_bytes());
        b[p + 12..p + 16].copy_from_slice(&7u32.to_le_bytes());
        p += 16;
        b[p..p + 12].copy_from_slice(&[1, 12, 3, 0, 0, 0, 0xC0, 0xFE, 0, 0, 0, 0]);
        p += 12;
        b[p..p + 10].copy_from_slice(&[2, 10, 0, 1, 9, 0, 0, 0, 0x0f, 0]);
        p += 10;
        b[p] = 5;
        b[p + 1] = 12;
        b[p + 4..p + 12].copy_from_slice(&0xfee01000u64.to_le_bytes());
        let t = Topology::<8, 2, 4>::parse_madt(&b).unwrap();
        assert_eq!(t.local_apic_address, 0xfee01000);
        assert_eq!(t.processors.len(), 2);
        assert_eq!(t.io_apics.len(), 1);
        assert_eq!(t.overrides.get(0).unwrap().gsi, 9);
    }
    #[test]
    fn rejects_duplicate_apic_ids() {
        let b = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 1, 1, 0, 0, 0, 9, 16, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2,
            0, 0, 0,
        ];
        assert_eq!(Topology::<4, 1, 1>::parse_madt(&b), Err(Error::Duplicate));
    }
    #[test]
    fn rejects_bad_entry_length() {
        assert_eq!(
            Topology::<1, 1, 1>::parse_madt(&[0; 9]),
            Err(Error::Truncated)
        );
    }
}
