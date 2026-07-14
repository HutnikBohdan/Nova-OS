#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerError {
    Truncated,
    IndefiniteLength,
    NonCanonicalLength,
    LengthOverflow,
    UnexpectedTag,
    TrailingData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element<'a> {
    pub tag: u8,
    pub value: &'a [u8],
    pub encoded: &'a [u8],
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub const fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    pub fn next(&mut self) -> Result<Element<'a>, DerError> {
        let start = self.offset;
        let tag = *self.bytes.get(self.offset).ok_or(DerError::Truncated)?;
        self.offset += 1;
        if tag & 0x1f == 0x1f {
            return Err(DerError::UnexpectedTag);
        }
        let first = *self.bytes.get(self.offset).ok_or(DerError::Truncated)?;
        self.offset += 1;
        let length = if first & 0x80 == 0 {
            first as usize
        } else {
            let count = (first & 0x7f) as usize;
            if count == 0 {
                return Err(DerError::IndefiniteLength);
            }
            if count > core::mem::size_of::<usize>() {
                return Err(DerError::LengthOverflow);
            }
            let length_bytes = self
                .bytes
                .get(self.offset..self.offset + count)
                .ok_or(DerError::Truncated)?;
            if length_bytes[0] == 0 {
                return Err(DerError::NonCanonicalLength);
            }
            self.offset += count;
            let mut length = 0usize;
            for byte in length_bytes {
                length = length
                    .checked_mul(256)
                    .and_then(|value| value.checked_add(*byte as usize))
                    .ok_or(DerError::LengthOverflow)?;
            }
            if length < 128 {
                return Err(DerError::NonCanonicalLength);
            }
            length
        };
        let end = self
            .offset
            .checked_add(length)
            .ok_or(DerError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DerError::Truncated)?;
        self.offset = end;
        Ok(Element {
            tag,
            value,
            encoded: &self.bytes[start..end],
        })
    }

    pub fn expect(&mut self, tag: u8) -> Result<Element<'a>, DerError> {
        let value = self.next()?;
        if value.tag != tag {
            return Err(DerError::UnexpectedTag);
        }
        Ok(value)
    }

    pub fn finish(self) -> Result<(), DerError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(DerError::TrailingData)
        }
    }
}

pub fn one(bytes: &[u8], tag: u8) -> Result<Element<'_>, DerError> {
    let mut reader = Reader::new(bytes);
    let element = reader.expect(tag)?;
    reader.finish()?;
    Ok(element)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_short_and_long_lengths() {
        assert_eq!(one(&[0x04, 1, 7], 0x04).unwrap().value, &[7]);
        let mut bytes = [0u8; 131];
        bytes[0] = 0x04;
        bytes[1] = 0x81;
        bytes[2] = 128;
        assert_eq!(one(&bytes, 0x04).unwrap().value.len(), 128);
    }

    #[test]
    fn rejects_ber_and_trailing_data() {
        assert_eq!(
            one(&[0x04, 0x80, 0, 0], 0x04),
            Err(DerError::IndefiniteLength)
        );
        assert_eq!(
            one(&[0x04, 0x81, 1, 0], 0x04),
            Err(DerError::NonCanonicalLength)
        );
        assert_eq!(one(&[0x04, 0, 0], 0x04), Err(DerError::TrailingData));
    }
}
