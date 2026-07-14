use crate::{Error, Result};

const INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: INITIAL,
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }
    pub fn update(&mut self, mut input: &[u8]) {
        self.total_len = self.total_len.wrapping_add(input.len() as u64);
        if self.block_len != 0 {
            let take = (64 - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
        while input.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&input[..64]);
            self.compress(&block);
            input = &input[64..];
        }
        self.block[..input.len()].copy_from_slice(input);
        self.block_len = input.len();
    }
    pub fn finalize(mut self) -> [u8; 32] {
        let bits = self.total_len.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..].copy_from_slice(&bits.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}
impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}
pub fn sha256(input: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(input);
    h.finalize()
}
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut i = [0x36u8; 64];
    let mut o = [0x5cu8; 64];
    for n in 0..64 {
        i[n] ^= k[n];
        o[n] ^= k[n];
    }
    let mut h = Sha256::new();
    h.update(&i);
    h.update(message);
    let x = h.finalize();
    let mut h = Sha256::new();
    h.update(&o);
    h.update(&x);
    h.finalize()
}
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    if salt.is_empty() {
        hmac_sha256(&[0u8; 32], ikm)
    } else {
        hmac_sha256(salt, ikm)
    }
}
pub fn hkdf_expand(prk: &[u8], info: &[u8], out: &mut [u8]) -> Result<()> {
    if out.len() > 255 * 32 || info.len() > 512 {
        return Err(Error::OutputTooLarge);
    }
    let mut previous = [0u8; 32];
    let mut previous_len = 0usize;
    let mut written = 0usize;
    let mut count = 1u8;
    while written < out.len() {
        let mut input = [0u8; 32 + 512 + 1];
        input[..previous_len].copy_from_slice(&previous[..previous_len]);
        input[previous_len..previous_len + info.len()].copy_from_slice(info);
        input[previous_len + info.len()] = count;
        previous = hmac_sha256(prk, &input[..previous_len + info.len() + 1]);
        previous_len = 32;
        let take = (out.len() - written).min(32);
        out[written..written + take].copy_from_slice(&previous[..take]);
        written += take;
        count = count.wrapping_add(1);
    }
    Ok(())
}
pub fn hkdf_expand_label(
    secret: &[u8],
    label: &[u8],
    context: &[u8],
    out: &mut [u8],
) -> Result<()> {
    const PREFIX: &[u8] = b"tls13 ";
    if label.len() + PREFIX.len() > 255 || context.len() > 255 || out.len() > u16::MAX as usize {
        return Err(Error::OutputTooLarge);
    }
    let mut info = [0u8; 2 + 1 + 255 + 1 + 255];
    let mut n = 0;
    info[..2].copy_from_slice(&(out.len() as u16).to_be_bytes());
    n += 2;
    info[n] = (PREFIX.len() + label.len()) as u8;
    n += 1;
    info[n..n + PREFIX.len()].copy_from_slice(PREFIX);
    n += PREFIX.len();
    info[n..n + label.len()].copy_from_slice(label);
    n += label.len();
    info[n] = context.len() as u8;
    n += 1;
    info[n..n + context.len()].copy_from_slice(context);
    n += context.len();
    hkdf_expand(secret, &info[..n], out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }
    #[test]
    fn rfc5869() {
        let ikm = [0x0b; 22];
        let prk = hkdf_extract(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], &ikm);
        let mut out = [0; 42];
        hkdf_expand(
            &prk,
            &[0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9],
            &mut out,
        )
        .unwrap();
        assert_eq!(
            &out[..],
            [
                0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
                0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
                0xec, 0xc4, 0xc5, 0xbf, 0x34, 0, 0x72, 8, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65
            ]
        );
    }
}
