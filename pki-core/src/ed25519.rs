use crate::sha512::Sha512;

const MASK: u64 = (1u64 << 51) - 1;
const D: Fe = Fe([
    929955233495203,
    466365720129213,
    1662059464998953,
    2033849074728123,
    1442794654840575,
]);
const SQRT_M1: Fe = Fe([
    1718705420411056,
    234908883556509,
    2233514472574048,
    2117202627021982,
    765476049583133,
]);
const BX: Fe = Fe([
    1738742601995546,
    1146398526822698,
    2070867633025821,
    562264141797630,
    587772402128613,
]);
const BY: Fe = Fe([
    1801439850948184,
    1351079888211148,
    450359962737049,
    900719925474099,
    1801439850948198,
]);
const L: [u64; 4] = [
    0x5812631a5cf5d3ed,
    0x14def9dea2f79cd6,
    0,
    0x1000000000000000,
];

#[derive(Clone, Copy)]
struct Fe([u64; 5]);
impl Fe {
    const ZERO: Self = Self([0; 5]);
    const ONE: Self = Self([1, 0, 0, 0, 0]);
    fn from_bytes(b: &[u8; 32]) -> Option<Self> {
        if b[31] & 0x80 != 0 {
            return None;
        }
        let mut h = [0u64; 5];
        for bit in 0..255 {
            h[bit / 51] |= (((b[bit / 8] >> (bit % 8)) & 1) as u64) << (bit % 51)
        }
        let f = Self(h);
        if ct_eq(&f.to_bytes(), b) == 1 {
            Some(f)
        } else {
            None
        }
    }
    fn carry(mut self) -> Self {
        for _ in 0..2 {
            for i in 0..4 {
                let c = self.0[i] >> 51;
                self.0[i] &= MASK;
                self.0[i + 1] = self.0[i + 1].wrapping_add(c)
            }
            let c = self.0[4] >> 51;
            self.0[4] &= MASK;
            self.0[0] = self.0[0].wrapping_add(c * 19)
        }
        self
    }
    fn to_bytes(self) -> [u8; 32] {
        let mut h = self.carry();
        let mut q = (h.0[0] + 19) >> 51;
        for i in 1..5 {
            q = (h.0[i] + q) >> 51
        }
        h.0[0] = h.0[0].wrapping_add(19 * q);
        for i in 0..4 {
            let c = h.0[i] >> 51;
            h.0[i] &= MASK;
            h.0[i + 1] += c
        }
        h.0[4] &= MASK;
        let mut o = [0u8; 32];
        for bit in 0..255 {
            o[bit / 8] |= (((h.0[bit / 51] >> (bit % 51)) & 1) as u8) << (bit % 8)
        }
        o
    }
    fn add(self, b: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] + b.0[i])).carry()
    }
    fn sub(self, b: Self) -> Self {
        Self([
            self.0[0] + 0xfffffffffffda - b.0[0],
            self.0[1] + 0xffffffffffffe - b.0[1],
            self.0[2] + 0xffffffffffffe - b.0[2],
            self.0[3] + 0xffffffffffffe - b.0[3],
            self.0[4] + 0xffffffffffffe - b.0[4],
        ])
        .carry()
    }
    fn neg(self) -> Self {
        Self::ZERO.sub(self)
    }
    fn mul(self, b: Self) -> Self {
        let a = self.0;
        let b = b.0;
        let mut r = [0u128; 5];
        for i in 0..5 {
            for j in 0..5 {
                let k = i + j;
                if k < 5 {
                    r[k] += a[i] as u128 * b[j] as u128
                } else {
                    r[k - 5] += 19 * a[i] as u128 * b[j] as u128
                }
            }
        }
        for i in 0..4 {
            let c = r[i] >> 51;
            r[i] &= MASK as u128;
            r[i + 1] += c
        }
        let c = r[4] >> 51;
        r[4] &= MASK as u128;
        r[0] += 19 * c;
        Self(core::array::from_fn(|i| r[i] as u64)).carry()
    }
    fn square(self) -> Self {
        self.mul(self)
    }
    fn pow(self, e: &[u8; 32]) -> Self {
        let mut r = Self::ONE;
        for i in (0..255).rev() {
            r = r.square();
            let m = 0u64.wrapping_sub(((e[i / 8] >> (i % 8)) & 1) as u64);
            let t = r.mul(self);
            r = Self(core::array::from_fn(|j| (r.0[j] & !m) | (t.0[j] & m)))
        }
        r
    }
    fn invert(self) -> Self {
        let mut e = [0xff; 32];
        e[0] = 0xeb;
        e[31] = 0x7f;
        self.pow(&e)
    }
    fn is_zero(self) -> u8 {
        ct_eq(&self.to_bytes(), &[0; 32])
    }
    fn is_negative(self) -> u8 {
        self.to_bytes()[0] & 1
    }
    fn select(a: Self, b: Self, choice: u8) -> Self {
        let m = 0u64.wrapping_sub(choice as u64);
        Self(core::array::from_fn(|i| (a.0[i] & !m) | (b.0[i] & m)))
    }
}

