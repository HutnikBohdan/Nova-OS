//! Portable Q8 inference kernels. Architecture-specific SIMD can wrap these
//! routines while retaining bit-for-bit test vectors.

use crate::{Error, Result};

pub const QK8: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Q8Block {
    pub scale: f32,
    pub values: [i8; QK8],
}

impl Q8Block {
    pub const ZERO: Self = Self {
        scale: 0.0,
        values: [0; QK8],
    };

    pub fn quantize(input: &[f32; QK8]) -> Self {
        let mut max_abs = 0.0f32;
        for value in input {
            let absolute = abs(*value);
            if absolute > max_abs {
                max_abs = absolute;
            }
        }
        if max_abs == 0.0 {
            return Self::ZERO;
        }
        let scale = max_abs / 127.0;
        let mut values = [0i8; QK8];
        for (slot, value) in values.iter_mut().zip(input.iter().copied()) {
            let scaled = value / scale;
            let rounded = if scaled >= 0.0 {
                (scaled + 0.5) as i32
            } else {
                (scaled - 0.5) as i32
            };
            *slot = rounded.clamp(-127, 127) as i8;
        }
        Self { scale, values }
    }

    pub fn dot(self, other: Self) -> f32 {
        let mut sum = 0i32;
        for index in 0..QK8 {
            sum += self.values[index] as i32 * other.values[index] as i32;
        }
        sum as f32 * self.scale * other.scale
    }

    pub fn dot_f32(self, other: &[f32; QK8]) -> f32 {
        let mut sum = 0.0f32;
        for (quantized, value) in self.values.iter().zip(other) {
            sum += *quantized as f32 * *value;
        }
        sum * self.scale
    }

    /// Decodes the GGML Q8_0 disk representation (f16 scale + 32 i8 values).
    pub fn from_ggml(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 + QK8 {
            return Err(Error::UnexpectedEof);
        }
        let scale = f16_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]));
        let mut values = [0i8; QK8];
        for (out, input) in values.iter_mut().zip(&bytes[2..2 + QK8]) {
            *out = *input as i8;
        }
        Ok(Self { scale, values })
    }
}

/// Row-major matrix/vector multiplication. `columns` must be a multiple of 32.
pub fn matvec_q8(
    matrix: &[Q8Block],
    rows: usize,
    columns: usize,
    vector: &[f32],
    output: &mut [f32],
) -> Result<()> {
    if columns == 0 || !columns.is_multiple_of(QK8) || vector.len() < columns || output.len() < rows
    {
        return Err(Error::InvalidConfiguration);
    }
    let blocks_per_row = columns / QK8;
    let required = rows
        .checked_mul(blocks_per_row)
        .ok_or(Error::IntegerOverflow)?;
    if matrix.len() < required {
        return Err(Error::UnexpectedEof);
    }
    for row in 0..rows {
        let mut sum = 0.0f32;
        for block_index in 0..blocks_per_row {
            let start = block_index * QK8;
            let chunk: &[f32; QK8] = vector[start..start + QK8].try_into().unwrap();
            sum += matrix[row * blocks_per_row + block_index].dot_f32(chunk);
        }
        output[row] = sum;
    }
    Ok(())
}

pub fn f16_to_f32(value: u16) -> f32 {
    let sign = ((value as u32) & 0x8000) << 16;
    let exponent = (value >> 10) & 0x1f;
    let fraction = (value & 0x03ff) as u32;
    let bits = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let mut frac = fraction;
            let mut shift = 0u32;
            while frac & 0x400 == 0 {
                frac <<= 1;
                shift += 1;
            }
            sign | ((127 - 15 - shift) << 23) | ((frac & 0x3ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | (((exponent as u32) + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(bits)
}

const fn abs(value: f32) -> f32 {
    if value < 0.0 {
        -value
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_conversion_known_values() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0), 0.0);
    }
    #[test]
    fn quantization_error_is_bounded() {
        let mut x = [0.0; QK8];
        for (i, v) in x.iter_mut().enumerate() {
            *v = i as f32 - 15.5;
        }
        let q = Q8Block::quantize(&x);
        for (quantized, original) in q.values.iter().zip(x) {
            assert!((*quantized as f32 * q.scale - original).abs() <= q.scale * 0.51);
        }
    }
    #[test]
    fn q8_dot_and_matvec() {
        let ones = [1.0; QK8];
        let twos = [2.0; QK8];
        let q1 = Q8Block::quantize(&ones);
        let q2 = Q8Block::quantize(&twos);
        assert!((q1.dot(q2) - 64.0).abs() < 0.01);
        let mut output = [0.0; 2];
        matvec_q8(&[q1, q2], 2, QK8, &twos, &mut output).unwrap();
        assert!((output[0] - 64.0).abs() < 0.01);
        assert!((output[1] - 128.0).abs() < 0.01);
    }
    #[test]
    fn parses_ggml_q8_block() {
        let mut bytes = [0u8; 34];
        bytes[..2].copy_from_slice(&0x3c00u16.to_le_bytes());
        for i in 0..32 {
            bytes[i + 2] = i as u8;
        }
        let q = Q8Block::from_ggml(&bytes).unwrap();
        assert_eq!(q.scale, 1.0);
        assert_eq!(q.values[31], 31);
    }
}
