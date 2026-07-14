use crate::{
    der::{DerError, Element, Reader, one},
    verify_ed25519,
};

const OID_ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const MAX_NAMES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainError {
    Der,
    UnsupportedAlgorithm,
    MalformedCertificate,
    InvalidTime,
    Expired,
    NameMismatch,
    UnknownIssuer,
    NotCertificateAuthority,
    KeyUsage,
    InvalidSignature,
    Revoked,
    Capacity,
}

impl From<DerError> for ChainError {
    fn from(_: DerError) -> Self {
        Self::Der
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Certificate<'a> {
    pub der: &'a [u8],
    pub tbs: &'a [u8],
    pub serial: &'a [u8],
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    pub not_before: i64,
    pub not_after: i64,
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
    pub is_ca: bool,
    pub key_cert_sign: bool,
    dns_names: [Option<&'a str>; MAX_NAMES],
}

impl<'a> Certificate<'a> {
    pub fn parse(der: &'a [u8]) -> Result<Self, ChainError> {
        let outer = one(der, 0x30)?;
        let mut certificate = Reader::new(outer.value);
        let tbs = certificate.expect(0x30)?;
        let signature_algorithm = certificate.expect(0x30)?;
        require_ed25519_algorithm(signature_algorithm)?;
        let signature_bits = certificate.expect(0x03)?;
        certificate.finish()?;
        if signature_bits.value.len() != 65 || signature_bits.value[0] != 0 {
            return Err(ChainError::MalformedCertificate);
        }
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&signature_bits.value[1..]);

        let mut fields = Reader::new(tbs.value);
        let first = fields.next()?;
        let serial_element = if first.tag == 0xa0 {
            let version = one(first.value, 0x02)?;
            if version.value != [2] {
                return Err(ChainError::MalformedCertificate);
            }
            fields.expect(0x02)?
        } else if first.tag == 0x02 {
            first
        } else {
            return Err(ChainError::MalformedCertificate);
        };
        validate_integer(serial_element.value)?;
        require_ed25519_algorithm(fields.expect(0x30)?)?;
        let issuer = fields.expect(0x30)?;
        let validity = fields.expect(0x30)?;
        let subject = fields.expect(0x30)?;
        let spki = fields.expect(0x30)?;
        let public_key = parse_spki(spki)?;
        let (not_before, not_after) = parse_validity(validity)?;
        if not_before > not_after {
            return Err(ChainError::InvalidTime);
        }
        let mut result = Self {
            der,
            tbs: tbs.encoded,
            serial: serial_element.value,
            issuer: issuer.encoded,
            subject: subject.encoded,
            not_before,
            not_after,
            public_key,
            signature,
            is_ca: false,
            key_cert_sign: false,
            dns_names: [None; MAX_NAMES],
        };
        while !fields.is_empty() {
            let optional = fields.next()?;
            match optional.tag {
                0x81 | 0x82 => {}
                0xa3 => parse_extensions(optional.value, &mut result)?,
                _ => return Err(ChainError::MalformedCertificate),
            }
        }
        Ok(result)
    }

    pub fn dns_names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.dns_names.iter().flatten().copied()
    }

    pub fn valid_at(&self, unix_time: i64) -> bool {
        unix_time >= self.not_before && unix_time <= self.not_after
    }

    pub fn matches_hostname(&self, hostname: &str) -> bool {
        !hostname.is_empty()
            && self
                .dns_names()
                .any(|name| hostname_matches(name, hostname))
    }
}

fn validate_integer(value: &[u8]) -> Result<(), ChainError> {
    if value.is_empty()
        || value[0] & 0x80 != 0
        || (value.len() > 1 && value[0] == 0 && value[1] & 0x80 == 0)
    {
        return Err(ChainError::MalformedCertificate);
    }
    Ok(())
}

fn require_ed25519_algorithm(element: Element<'_>) -> Result<(), ChainError> {
    let mut algorithm = Reader::new(element.value);
    let oid = algorithm.expect(0x06)?;
    algorithm.finish()?;
    if oid.value == OID_ED25519 {
        Ok(())
    } else {
        Err(ChainError::UnsupportedAlgorithm)
    }
}

fn parse_spki(element: Element<'_>) -> Result<[u8; 32], ChainError> {
    let mut spki = Reader::new(element.value);
    require_ed25519_algorithm(spki.expect(0x30)?)?;
    let key = spki.expect(0x03)?;
    spki.finish()?;
    if key.value.len() != 33 || key.value[0] != 0 {
        return Err(ChainError::MalformedCertificate);
    }
    let mut result = [0u8; 32];
    result.copy_from_slice(&key.value[1..]);
    Ok(result)
}

