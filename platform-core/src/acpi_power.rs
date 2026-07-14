use crate::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSpace {
    SystemMemory,
    SystemIo,
    PciConfig,
    EmbeddedController,
    SmBus,
    Other(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericAddress {
    pub space: AddressSpace,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}
impl GenericAddress {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::Truncated);
        }
        let space = match bytes[0] {
            0 => AddressSpace::SystemMemory,
            1 => AddressSpace::SystemIo,
            2 => AddressSpace::PciConfig,
            3 => AddressSpace::EmbeddedController,
            4 => AddressSpace::SmBus,
            x => AddressSpace::Other(x),
        };
        let address = u64::from_le_bytes(bytes[4..12].try_into().map_err(|_| Error::Truncated)?);
        if address == 0 {
            return Err(Error::InvalidValue);
        }
        Ok(Self {
            space,
            bit_width: bytes[1],
            bit_offset: bytes[2],
            access_size: bytes[3],
            address,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FadtPower {
    pub pm1a_control: GenericAddress,
    pub pm1b_control: Option<GenericAddress>,
    pub sleep_control: Option<GenericAddress>,
    pub reset: Option<(GenericAddress, u8)>,
}
impl FadtPower {
    /// Parses a FADT body (the 36-byte common SDT header is excluded).
    pub fn parse_body(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 36 {
            return Err(Error::Truncated);
        }
        let legacy_a = u32le(b, 28)? as u64;
        let legacy_b = u32le(b, 32)? as u64;
        let extended_a = gas_at(b, 136).ok();
        let extended_b = gas_at(b, 148).ok();
        let pm1a_control = extended_a
            .or_else(|| legacy_gas(legacy_a))
            .ok_or(Error::InvalidValue)?;
        let pm1b_control = extended_b.or_else(|| legacy_gas(legacy_b));
        let reset = if b.len() >= 93 {
            gas_at(b, 80).ok().map(|g| (g, b[92]))
        } else {
            None
        };
        let sleep_control = gas_at(b, 208).ok();
        Ok(Self {
            pm1a_control,
            pm1b_control,
            sleep_control,
            reset,
        })
    }
    pub fn s5_plan(&self, s5: SleepTypes) -> Result<PowerPlan, Error> {
        if let Some(reg) = self.sleep_control {
            return Ok(PowerPlan {
                writes: [
                    Some(RegisterWrite {
                        register: reg,
                        value: ((s5.a as u64) << 2) | 0x20,
                    }),
                    None,
                ],
                len: 1,
            });
        }
        let a = RegisterWrite {
            register: self.pm1a_control,
            value: ((s5.a as u64) << 10) | (1 << 13),
        };
        let b = self.pm1b_control.map(|register| RegisterWrite {
            register,
            value: ((s5.b as u64) << 10) | (1 << 13),
        });
        Ok(PowerPlan {
            writes: [Some(a), b],
            len: if b.is_some() { 2 } else { 1 },
        })
    }
}
fn legacy_gas(address: u64) -> Option<GenericAddress> {
    if address == 0 {
        None
    } else {
        Some(GenericAddress {
            space: AddressSpace::SystemIo,
            bit_width: 16,
            bit_offset: 0,
            access_size: 2,
            address,
        })
    }
}
fn gas_at(b: &[u8], offset: usize) -> Result<GenericAddress, Error> {
    GenericAddress::parse(b.get(offset..offset + 12).ok_or(Error::Truncated)?)
}
fn u32le(b: &[u8], o: usize) -> Result<u32, Error> {
    let s = b.get(o..o + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SleepTypes {
    pub a: u8,
    pub b: u8,
}
/// Finds `Name (_S5, Package (...))` and decodes the first two AML integers.
pub fn parse_s5_aml(aml: &[u8]) -> Result<SleepTypes, Error> {
    let mut i = 0;
    while i + 6 <= aml.len() {
        let name = if aml[i] == 0x08 && aml.get(i + 1..i + 5) == Some(b"_S5_") {
            Some(i + 5)
        } else if aml.get(i..i + 4) == Some(b"_S5_") {
            Some(i + 4)
        } else {
            None
        };
        if let Some(mut p) = name {
            if aml.get(p) != Some(&0x12) {
                i += 1;
                continue;
            }
            p += 1;
            let (pkg_len, used) = aml_pkg_len(aml.get(p..).ok_or(Error::Truncated)?)?;
            p += used;
            if pkg_len < used + 1 || p >= aml.len() {
                return Err(Error::InvalidLength);
            }
            p += 1;
            let (a, n) = aml_int(aml.get(p..).ok_or(Error::Truncated)?)?;
            p += n;
            let (b, _) = aml_int(aml.get(p..).ok_or(Error::Truncated)?)?;
            if a > 7 || b > 7 {
                return Err(Error::InvalidValue);
            }
            return Ok(SleepTypes {
                a: a as u8,
                b: b as u8,
            });
        }
        i += 1
    }
    Err(Error::NotFound)
}
fn aml_pkg_len(b: &[u8]) -> Result<(usize, usize), Error> {
    let first = *b.first().ok_or(Error::Truncated)?;
    let follow = (first >> 6) as usize;
    if follow > 3 || b.len() < follow + 1 {
        return Err(Error::Truncated);
    }
    if follow == 0 {
        return Ok(((first & 0x3f) as usize, 1));
    }
    let mut len = (first & 0x0f) as usize;
    for n in 0..follow {
        len |= (b[n + 1] as usize) << (4 + n * 8)
    }
    Ok((len, follow + 1))
}
fn aml_int(b: &[u8]) -> Result<(u64, usize), Error> {
    match b.first().copied().ok_or(Error::Truncated)? {
        0 => Ok((0, 1)),
        1 => Ok((1, 1)),
        0x0a => Ok((*b.get(1).ok_or(Error::Truncated)? as u64, 2)),
        0x0b => {
            let s = b.get(1..3).ok_or(Error::Truncated)?;
            Ok((u16::from_le_bytes([s[0], s[1]]) as u64, 3))
        }
        _ => Err(Error::Unsupported),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterWrite {
    pub register: GenericAddress,
    pub value: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerPlan {
    writes: [Option<RegisterWrite>; 2],
    len: usize,
}
impl PowerPlan {
    pub fn iter(&self) -> impl Iterator<Item = &RegisterWrite> {
        self.writes[..self.len].iter().filter_map(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_legacy_fadt_and_builds_dual_plan() {
        let mut b = [0u8; 96];
        b[28..32].copy_from_slice(&0x404u32.to_le_bytes());
        b[32..36].copy_from_slice(&0x504u32.to_le_bytes());
        let f = FadtPower::parse_body(&b).unwrap();
        let p = f.s5_plan(SleepTypes { a: 5, b: 6 }).unwrap();
        assert_eq!(p.iter().count(), 2);
        assert_eq!(p.iter().next().unwrap().value, (5 << 10) | (1 << 13));
    }
    #[test]
    fn parses_s5_byte_constants() {
        let aml = [
            0x08, b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 5, 0x0a, 5,
        ];
        assert_eq!(parse_s5_aml(&aml), Ok(SleepTypes { a: 5, b: 5 }));
    }
    #[test]
    fn rejects_invalid_sleep_type() {
        let aml = [b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 9, 0x0a, 5];
        assert_eq!(parse_s5_aml(&aml), Err(Error::InvalidValue));
    }
    #[test]
    fn parses_gas() {
        let mut b = [0u8; 12];
        b[0] = 1;
        b[1] = 16;
        b[3] = 2;
        b[4..12].copy_from_slice(&0x404u64.to_le_bytes());
        assert_eq!(
            GenericAddress::parse(&b).unwrap().space,
            AddressSpace::SystemIo
        );
    }
}
