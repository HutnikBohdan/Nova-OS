use crate::{Error, Result, ct_eq};

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(7);
}
fn chacha_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&[0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
    for i in 0..8 {
        s[4 + i] = u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]])
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = u32::from_le_bytes([
            nonce[i * 4],
            nonce[i * 4 + 1],
            nonce[i * 4 + 2],
            nonce[i * 4 + 3],
        ])
    }
    let original = s;
    for _ in 0..10 {
        quarter_round(&mut s, 0, 4, 8, 12);
        quarter_round(&mut s, 1, 5, 9, 13);
        quarter_round(&mut s, 2, 6, 10, 14);
        quarter_round(&mut s, 3, 7, 11, 15);
        quarter_round(&mut s, 0, 5, 10, 15);
        quarter_round(&mut s, 1, 6, 11, 12);
        quarter_round(&mut s, 2, 7, 8, 13);
        quarter_round(&mut s, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&s[i].wrapping_add(original[i]).to_le_bytes())
    }
    out
}
fn chacha_xor(key: &[u8; 32], nonce: &[u8; 12], data: &mut [u8]) {
    let mut counter = 1u32;
    for chunk in data.chunks_mut(64) {
        let block = chacha_block(key, counter, nonce);
        for i in 0..chunk.len() {
            chunk[i] ^= block[i]
        }
        counter = counter.wrapping_add(1);
    }
}

struct Poly1305 {
    r: [u64; 5],
    h: [u64; 5],
    pad: [u32; 4],
}
impl Poly1305 {
    fn new(key: &[u8; 32]) -> Self {
        let t0 = u32::from_le_bytes(key[0..4].try_into().unwrap()) as u64;
        let t1 = u32::from_le_bytes(key[4..8].try_into().unwrap()) as u64;
        let t2 = u32::from_le_bytes(key[8..12].try_into().unwrap()) as u64;
        let t3 = u32::from_le_bytes(key[12..16].try_into().unwrap()) as u64;
        Self {
            r: [
                t0 & 0x3ffffff,
                ((t0 >> 26) | (t1 << 6)) & 0x3ffff03,
                ((t1 >> 20) | (t2 << 12)) & 0x3ffc0ff,
                ((t2 >> 14) | (t3 << 18)) & 0x3f03fff,
                (t3 >> 8) & 0x00fffff,
            ],
            h: [0; 5],
            pad: [
                u32::from_le_bytes(key[16..20].try_into().unwrap()),
                u32::from_le_bytes(key[20..24].try_into().unwrap()),
                u32::from_le_bytes(key[24..28].try_into().unwrap()),
                u32::from_le_bytes(key[28..32].try_into().unwrap()),
            ],
        }
    }
    fn block(&mut self, block: &[u8; 16], full: bool) {
        let t0 = u32::from_le_bytes(block[0..4].try_into().unwrap()) as u64;
        let t1 = u32::from_le_bytes(block[4..8].try_into().unwrap()) as u64;
        let t2 = u32::from_le_bytes(block[8..12].try_into().unwrap()) as u64;
        let t3 = u32::from_le_bytes(block[12..16].try_into().unwrap()) as u64;
        self.h[0] += t0 & 0x3ffffff;
        self.h[1] += ((t0 >> 26) | (t1 << 6)) & 0x3ffffff;
        self.h[2] += ((t1 >> 20) | (t2 << 12)) & 0x3ffffff;
        self.h[3] += ((t2 >> 14) | (t3 << 18)) & 0x3ffffff;
        self.h[4] += (t3 >> 8) | if full { 1 << 24 } else { 0 };
        let r = &self.r;
        let s1 = r[1] * 5;
        let s2 = r[2] * 5;
        let s3 = r[3] * 5;
        let s4 = r[4] * 5;
        let d0 =
            self.h[0] * r[0] + self.h[1] * s4 + self.h[2] * s3 + self.h[3] * s2 + self.h[4] * s1;
        let mut d1 =
            self.h[0] * r[1] + self.h[1] * r[0] + self.h[2] * s4 + self.h[3] * s3 + self.h[4] * s2;
        let mut d2 = self.h[0] * r[2]
            + self.h[1] * r[1]
            + self.h[2] * r[0]
            + self.h[3] * s4
            + self.h[4] * s3;
        let mut d3 = self.h[0] * r[3]
            + self.h[1] * r[2]
            + self.h[2] * r[1]
            + self.h[3] * r[0]
            + self.h[4] * s4;
        let mut d4 = self.h[0] * r[4]
            + self.h[1] * r[3]
            + self.h[2] * r[2]
            + self.h[3] * r[1]
            + self.h[4] * r[0];
        let mut c = d0 >> 26;
        self.h[0] = d0 & 0x3ffffff;
        d1 += c;
        c = d1 >> 26;
        self.h[1] = d1 & 0x3ffffff;
        d2 += c;
        c = d2 >> 26;
        self.h[2] = d2 & 0x3ffffff;
        d3 += c;
        c = d3 >> 26;
        self.h[3] = d3 & 0x3ffffff;
        d4 += c;
        c = d4 >> 26;
        self.h[4] = d4 & 0x3ffffff;
        self.h[0] += c * 5;
        c = self.h[0] >> 26;
        self.h[0] &= 0x3ffffff;
        self.h[1] += c;
    }
    fn update_padded(&mut self, data: &[u8]) {
        let mut chunks = data.chunks_exact(16);
        for chunk in &mut chunks {
            let mut b = [0u8; 16];
            b.copy_from_slice(chunk);
            self.block(&b, true)
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut b = [0u8; 16];
            b[..rem.len()].copy_from_slice(rem);
            self.block(&b, true)
        }
    }
    fn finish(mut self) -> [u8; 16] {
        let mut c = self.h[1] >> 26;
        self.h[1] &= 0x3ffffff;
        self.h[2] += c;
        c = self.h[2] >> 26;
        self.h[2] &= 0x3ffffff;
        self.h[3] += c;
        c = self.h[3] >> 26;
        self.h[3] &= 0x3ffffff;
        self.h[4] += c;
        c = self.h[4] >> 26;
        self.h[4] &= 0x3ffffff;
        self.h[0] += c * 5;
        c = self.h[0] >> 26;
        self.h[0] &= 0x3ffffff;
        self.h[1] += c;
        let mut g = [0u64; 5];
        g[0] = self.h[0] + 5;
        c = g[0] >> 26;
        g[0] &= 0x3ffffff;
        let mut i = 1;
        while i < 4 {
            g[i] = self.h[i] + c;
            c = g[i] >> 26;
            g[i] &= 0x3ffffff;
            i += 1;
        }
        g[4] = (self.h[4] + c).wrapping_sub(1 << 26);
        let mask = (g[4] >> 63).wrapping_sub(1);
        for (h, selected) in self.h.iter_mut().zip(g) {
            *h = (*h & !mask) | (selected & mask)
        }
        let f0 = (self.h[0] | (self.h[1] << 26)) as u32;
        let f1 = ((self.h[1] >> 6) | (self.h[2] << 20)) as u32;
        let f2 = ((self.h[2] >> 12) | (self.h[3] << 14)) as u32;
        let f3 = ((self.h[3] >> 18) | (self.h[4] << 8)) as u32;
        let mut f = f0 as u64 + self.pad[0] as u64;
        let o0 = f as u32;
        f = f1 as u64 + self.pad[1] as u64 + (f >> 32);
        let o1 = f as u32;
        f = f2 as u64 + self.pad[2] as u64 + (f >> 32);
        let o2 = f as u32;
        f = f3 as u64 + self.pad[3] as u64 + (f >> 32);
        let o3 = f as u32;
        let mut out = [0u8; 16];
        out[..4].copy_from_slice(&o0.to_le_bytes());
        out[4..8].copy_from_slice(&o1.to_le_bytes());
        out[8..12].copy_from_slice(&o2.to_le_bytes());
        out[12..].copy_from_slice(&o3.to_le_bytes());
        out
    }
}
fn tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let block = chacha_block(key, 0, nonce);
    let mut one = [0u8; 32];
    one.copy_from_slice(&block[..32]);
    let mut p = Poly1305::new(&one);
    p.update_padded(aad);
    p.update_padded(ciphertext);
    let mut lengths = [0u8; 16];
    lengths[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lengths[8..].copy_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    p.update_padded(&lengths);
    p.finish()
}

