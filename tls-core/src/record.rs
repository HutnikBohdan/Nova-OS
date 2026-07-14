use crate::{
    Error, MAX_RECORD_CIPHERTEXT, MAX_RECORD_PLAINTEXT, Result, TLS_LEGACY_VERSION, open_in_place,
    seal_in_place,
};

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentType {
    ChangeCipherSpec = 20,
    Alert = 21,
    Handshake = 22,
    ApplicationData = 23,
}
impl ContentType {
    fn parse(value: u8) -> Result<Self> {
        match value {
            20 => Ok(Self::ChangeCipherSpec),
            21 => Ok(Self::Alert),
            22 => Ok(Self::Handshake),
            23 => Ok(Self::ApplicationData),
            _ => Err(Error::Decode),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordHeader {
    pub content_type: ContentType,
    pub length: u16,
}

/// Frames one TLSPlaintext/TLSCiphertext record and returns bytes written.
pub fn encode_record(
    content_type: ContentType,
    payload: &[u8],
    output: &mut [u8],
) -> Result<usize> {
    let cap = if content_type == ContentType::ApplicationData {
        MAX_RECORD_CIPHERTEXT
    } else {
        MAX_RECORD_PLAINTEXT
    };
    if payload.len() > cap || payload.len() > u16::MAX as usize {
        return Err(Error::InvalidLength);
    }
    if output.len() < 5 + payload.len() {
        return Err(Error::BufferTooSmall);
    }
    output[0] = content_type as u8;
    output[1..3].copy_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
    output[3..5].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    output[5..5 + payload.len()].copy_from_slice(payload);
    Ok(5 + payload.len())
}
/// Decodes exactly one record. Trailing bytes are returned for stream reassembly.
pub fn decode_record(input: &[u8]) -> Result<(RecordHeader, &[u8], &[u8])> {
    if input.len() < 5 {
        return Err(Error::Decode);
    }
    let content_type = ContentType::parse(input[0])?;
    if u16::from_be_bytes([input[1], input[2]]) != TLS_LEGACY_VERSION {
        return Err(Error::UnsupportedVersion);
    }
    let length = u16::from_be_bytes([input[3], input[4]]) as usize;
    let cap = if content_type == ContentType::ApplicationData {
        MAX_RECORD_CIPHERTEXT
    } else {
        MAX_RECORD_PLAINTEXT
    };
    if length > cap {
        return Err(Error::InvalidLength);
    }
    if input.len() < 5 + length {
        return Err(Error::Decode);
    }
    Ok((
        RecordHeader {
            content_type,
            length: length as u16,
        },
        &input[5..5 + length],
        &input[5 + length..],
    ))
}

/// Stateful TLS 1.3 record protection. A sequence number is consumed only after
/// a successful operation, preventing desynchronization on rejected records.
pub struct RecordProtector {
    key: [u8; 32],
    iv: [u8; 12],
    sequence: u64,
}
impl RecordProtector {
    pub const fn new(key: [u8; 32], iv: [u8; 12]) -> Self {
        Self {
            key,
            iv,
            sequence: 0,
        }
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        let seq = self.sequence.to_be_bytes();
        for i in 0..8 {
            n[4 + i] ^= seq[i]
        }
        n
    }
    pub fn seal(
        &mut self,
        content_type: ContentType,
        plaintext: &[u8],
        padding: usize,
        output: &mut [u8],
    ) -> Result<usize> {
        if self.sequence == u64::MAX {
            return Err(Error::SequenceOverflow);
        }
        let inner = plaintext
            .len()
            .checked_add(1)
            .and_then(|x| x.checked_add(padding))
            .ok_or(Error::InvalidLength)?;
        let cipher_len = inner.checked_add(16).ok_or(Error::InvalidLength)?;
        if inner > MAX_RECORD_PLAINTEXT || cipher_len > MAX_RECORD_CIPHERTEXT {
            return Err(Error::InvalidLength);
        }
        if output.len() < 5 + cipher_len {
            return Err(Error::BufferTooSmall);
        }
        output[5..5 + plaintext.len()].copy_from_slice(plaintext);
        output[5 + plaintext.len()] = content_type as u8;
        output[6 + plaintext.len()..5 + inner].fill(0);
        let mut aad = [0u8; 5];
        aad[0] = ContentType::ApplicationData as u8;
        aad[1..3].copy_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
        aad[3..5].copy_from_slice(&(cipher_len as u16).to_be_bytes());
        let nonce = self.nonce();
        seal_in_place(
            &self.key,
            &nonce,
            &aad,
            &mut output[5..5 + cipher_len],
            inner,
        )?;
        output[..5].copy_from_slice(&aad);
        self.sequence += 1;
        Ok(5 + cipher_len)
    }
    pub fn open(&mut self, record: &[u8], output: &mut [u8]) -> Result<(ContentType, usize)> {
        if self.sequence == u64::MAX {
            return Err(Error::SequenceOverflow);
        }
        let (header, payload, rest) = decode_record(record)?;
        if header.content_type != ContentType::ApplicationData || !rest.is_empty() {
            return Err(Error::UnexpectedMessage);
        }
        if output.len() < payload.len() {
            return Err(Error::BufferTooSmall);
        }
        output[..payload.len()].copy_from_slice(payload);
        let nonce = self.nonce();
        let plain_len = open_in_place(&self.key, &nonce, &record[..5], output, payload.len())?;
        let mut end = plain_len;
        while end > 0 && output[end - 1] == 0 {
            end -= 1
        }
        if end == 0 {
            return Err(Error::Decode);
        }
        let kind = ContentType::parse(output[end - 1])?;
        self.sequence += 1;
        Ok((kind, end - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_roundtrip() {
        let mut out = [0; 32];
        let n = encode_record(ContentType::Handshake, b"nova", &mut out).unwrap();
        let (h, p, rest) = decode_record(&out[..n]).unwrap();
        assert_eq!(h.length, 4);
        assert_eq!(p, b"nova");
        assert!(rest.is_empty())
    }
    #[test]
    fn rejects_oversize_before_slice() {
        let input = [23, 3, 3, 0xff, 0xff];
        assert_eq!(decode_record(&input), Err(Error::InvalidLength));
    }
    #[test]
    fn protected_record_roundtrip() {
        let mut sender = RecordProtector::new([3; 32], [4; 12]);
        let mut receiver = RecordProtector::new([3; 32], [4; 12]);
        let mut wire = [0u8; 128];
        let n = sender
            .seal(ContentType::Handshake, b"encrypted nova", 7, &mut wire)
            .unwrap();
        let mut plain = [0u8; 128];
        let (kind, len) = receiver.open(&wire[..n], &mut plain).unwrap();
        assert_eq!(kind, ContentType::Handshake);
        assert_eq!(&plain[..len], b"encrypted nova");
        assert_eq!(sender.sequence(), 1);
        assert_eq!(receiver.sequence(), 1)
    }
    #[test]
    fn failed_tag_does_not_advance_sequence() {
        let mut sender = RecordProtector::new([3; 32], [4; 12]);
        let mut receiver = RecordProtector::new([3; 32], [4; 12]);
        let mut wire = [0u8; 64];
        let n = sender.seal(ContentType::Alert, b"x", 0, &mut wire).unwrap();
        wire[n - 1] ^= 1;
        let mut plain = [0u8; 64];
        assert_eq!(
            receiver.open(&wire[..n], &mut plain),
            Err(Error::InvalidTag)
        );
        assert_eq!(receiver.sequence(), 0)
    }
}