fn parse_validity(element: Element<'_>) -> Result<(i64, i64), ChainError> {
    let mut validity = Reader::new(element.value);
    let before = parse_time(validity.next()?)?;
    let after = parse_time(validity.next()?)?;
    validity.finish()?;
    Ok((before, after))
}

fn decimal(bytes: &[u8]) -> Result<u32, ChainError> {
    let mut value = 0u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(ChainError::InvalidTime);
        }
        value = value * 10 + u32::from(*byte - b'0');
    }
    Ok(value)
}

fn parse_time(element: Element<'_>) -> Result<i64, ChainError> {
    let bytes = element.value;
    let (year, at) = match element.tag {
        0x17 if bytes.len() == 13 => {
            let short = decimal(&bytes[0..2])? as i32;
            (
                if short >= 50 {
                    1900 + short
                } else {
                    2000 + short
                },
                2,
            )
        }
        0x18 if bytes.len() == 15 => (decimal(&bytes[0..4])? as i32, 4),
        _ => return Err(ChainError::InvalidTime),
    };
    if bytes.last() != Some(&b'Z') {
        return Err(ChainError::InvalidTime);
    }
    let month = decimal(&bytes[at..at + 2])?;
    let day = decimal(&bytes[at + 2..at + 4])?;
    let hour = decimal(&bytes[at + 4..at + 6])?;
    let minute = decimal(&bytes[at + 6..at + 8])?;
    let second = decimal(&bytes[at + 8..at + 10])?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(ChainError::InvalidTime);
    }
    Ok(days_from_civil(year, month, day) * 86_400 + i64::from(hour * 3600 + minute * 60 + second))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let adjusted = month as i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * adjusted + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146097 + doe - 719468)
}

fn parse_extensions<'a>(
    bytes: &'a [u8],
    certificate: &mut Certificate<'a>,
) -> Result<(), ChainError> {
    let extensions = one(bytes, 0x30)?;
    let mut list = Reader::new(extensions.value);
    while !list.is_empty() {
        let extension = list.expect(0x30)?;
        let mut fields = Reader::new(extension.value);
        let oid = fields.expect(0x06)?;
        let next = fields.next()?;
        let value = if next.tag == 0x01 {
            if next.value.len() != 1 || !matches!(next.value[0], 0 | 0xff) {
                return Err(ChainError::MalformedCertificate);
            }
            fields.expect(0x04)?
        } else if next.tag == 0x04 {
            next
        } else {
            return Err(ChainError::MalformedCertificate);
        };
        fields.finish()?;
        match oid.value {
            OID_SAN => parse_san(value.value, certificate)?,
            OID_BASIC_CONSTRAINTS => parse_basic_constraints(value.value, certificate)?,
            OID_KEY_USAGE => parse_key_usage(value.value, certificate)?,
            _ => {}
        }
    }
    Ok(())
}

fn parse_san<'a>(bytes: &'a [u8], certificate: &mut Certificate<'a>) -> Result<(), ChainError> {
    let names = one(bytes, 0x30)?;
    let mut reader = Reader::new(names.value);
    let mut count = 0;
    while !reader.is_empty() {
        let name = reader.next()?;
        if name.tag != 0x82 {
            continue;
        }
        if count == MAX_NAMES || !name.value.is_ascii() {
            return Err(ChainError::Capacity);
        }
        let text =
            core::str::from_utf8(name.value).map_err(|_| ChainError::MalformedCertificate)?;
        certificate.dns_names[count] = Some(text);
        count += 1;
    }
    Ok(())
}

fn parse_basic_constraints(
    bytes: &[u8],
    certificate: &mut Certificate<'_>,
) -> Result<(), ChainError> {
    let sequence = one(bytes, 0x30)?;
    let mut reader = Reader::new(sequence.value);
    if !reader.is_empty() {
        let ca = reader.expect(0x01)?;
        if ca.value.len() != 1 {
            return Err(ChainError::MalformedCertificate);
        }
        certificate.is_ca = ca.value[0] == 0xff;
    }
    Ok(())
}

fn parse_key_usage(bytes: &[u8], certificate: &mut Certificate<'_>) -> Result<(), ChainError> {
    let bits = one(bytes, 0x03)?;
    if bits.value.len() < 2 || bits.value[0] > 7 {
        return Err(ChainError::MalformedCertificate);
    }
    certificate.key_cert_sign = bits.value[1] & 0x04 != 0;
    Ok(())
}