/// Encrypts `buffer[..plaintext_len]` and appends the 16-byte authentication tag.
pub fn seal_in_place(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    buffer: &mut [u8],
    plaintext_len: usize,
) -> Result<usize> {
    if plaintext_len > buffer.len() || buffer.len() - plaintext_len < 16 {
        return Err(Error::BufferTooSmall);
    }
    chacha_xor(key, nonce, &mut buffer[..plaintext_len]);
    let t = tag(key, nonce, aad, &buffer[..plaintext_len]);
    buffer[plaintext_len..plaintext_len + 16].copy_from_slice(&t);
    Ok(plaintext_len + 16)
}
/// Authenticates then decrypts `buffer[..ciphertext_and_tag_len]`.
pub fn open_in_place(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    buffer: &mut [u8],
    ciphertext_and_tag_len: usize,
) -> Result<usize> {
    if ciphertext_and_tag_len > buffer.len() || ciphertext_and_tag_len < 16 {
        return Err(Error::InvalidLength);
    }
    let n = ciphertext_and_tag_len - 16;
    let expected = tag(key, nonce, aad, &buffer[..n]);
    if !ct_eq(&expected, &buffer[n..ciphertext_and_tag_len]) {
        return Err(Error::InvalidTag);
    }
    chacha_xor(key, nonce, &mut buffer[..n]);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()
        }
    }
    #[test]
    fn rfc8439_aead() {
        let mut key = [0; 32];
        hex(
            "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
            &mut key,
        );
        let mut nonce = [0; 12];
        hex("070000004041424344454647", &mut nonce);
        let aad = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let msg=b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let mut buf = [0u8; 130];
        buf[..msg.len()].copy_from_slice(msg);
        let n = seal_in_place(&key, &nonce, &aad, &mut buf, msg.len()).unwrap();
        let mut expected = [0u8; 130];
        hex(
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691",
            &mut expected,
        );
        assert_eq!(&buf[..n], &expected);
        assert_eq!(
            open_in_place(&key, &nonce, &aad, &mut buf, n).unwrap(),
            msg.len()
        );
        assert_eq!(&buf[..msg.len()], msg)
    }
    #[test]
    fn rejects_modified_tag() {
        let mut b = [0u8; 32];
        let n = seal_in_place(&[7; 32], &[8; 12], b"a", &mut b, 3).unwrap();
        b[n - 1] ^= 1;
        assert_eq!(
            open_in_place(&[7; 32], &[8; 12], b"a", &mut b, n),
            Err(Error::InvalidTag)
        );
    }
}
