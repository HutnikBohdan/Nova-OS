//! Byte fallback and bounded byte-pair encoding.

use crate::{Error, Result};

pub struct ByteTokenizer;
impl ByteTokenizer {
    pub fn encode(input: &[u8], output: &mut [u32]) -> Result<usize> {
        if output.len() < input.len() {
            return Err(Error::OutputTooSmall);
        }
        for (slot, byte) in output.iter_mut().zip(input.iter().copied()) {
            *slot = byte as u32;
        }
        Ok(input.len())
    }
    pub fn decode(tokens: &[u32], output: &mut [u8]) -> Result<usize> {
        if output.len() < tokens.len() {
            return Err(Error::OutputTooSmall);
        }
        for (slot, token) in output.iter_mut().zip(tokens.iter().copied()) {
            *slot = u8::try_from(token).map_err(|_| Error::UnknownToken)?;
        }
        Ok(tokens.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Merge {
    pub left: u32,
    pub right: u32,
    pub result: u32,
    pub rank: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TokenPiece<'a> {
    pub id: u32,
    pub bytes: &'a [u8],
}

pub struct BpeTokenizer<'a> {
    byte_tokens: &'a [u32; 256],
    merges: &'a [Merge],
    pieces: &'a [TokenPiece<'a>],
}

impl<'a> BpeTokenizer<'a> {
    pub const fn new(
        byte_tokens: &'a [u32; 256],
        merges: &'a [Merge],
        pieces: &'a [TokenPiece<'a>],
    ) -> Self {
        Self {
            byte_tokens,
            merges,
            pieces,
        }
    }

    /// Encodes into caller-owned storage. At most `input.len()` tokens are used.
    pub fn encode(&self, input: &[u8], output: &mut [u32]) -> Result<usize> {
        if output.len() < input.len() {
            return Err(Error::OutputTooSmall);
        }
        let mut len = input.len();
        for (slot, byte) in output.iter_mut().zip(input.iter().copied()) {
            *slot = self.byte_tokens[byte as usize];
        }
        loop {
            let mut best: Option<(usize, Merge)> = None;
            for index in 0..len.saturating_sub(1) {
                if let Some(rule) = self.rule(output[index], output[index + 1]) {
                    if best.is_none_or(|(best_index, current)| {
                        (rule.rank, index) < (current.rank, best_index)
                    }) {
                        best = Some((index, rule));
                    }
                }
            }
            let Some((index, rule)) = best else {
                break;
            };
            output[index] = rule.result;
            output.copy_within(index + 2..len, index + 1);
            len -= 1;
        }
        Ok(len)
    }

    pub fn decode(&self, tokens: &[u32], output: &mut [u8]) -> Result<usize> {
        let mut written = 0usize;
        for token in tokens {
            let piece = self
                .pieces
                .iter()
                .find(|piece| piece.id == *token)
                .ok_or(Error::UnknownToken)?;
            let end = written
                .checked_add(piece.bytes.len())
                .ok_or(Error::IntegerOverflow)?;
            if end > output.len() {
                return Err(Error::OutputTooSmall);
            }
            output[written..end].copy_from_slice(piece.bytes);
            written = end;
        }
        Ok(written)
    }

    fn rule(&self, left: u32, right: u32) -> Option<Merge> {
        self.merges
            .iter()
            .copied()
            .filter(|r| r.left == left && r.right == right)
            .min_by_key(|r| r.rank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_roundtrip_binary_and_utf8() {
        let input = "Привіт, Nova".as_bytes();
        let mut ids = [0; 64];
        let mut bytes = [0; 64];
        let n = ByteTokenizer::encode(input, &mut ids).unwrap();
        let m = ByteTokenizer::decode(&ids[..n], &mut bytes).unwrap();
        assert_eq!(&bytes[..m], input);
    }
    #[test]
    fn bpe_uses_rank_and_roundtrips() {
        let mut bytes = [0u32; 256];
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = i as u32;
        }
        let merges = [
            Merge {
                left: b'n' as u32,
                right: b'o' as u32,
                result: 256,
                rank: 1,
            },
            Merge {
                left: 256,
                right: b'v' as u32,
                result: 257,
                rank: 0,
            },
        ];
        let pieces = [
            TokenPiece {
                id: 257,
                bytes: b"nov",
            },
            TokenPiece {
                id: b'a' as u32,
                bytes: b"a",
            },
        ];
        let tokenizer = BpeTokenizer::new(&bytes, &merges, &pieces);
        let mut ids = [0; 8];
        let n = tokenizer.encode(b"nova", &mut ids).unwrap();
        assert_eq!(&ids[..n], &[257, b'a' as u32]);
        let mut out = [0; 8];
        let m = tokenizer.decode(&ids[..n], &mut out).unwrap();
        assert_eq!(&out[..m], b"nova");
    }
    #[test]
    fn reports_bounded_buffer_errors() {
        assert_eq!(
            ByteTokenizer::encode(b"two", &mut [0; 2]),
            Err(Error::OutputTooSmall)
        );
        assert_eq!(
            ByteTokenizer::decode(&[999], &mut [0; 1]),
            Err(Error::UnknownToken)
        );
    }
}