#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}
impl Point {
    fn identity() -> Self {
        Self {
            x: Fe::ZERO,
            y: Fe::ONE,
            z: Fe::ONE,
            t: Fe::ZERO,
        }
    }
    fn base() -> Self {
        Self {
            x: BX,
            y: BY,
            z: Fe::ONE,
            t: BX.mul(BY),
        }
    }
    fn decode(bytes: &[u8; 32]) -> Option<Self> {
        let sign = bytes[31] >> 7;
        let mut yb = *bytes;
        yb[31] &= 0x7f;
        let y = Fe::from_bytes(&yb)?;
        let y2 = y.square();
        let u = y2.sub(Fe::ONE);
        let v = D.mul(y2).add(Fe::ONE);
        let mut e = [0xff; 32];
        e[0] = 0xfe;
        e[31] = 0x0f; // (p+3)/8 = 2^252-2
        let mut x = u.mul(v.invert()).pow(&e);
        if x.square().mul(v).sub(u).is_zero() == 0 {
            x = x.mul(SQRT_M1)
        }
        if x.square().mul(v).sub(u).is_zero() == 0 {
            return None;
        }
        if x.is_zero() == 1 && sign == 1 {
            return None;
        }
        if x.is_negative() != sign {
            x = x.neg()
        }
        Some(Self {
            x,
            y,
            z: Fe::ONE,
            t: x.mul(y),
        })
    }
    fn add(self, q: Self) -> Self {
        let a = self.y.sub(self.x).mul(q.y.sub(q.x));
        let b = self.y.add(self.x).mul(q.y.add(q.x));
        let c = self.t.mul(q.t).mul(D.add(D));
        let d = self.z.mul(q.z).add(self.z.mul(q.z));
        let e = b.sub(a);
        let f = d.sub(c);
        let g = d.add(c);
        let h = b.add(a);
        Self {
            x: e.mul(f),
            y: g.mul(h),
            z: f.mul(g),
            t: e.mul(h),
        }
    }
    fn double(self) -> Self {
        let a = self.x.square();
        let b = self.y.square();
        let c = self.z.square().add(self.z.square());
        let d = a.neg();
        let e = self.x.add(self.y).square().sub(a).sub(b);
        let g = d.add(b);
        let f = g.sub(c);
        let h = d.sub(b);
        Self {
            x: e.mul(f),
            y: g.mul(h),
            z: f.mul(g),
            t: e.mul(h),
        }
    }
    fn select(a: Self, b: Self, c: u8) -> Self {
        Self {
            x: Fe::select(a.x, b.x, c),
            y: Fe::select(a.y, b.y, c),
            z: Fe::select(a.z, b.z, c),
            t: Fe::select(a.t, b.t, c),
        }
    }
    fn scalar_mul(self, s: &[u8; 32]) -> Self {
        let mut r = Self::identity();
        for i in (0..256).rev() {
            r = r.double();
            let t = r.add(self);
            r = Self::select(r, t, (s[i / 8] >> (i % 8)) & 1)
        }
        r
    }
    fn equal(self, q: Self) -> u8 {
        self.x.mul(q.z).sub(q.x.mul(self.z)).is_zero()
            & self.y.mul(q.z).sub(q.y.mul(self.z)).is_zero()
    }
    #[cfg(test)]
    fn encode(self) -> [u8; 32] {
        let iz = self.z.invert();
        let x = self.x.mul(iz);
        let y = self.y.mul(iz);
        let mut out = y.to_bytes();
        out[31] |= x.is_negative() << 7;
        out
    }
    fn small_order(self) -> bool {
        self.double().double().double().equal(Self::identity()) == 1
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> u8 {
    if a.len() != b.len() {
        return 0;
    }
    let mut x = 0u8;
    for i in 0..a.len() {
        x |= a[i] ^ b[i]
    }
    (((x as u16).wrapping_sub(1) >> 8) & 1) as u8
}
fn scalar_canonical(s: &[u8; 32]) -> bool {
    let mut borrow = 0u128;
    for i in 0..4 {
        let x = u64::from_le_bytes(s[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8])) as u128;
        let sub = L[i] as u128 + borrow;
        borrow = (x < sub) as u128;
    }
    borrow == 1
}
fn scalar_reduce(input: &[u8; 64]) -> [u8; 32] {
    let mut r = [0u64; 4];
    for bit in (0..512).rev() {
        let incoming = ((input[bit / 8] >> (bit % 8)) & 1) as u64;
        let mut carry = incoming;
        for word in &mut r {
            let next = *word >> 63;
            *word = (*word << 1) | carry;
            carry = next
        }
        let ge = {
            let mut borrow = 0u128;
            for i in 0..4 {
                let x = r[i] as u128;
                let sub = L[i] as u128 + borrow;
                borrow = (x < sub) as u128;
            }
            borrow == 0
        };
        let m = 0u64.wrapping_sub(ge as u64);
        let mut borrow = 0u128;
        for i in 0..4 {
            let sub = (L[i] & m) as u128 + borrow;
            let x = r[i] as u128;
            r[i] = x.wrapping_sub(sub) as u64;
            borrow = (x < sub) as u128;
        }
    }
    let mut o = [0u8; 32];
    for i in 0..4 {
        o[i * 8..i * 8 + 8].copy_from_slice(&r[i].to_le_bytes())
    }
    o
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ed25519Error {
    MalformedKey,
    MalformedSignature,
    InvalidSignature,
}
pub fn verify_ed25519(
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), Ed25519Error> {
    let a = Point::decode(public_key).ok_or(Ed25519Error::MalformedKey)?;
    let r_bytes: &[u8; 32] = signature[..32]
        .try_into()
        .map_err(|_| Ed25519Error::MalformedSignature)?;
    let r = Point::decode(r_bytes).ok_or(Ed25519Error::MalformedSignature)?;
    let s: &[u8; 32] = signature[32..]
        .try_into()
        .map_err(|_| Ed25519Error::MalformedSignature)?;
    if !scalar_canonical(s) {
        return Err(Ed25519Error::MalformedSignature);
    }
    if a.small_order() || r.small_order() {
        return Err(Ed25519Error::MalformedSignature);
    }
    let mut h = Sha512::new();
    h.update(r_bytes);
    h.update(public_key);
    h.update(message);
    let k = scalar_reduce(&h.finalize());
    let lhs = Point::base().scalar_mul(s);
    let rhs = r.add(a.scalar_mul(&k));
    if lhs.equal(rhs) == 1 {
        Ok(())
    } else {
        Err(Ed25519Error::InvalidSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex<const N: usize>(s: &str) -> [u8; N] {
        let mut o = [0; N];
        for i in 0..N {
            o[i] = (n(s.as_bytes()[2 * i]) << 4) | n(s.as_bytes()[2 * i + 1])
        }
        o
    }
    fn n(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            _ => c - b'a' + 10,
        }
    }
    #[test]
    fn rfc8032_vector1() {
        let pk = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let sig = hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        assert_eq!(verify_ed25519(&pk, b"", &sig), Ok(()));
    }
    #[test]
    fn rejects_modified() {
        let pk = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let mut sig = hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        sig[10] ^= 1;
        assert!(verify_ed25519(&pk, b"", &sig).is_err());
    }

    #[test]
    fn reduces_rfc8032_challenge() {
        let input = hex(
            "2771062b6b536fe7ffbdda0320c3827b035df10d284df3f08222f04dbca7a4c20ef15bdc988a22c7207411377c33f2ac09b1e86a046234283768ee7ba03c0e9f",
        );
        let expected = hex("86eabc8e4c96193d290504e7c600df6cf8d8256131ec2c138a3e7e162e525404");
        assert_eq!(scalar_reduce(&input), expected);
    }

    #[test]
    fn basepoint_encoding_and_scalar_one() {
        let encoded = hex("5866666666666666666666666666666666666666666666666666666666666666");
        assert_eq!(Point::base().encode(), encoded);
        let mut one = [0u8; 32];
        one[0] = 1;
        assert_eq!(Point::base().scalar_mul(&one).encode(), encoded);
        assert_eq!(Point::decode(&encoded).unwrap().encode(), encoded);
        let secret_scalar = hex("307c83864f2833cb427a2ef1c00a013cfdff2768d980c0a3a520f006904de94f");
        let public_key = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        assert_eq!(
            Point::base().scalar_mul(&secret_scalar).encode(),
            public_key
        );
    }

    #[test]
    fn rfc8032_equation_points() {
        let a = Point::decode(&hex(
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        ))
        .unwrap();
        let r = Point::decode(&hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
        ))
        .unwrap();
        let s = hex("5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
        let k = hex("86eabc8e4c96193d290504e7c600df6cf8d8256131ec2c138a3e7e162e525404");
        let expected = hex("fe6c8a99af92f9e5f16dc1c5366f9c2cd9c5e890f27544e542cfa5d6dddd9427");
        assert_eq!(Point::base().scalar_mul(&s).encode(), expected);
        assert_eq!(r.add(a.scalar_mul(&k)).encode(), expected);
    }
}