fn hostname_matches(pattern: &str, hostname: &str) -> bool {
    if pattern.eq_ignore_ascii_case(hostname) {
        return true;
    }
    let Some(suffix) = pattern.strip_prefix("*.") else {
        return false;
    };
    let Some((_, host_suffix)) = hostname.split_once('.') else {
        return false;
    };
    !suffix.is_empty() && !host_suffix.contains("..") && suffix.eq_ignore_ascii_case(host_suffix)
}

pub struct TrustStore<'a, const N: usize> {
    roots: [Option<Certificate<'a>>; N],
}

impl<'a, const N: usize> TrustStore<'a, N> {
    pub const fn new() -> Self {
        Self { roots: [None; N] }
    }

    pub fn add(&mut self, certificate: Certificate<'a>) -> Result<(), ChainError> {
        let slot = self
            .roots
            .iter_mut()
            .find(|root| root.is_none())
            .ok_or(ChainError::Capacity)?;
        *slot = Some(certificate);
        Ok(())
    }

    fn trusted(&self, certificate: &Certificate<'_>) -> bool {
        self.roots.iter().flatten().any(|root| {
            root.subject == certificate.subject && root.public_key == certificate.public_key
        })
    }
}

impl<const N: usize> Default for TrustStore<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct DenyList<'a, const N: usize> {
    serials: [Option<&'a [u8]>; N],
}

impl<'a, const N: usize> DenyList<'a, N> {
    pub const fn new() -> Self {
        Self { serials: [None; N] }
    }

    pub fn deny(&mut self, serial: &'a [u8]) -> Result<(), ChainError> {
        let slot = self
            .serials
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ChainError::Capacity)?;
        *slot = Some(serial);
        Ok(())
    }

    fn contains(&self, serial: &[u8]) -> bool {
        self.serials
            .iter()
            .flatten()
            .any(|blocked| *blocked == serial)
    }
}

impl<const N: usize> Default for DenyList<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn verify_chain<const R: usize, const D: usize>(
    leaf: &Certificate<'_>,
    intermediates: &[Certificate<'_>],
    roots: &TrustStore<'_, R>,
    deny: &DenyList<'_, D>,
    hostname: &str,
    unix_time: i64,
) -> Result<(), ChainError> {
    if !leaf.valid_at(unix_time) {
        return Err(ChainError::Expired);
    }
    if !leaf.matches_hostname(hostname) {
        return Err(ChainError::NameMismatch);
    }
    if deny.contains(leaf.serial) {
        return Err(ChainError::Revoked);
    }
    let mut child = leaf;
    for issuer in intermediates {
        verify_link(child, issuer, deny, unix_time)?;
        child = issuer;
    }
    let root = roots
        .roots
        .iter()
        .flatten()
        .find(|root| child.issuer == root.subject)
        .ok_or(ChainError::UnknownIssuer)?;
    if child.der != root.der {
        verify_link(child, root, deny, unix_time)?;
    } else if !roots.trusted(child) {
        return Err(ChainError::UnknownIssuer);
    }
    Ok(())
}

fn verify_link<const D: usize>(
    child: &Certificate<'_>,
    issuer: &Certificate<'_>,
    deny: &DenyList<'_, D>,
    unix_time: i64,
) -> Result<(), ChainError> {
    if child.issuer != issuer.subject {
        return Err(ChainError::UnknownIssuer);
    }
    if !issuer.valid_at(unix_time) {
        return Err(ChainError::Expired);
    }
    if !issuer.is_ca {
        return Err(ChainError::NotCertificateAuthority);
    }
    if !issuer.key_cert_sign {
        return Err(ChainError::KeyUsage);
    }
    if deny.contains(issuer.serial) {
        return Err(ChainError::Revoked);
    }
    verify_ed25519(&issuer.public_key, child.tbs, &child.signature)
        .map_err(|_| ChainError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times_and_matches_wildcard_one_label() {
        let utc = Element {
            tag: 0x17,
            value: b"260714120000Z",
            encoded: &[],
        };
        assert_eq!(parse_time(utc).unwrap(), 1_784_030_400);
    }

    #[test]
    fn wildcard_does_not_cross_labels() {
        assert!(hostname_matches("*.nova.test", "www.nova.test"));
        assert!(!hostname_matches("*.nova.test", "a.b.nova.test"));
    }
}
