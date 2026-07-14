use crate::ParseError;

fn read_u16(b: &[u8], o: usize) -> Result<u16, ParseError> {
    let s = b.get(o..o + 2).ok_or(ParseError::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn read_u32(b: &[u8], o: usize) -> Result<u32, ParseError> {
    let s = b.get(o..o + 4).ok_or(ParseError::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn read_u64(b: &[u8], o: usize) -> Result<u64, ParseError> {
    let s = b.get(o..o + 8).ok_or(ParseError::Truncated)?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}
fn checksum_ok(b: &[u8]) -> bool {
    b.iter().fold(0u8, |a, x| a.wrapping_add(*x)) == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rsdp {
    pub revision: u8,
    pub rsdt_address: u32,
    pub xsdt_address: Option<u64>,
    pub oem_id: [u8; 6],
}

impl Rsdp {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < 20 {
            return Err(ParseError::Truncated);
        }
        if &bytes[..8] != b"RSD PTR " {
            return Err(ParseError::InvalidSignature);
        }
        if !checksum_ok(&bytes[..20]) {
            return Err(ParseError::InvalidChecksum);
        }
        let mut oem_id = [0; 6];
        oem_id.copy_from_slice(&bytes[9..15]);
        let revision = bytes[15];
        let rsdt_address = read_u32(bytes, 16)?;
        let xsdt_address = if revision >= 2 {
            if bytes.len() < 36 {
                return Err(ParseError::Truncated);
            }
            let length = read_u32(bytes, 20)? as usize;
            if length < 36 || length > bytes.len() {
                return Err(ParseError::InvalidLength);
            }
            if !checksum_ok(&bytes[..length]) {
                return Err(ParseError::InvalidChecksum);
            }
            Some(read_u64(bytes, 24)?)
        } else {
            None
        };
        Ok(Self {
            revision,
            rsdt_address,
            xsdt_address,
            oem_id,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sdt<'a> {
    pub signature: [u8; 4],
    pub revision: u8,
    bytes: &'a [u8],
}

impl<'a> Sdt<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < 36 {
            return Err(ParseError::Truncated);
        }
        let length = read_u32(bytes, 4)? as usize;
        if length < 36 || length > bytes.len() {
            return Err(ParseError::InvalidLength);
        }
        let bytes = &bytes[..length];
        if !checksum_ok(bytes) {
            return Err(ParseError::InvalidChecksum);
        }
        let mut signature = [0; 4];
        signature.copy_from_slice(&bytes[..4]);
        Ok(Self {
            signature,
            revision: bytes[8],
            bytes,
        })
    }
    pub fn body(&self) -> &'a [u8] {
        &self.bytes[36..]
    }
    pub fn xsdt_addresses(&self) -> Result<XsdtAddresses<'a>, ParseError> {
        if &self.signature != b"XSDT" {
            return Err(ParseError::InvalidSignature);
        }
        if self.body().len() % 8 != 0 {
            return Err(ParseError::InvalidLength);
        }
        Ok(XsdtAddresses {
            bytes: self.body(),
            offset: 0,
        })
    }
}

pub struct XsdtAddresses<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Iterator for XsdtAddresses<'_> {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let value = read_u64(self.bytes, self.offset).ok()?;
        self.offset += 8;
        Some(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McfgAllocation {
    pub base_address: u64,
    pub segment: u16,
    pub start_bus: u8,
    pub end_bus: u8,
}

pub struct McfgAllocations<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> McfgAllocations<'a> {
    pub fn parse(sdt: &'a Sdt<'a>) -> Result<Self, ParseError> {
        if &sdt.signature != b"MCFG" {
            return Err(ParseError::InvalidSignature);
        }
        let body = sdt.body();
        if body.len() < 8 || (body.len() - 8) % 16 != 0 {
            return Err(ParseError::InvalidLength);
        }
        Ok(Self {
            bytes: body,
            offset: 8,
        })
    }
}
impl Iterator for McfgAllocations<'_> {
    type Item = Result<McfgAllocation, ParseError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let o = self.offset;
        self.offset += 16;
        let result = (|| {
            let allocation = McfgAllocation {
                base_address: read_u64(self.bytes, o)?,
                segment: read_u16(self.bytes, o + 8)?,
                start_bus: self.bytes[o + 10],
                end_bus: self.bytes[o + 11],
            };
            if allocation.start_bus > allocation.end_bus {
                return Err(ParseError::InvalidValue);
            }
            Ok(allocation)
        })();
        Some(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadtEntry<'a> {
    LocalApic {
        processor_id: u8,
        apic_id: u8,
        flags: u32,
    },
    IoApic {
        id: u8,
        address: u32,
        global_interrupt_base: u32,
    },
    InterruptOverride {
        bus: u8,
        source: u8,
        global_interrupt: u32,
        flags: u16,
    },
    Other {
        kind: u8,
        data: &'a [u8],
    },
}

pub struct MadtEntries<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> MadtEntries<'a> {
    pub fn parse(sdt: &'a Sdt<'a>) -> Result<(u32, u32, Self), ParseError> {
        if &sdt.signature != b"APIC" {
            return Err(ParseError::InvalidSignature);
        }
        let body = sdt.body();
        if body.len() < 8 {
            return Err(ParseError::InvalidLength);
        }
        Ok((
            read_u32(body, 0)?,
            read_u32(body, 4)?,
            Self {
                bytes: &body[8..],
                offset: 0,
            },
        ))
    }
}
impl<'a> Iterator for MadtEntries<'a> {
    type Item = Result<MadtEntry<'a>, ParseError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        if self.offset + 2 > self.bytes.len() {
            self.offset = self.bytes.len();
            return Some(Err(ParseError::Truncated));
        }
        let kind = self.bytes[self.offset];
        let len = self.bytes[self.offset + 1] as usize;
        if len < 2 || self.offset + len > self.bytes.len() {
            self.offset = self.bytes.len();
            return Some(Err(ParseError::InvalidLength));
        }
        let e = &self.bytes[self.offset..self.offset + len];
        self.offset += len;
        Some(match kind {
            0 if len >= 8 => Ok(MadtEntry::LocalApic {
                processor_id: e[2],
                apic_id: e[3],
                flags: read_u32(e, 4).unwrap(),
            }),
            1 if len >= 12 => Ok(MadtEntry::IoApic {
                id: e[2],
                address: read_u32(e, 4).unwrap(),
                global_interrupt_base: read_u32(e, 8).unwrap(),
            }),
            2 if len >= 10 => Ok(MadtEntry::InterruptOverride {
                bus: e[2],
                source: e[3],
                global_interrupt: read_u32(e, 4).unwrap(),
                flags: read_u16(e, 8).unwrap(),
            }),
            0..=2 => Err(ParseError::InvalidLength),
            _ => Ok(MadtEntry::Other {
                kind,
                data: &e[2..],
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checksum(buf: &mut [u8]) {
        buf[9] = 0;
        buf[9] = 0u8.wrapping_sub(buf.iter().fold(0u8, |a, x| a.wrapping_add(*x)));
    }
    fn sdt(sig: &[u8; 4], body: &[u8]) -> [u8; 80] {
        let mut b = [0u8; 80];
        let len = 36 + body.len();
        b[..4].copy_from_slice(sig);
        b[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        b[8] = 1;
        b[36..len].copy_from_slice(body);
        let sum = b[..len].iter().fold(0u8, |a, x| a.wrapping_add(*x));
        b[9] = 0u8.wrapping_sub(sum);
        b
    }
    #[test]
    fn parses_rsdp_v1() {
        let mut b = [0u8; 20];
        b[..8].copy_from_slice(b"RSD PTR ");
        b[9..15].copy_from_slice(b"NOVAOS");
        b[16..20].copy_from_slice(&0x1234u32.to_le_bytes());
        checksum(&mut b);
        assert_eq!(Rsdp::parse(&b).unwrap().rsdt_address, 0x1234);
    }
    #[test]
    fn bad_rsdp_checksum_fails() {
        let b = [0u8; 20];
        assert_eq!(Rsdp::parse(&b), Err(ParseError::InvalidSignature));
    }
    #[test]
    fn iterates_xsdt() {
        let mut body = [0u8; 16];
        body[..8].copy_from_slice(&0x1000u64.to_le_bytes());
        body[8..].copy_from_slice(&0x2000u64.to_le_bytes());
        let b = sdt(b"XSDT", &body);
        let t = Sdt::parse(&b).unwrap();
        let mut got = t.xsdt_addresses().unwrap();
        assert_eq!(got.next(), Some(0x1000));
        assert_eq!(got.next(), Some(0x2000));
        assert_eq!(got.next(), None);
    }
    #[test]
    fn parses_mcfg_and_rejects_reverse_range() {
        let mut body = [0u8; 24];
        body[8..16].copy_from_slice(&0xe000_0000u64.to_le_bytes());
        body[18] = 2;
        body[19] = 1;
        let b = sdt(b"MCFG", &body);
        let t = Sdt::parse(&b).unwrap();
        assert_eq!(
            McfgAllocations::parse(&t).unwrap().next().unwrap(),
            Err(ParseError::InvalidValue)
        );
    }
    #[test]
    fn parses_madt_entries() {
        let body = [0, 0, 0xe0, 0xfe, 1, 0, 0, 0, 0, 8, 2, 3, 1, 0, 0, 0];
        let b = sdt(b"APIC", &body);
        let t = Sdt::parse(&b).unwrap();
        let (_, _, mut it) = MadtEntries::parse(&t).unwrap();
        assert_eq!(
            it.next().unwrap().unwrap(),
            MadtEntry::LocalApic {
                processor_id: 2,
                apic_id: 3,
                flags: 1
            }
        );
    }
}
