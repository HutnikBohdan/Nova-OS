use crate::{Error, Result};

type Field = [i64; 16];
const C121665: Field = [0xdb41, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

fn carry(a: &mut Field) {
    for i in 0..16 {
        a[i] += 1 << 16;
        let c = a[i] >> 16;
        if i < 15 {
            a[i + 1] += c - 1
        } else {
            a[0] += 38 * (c - 1)
        }
        a[i] -= c << 16
    }
}
fn select(a: &mut Field, b: &mut Field, bit: i64) {
    let mask = -bit;
    for i in 0..16 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t
    }
}
fn add(out: &mut Field, a: &Field, b: &Field) {
    for i in 0..16 {
        out[i] = a[i] + b[i]
    }
}
fn sub(out: &mut Field, a: &Field, b: &Field) {
    for i in 0..16 {
        out[i] = a[i] - b[i]
    }
}
fn mul(out: &mut Field, a: &Field, b: &Field) {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += a[i] * b[j]
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16]
    }
    out.copy_from_slice(&t[..16]);
    carry(out);
    carry(out)
}
fn square(out: &mut Field, a: &Field) {
    mul(out, a, a)
}
fn unpack(input: &[u8; 32]) -> Field {
    let mut out = [0i64; 16];
    for i in 0..16 {
        out[i] = (input[i * 2] as i64) | ((input[i * 2 + 1] as i64) << 8)
    }
    out[15] &= 0x7fff;
    out
}
fn pack(input: &Field) -> [u8; 32] {
    let mut t = *input;
    carry(&mut t);
    carry(&mut t);
    carry(&mut t);
    for _ in 0..2 {
        let mut m = [0i64; 16];
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let bit = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        select(&mut t, &mut m, 1 - bit)
    }
    let mut out = [0u8; 32];
    for i in 0..16 {
        out[i * 2] = t[i] as u8;
        out[i * 2 + 1] = (t[i] >> 8) as u8
    }
    out
}
fn inverse(out: &mut Field, input: &Field) {
    let mut c = *input;
    for exponent in (0..=253).rev() {
        let previous = c;
        square(&mut c, &previous);
        if exponent != 2 && exponent != 4 {
            let previous = c;
            mul(&mut c, &previous, input)
        }
    }
    *out = c
}

/// RFC 7748 X25519. Low-order inputs are rejected instead of returning an all-zero secret.
pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> Result<[u8; 32]> {
    let mut z = *scalar;
    z[0] &= 248;
    z[31] = (z[31] & 127) | 64;
    let x = unpack(point);
    let mut a = [0i64; 16];
    a[0] = 1;
    let mut b = x;
    let mut c = [0i64; 16];
    let mut d = [0i64; 16];
    d[0] = 1;
    for i in (0..255).rev() {
        let r = ((z[i >> 3] >> (i & 7)) & 1) as i64;
        select(&mut a, &mut b, r);
        select(&mut c, &mut d, r);
        let (a0, b0, c0, d0) = (a, b, c, d);
        let mut e = [0; 16];
        add(&mut e, &a0, &c0);
        sub(&mut a, &a0, &c0);
        add(&mut c, &b0, &d0);
        sub(&mut b, &b0, &d0);
        let mut f = [0; 16];
        let e0 = e;
        square(&mut d, &e0);
        let a0 = a;
        square(&mut f, &a0);
        let c0 = c;
        let a0 = a;
        mul(&mut a, &c0, &a0);
        let b0 = b;
        mul(&mut c, &b0, &e0);
        let a0 = a;
        let c0 = c;
        add(&mut e, &a0, &c0);
        sub(&mut a, &a0, &c0);
        let a0 = a;
        square(&mut b, &a0);
        let d0 = d;
        let f0 = f;
        sub(&mut c, &d0, &f0);
        let c0 = c;
        mul(&mut a, &c0, &C121665);
        let a0 = a;
        add(&mut a, &a0, &d0);
        let c0 = c;
        let a0 = a;
        mul(&mut c, &c0, &a0);
        mul(&mut a, &d0, &f0);
        let b0 = b;
        mul(&mut d, &b0, &x);
        let e0 = e;
        square(&mut b, &e0);
        select(&mut a, &mut b, r);
        select(&mut c, &mut d, r)
    }
    let c0 = c;
    inverse(&mut c, &c0);
    let a0 = a;
    let c0 = c;
    mul(&mut a, &a0, &c0);
    let out = pack(&a);
    let mut any = 0u8;
    for byte in out {
        any |= byte
    }
    if any == 0 {
        Err(Error::InvalidPublicKey)
    } else {
        Ok(out)
    }
}
pub fn x25519_base(scalar: &[u8; 32]) -> [u8; 32] {
    let mut base = [0u8; 32];
    base[0] = 9;
    x25519(scalar, &base).expect("base point is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> [u8; 32] {
        let mut o = [0; 32];
        for i in 0..32 {
            o[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()
        }
        o
    }
    #[test]
    fn rfc7748_alice_bob() {
        let alice = parse("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let bob = parse("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let ap = x25519_base(&alice);
        let bp = x25519_base(&bob);
        assert_eq!(
            ap,
            parse("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            bp,
            parse("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let secret = x25519(&alice, &bp).unwrap();
        assert_eq!(
            secret,
            parse("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
        );
        assert_eq!(x25519(&bob, &ap).unwrap(), secret)
    }
    #[test]
    fn low_order_rejected() {
        assert_eq!(x25519(&[1; 32], &[0; 32]), Err(Error::InvalidPublicKey));
    }
}
