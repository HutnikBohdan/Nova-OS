//! Zero-copy GGUF v3 metadata and tensor-directory parser.

use crate::{Error, Result};

pub const MAGIC: [u8; 4] = *b"GGUF";
pub const VERSION: u32 = 3;
pub const DEFAULT_ALIGNMENT: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ValueType {
    U8 = 0,
    I8 = 1,
    U16 = 2,
    I16 = 3,
    U32 = 4,
    I32 = 5,
    F32 = 6,
    Bool = 7,
    String = 8,
    Array = 9,
    U64 = 10,
    I64 = 11,
    F64 = 12,
}

impl ValueType {
    fn from_raw(value: u32) -> Result<Self> {
        Ok(match value {
            0 => Self::U8,
            1 => Self::I8,
            2 => Self::U16,
            3 => Self::I16,
            4 => Self::U32,
            5 => Self::I32,
            6 => Self::F32,
            7 => Self::Bool,
            8 => Self::String,
            9 => Self::Array,
            10 => Self::U64,
            11 => Self::I64,
            12 => Self::F64,
            other => return Err(Error::UnsupportedValueType(other)),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar<'a> {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    String(&'a str),
    U64(u64),
    I64(i64),
    F64(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    Scalar(Scalar<'a>),
    Array(ArrayRef<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrayRef<'a> {
    pub element_type: ValueType,
    pub len: u64,
    bytes: &'a [u8],
}

impl<'a> ArrayRef<'a> {
    pub fn iter(self) -> ArrayIter<'a> {
        ArrayIter {
            cursor: Cursor::new(self.bytes),
            ty: self.element_type,
            remaining: self.len,
        }
    }
}

pub struct ArrayIter<'a> {
    cursor: Cursor<'a>,
    ty: ValueType,
    remaining: u64,
}

impl<'a> Iterator for ArrayIter<'a> {
    type Item = Result<Scalar<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(read_scalar(&mut self.cursor, self.ty))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metadata<'a> {
    pub key: &'a str,
    pub value: Value<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tensor<'a> {
    pub name: &'a str,
    pub dimensions: &'a [u8],
    pub dimension_count: u32,
    pub ggml_type: u32,
    pub relative_offset: u64,
}

impl Tensor<'_> {
    pub fn dimension(&self, index: usize) -> Option<u64> {
        if index >= self.dimension_count as usize {
            return None;
        }
        let start = index.checked_mul(8)?;
        let bytes: [u8; 8] = self.dimensions.get(start..start + 8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    }

    pub fn element_count(&self) -> Result<u64> {
        let mut count = 1u64;
        for index in 0..self.dimension_count as usize {
            count = count
                .checked_mul(self.dimension(index).ok_or(Error::InvalidTensor)?)
                .ok_or(Error::IntegerOverflow)?;
        }
        Ok(count)
    }

    /// Validates a GGUF tensor's rank and dimensions without allocating.
    /// GGUF stores dimensions in fastest-changing-first order.
    pub fn validate_shape(&self, expected: &[u64]) -> Result<()> {
        if self.dimension_count as usize != expected.len() {
            return Err(Error::ShapeMismatch);
        }
        for (index, wanted) in expected.iter().copied().enumerate() {
            if self.dimension(index) != Some(wanted) {
                return Err(Error::ShapeMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Gguf<'a> {
    bytes: &'a [u8],
    tensor_count: u64,
    metadata_count: u64,
    metadata_start: usize,
    tensor_start: usize,
    data_start: usize,
    alignment: usize,
}

impl<'a> Gguf<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        if cursor.take(4)? != MAGIC {
            return Err(Error::InvalidMagic);
        }
        let version = cursor.u32()?;
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let tensor_count = cursor.u64()?;
        let metadata_count = cursor.u64()?;
        let metadata_start = cursor.position;
        let mut alignment = DEFAULT_ALIGNMENT;
        for _ in 0..metadata_count {
            let key = cursor.string()?;
            let ty = ValueType::from_raw(cursor.u32()?)?;
            if key == "general.alignment" && ty == ValueType::U32 {
                alignment = cursor.u32()? as usize;
                if alignment == 0 || !alignment.is_power_of_two() {
                    return Err(Error::InvalidConfiguration);
                }
            } else {
                skip_value(&mut cursor, ty)?;
            }
        }
        let tensor_start = cursor.position;
        for _ in 0..tensor_count {
            cursor.string()?;
            let dimensions = cursor.u32()? as usize;
            cursor.skip(dimensions.checked_mul(8).ok_or(Error::IntegerOverflow)?)?;
            cursor.u32()?;
            cursor.u64()?;
        }
        let data_start = align_up(cursor.position, alignment)?;
        if data_start > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            tensor_count,
            metadata_count,
            metadata_start,
            tensor_start,
            data_start,
            alignment,
        })
    }

    pub const fn tensor_count(&self) -> u64 {
        self.tensor_count
    }
    pub const fn metadata_count(&self) -> u64 {
        self.metadata_count
    }
    pub const fn data_offset(&self) -> usize {
        self.data_start
    }
    pub const fn alignment(&self) -> usize {
        self.alignment
    }

    pub fn metadata(&self) -> MetadataIter<'a> {
        MetadataIter {
            cursor: Cursor::at(self.bytes, self.metadata_start),
            remaining: self.metadata_count,
        }
    }

    pub fn tensors(&self) -> TensorIter<'a> {
        TensorIter {
            cursor: Cursor::at(self.bytes, self.tensor_start),
            remaining: self.tensor_count,
        }
    }

    pub fn find_metadata(&self, needle: &str) -> Result<Option<Value<'a>>> {
        for item in self.metadata() {
            let item = item?;
            if item.key == needle {
                return Ok(Some(item.value));
            }
        }
        Ok(None)
    }

    pub fn find_tensor(&self, needle: &str) -> Result<Option<Tensor<'a>>> {
        for tensor in self.tensors() {
            let tensor = tensor?;
            if tensor.name == needle {
                return Ok(Some(tensor));
            }
        }
        Ok(None)
    }

    /// Looks up a required tensor and validates its shape and GGML storage type.
    pub fn require_tensor(
        &self,
        name: &str,
        expected_shape: &[u64],
        expected_ggml_type: u32,
    ) -> Result<Tensor<'a>> {
        let tensor = self.find_tensor(name)?.ok_or(Error::MissingTensor)?;
        tensor.validate_shape(expected_shape)?;
        if tensor.ggml_type != expected_ggml_type {
            return Err(Error::InvalidTensor);
        }
        Ok(tensor)
    }

    pub fn tensor_data(&self, tensor: Tensor<'a>, byte_len: usize) -> Result<&'a [u8]> {
        let relative =
            usize::try_from(tensor.relative_offset).map_err(|_| Error::IntegerOverflow)?;
        let start = self
            .data_start
            .checked_add(relative)
            .ok_or(Error::IntegerOverflow)?;
        let end = start.checked_add(byte_len).ok_or(Error::IntegerOverflow)?;
        self.bytes.get(start..end).ok_or(Error::UnexpectedEof)
    }
}

pub struct MetadataIter<'a> {
    cursor: Cursor<'a>,
    remaining: u64,
}

impl<'a> Iterator for MetadataIter<'a> {
    type Item = Result<Metadata<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some((|| {
            let key = self.cursor.string()?;
            let ty = ValueType::from_raw(self.cursor.u32()?)?;
            let value = read_value(&mut self.cursor, ty)?;
            Ok(Metadata { key, value })
        })())
    }
}

pub struct TensorIter<'a> {
    cursor: Cursor<'a>,
    remaining: u64,
}

impl<'a> Iterator for TensorIter<'a> {
    type Item = Result<Tensor<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some((|| {
            let name = self.cursor.string()?;
            let dimension_count = self.cursor.u32()?;
            let byte_count = (dimension_count as usize)
                .checked_mul(8)
                .ok_or(Error::IntegerOverflow)?;
            let dimensions = self.cursor.take(byte_count)?;
            let ggml_type = self.cursor.u32()?;
            let relative_offset = self.cursor.u64()?;
            Ok(Tensor {
                name,
                dimensions,
                dimension_count,
                ggml_type,
                relative_offset,
            })
        })())
    }
}

fn read_value<'a>(cursor: &mut Cursor<'a>, ty: ValueType) -> Result<Value<'a>> {
    if ty == ValueType::Array {
        let element_type = ValueType::from_raw(cursor.u32()?)?;
        if element_type == ValueType::Array {
            return Err(Error::UnsupportedValueType(ValueType::Array as u32));
        }
        let len = cursor.u64()?;
        let start = cursor.position;
        for _ in 0..len {
            skip_value(cursor, element_type)?;
        }
        Ok(Value::Array(ArrayRef {
            element_type,
            len,
            bytes: &cursor.bytes[start..cursor.position],
        }))
    } else {
        Ok(Value::Scalar(read_scalar(cursor, ty)?))
    }
}

fn read_scalar<'a>(cursor: &mut Cursor<'a>, ty: ValueType) -> Result<Scalar<'a>> {
    Ok(match ty {
        ValueType::U8 => Scalar::U8(cursor.u8()?),
        ValueType::I8 => Scalar::I8(cursor.u8()? as i8),
        ValueType::U16 => Scalar::U16(cursor.u16()?),
        ValueType::I16 => Scalar::I16(cursor.u16()? as i16),
        ValueType::U32 => Scalar::U32(cursor.u32()?),
        ValueType::I32 => Scalar::I32(cursor.u32()? as i32),
        ValueType::F32 => Scalar::F32(f32::from_bits(cursor.u32()?)),
        ValueType::Bool => match cursor.u8()? {
            0 => Scalar::Bool(false),
            1 => Scalar::Bool(true),
            other => return Err(Error::InvalidBool(other)),
        },
        ValueType::String => Scalar::String(cursor.string()?),
        ValueType::U64 => Scalar::U64(cursor.u64()?),
        ValueType::I64 => Scalar::I64(cursor.u64()? as i64),
        ValueType::F64 => Scalar::F64(f64::from_bits(cursor.u64()?)),
        ValueType::Array => return Err(Error::UnsupportedValueType(ValueType::Array as u32)),
    })
}

fn skip_value(cursor: &mut Cursor<'_>, ty: ValueType) -> Result<()> {
    if ty == ValueType::Array {
        let element = ValueType::from_raw(cursor.u32()?)?;
        if element == ValueType::Array {
            return Err(Error::UnsupportedValueType(ValueType::Array as u32));
        }
        let len = cursor.u64()?;
        for _ in 0..len {
            skip_value(cursor, element)?;
        }
        return Ok(());
    }
    match ty {
        ValueType::U8 | ValueType::I8 | ValueType::Bool => cursor.skip(1),
        ValueType::U16 | ValueType::I16 => cursor.skip(2),
        ValueType::U32 | ValueType::I32 | ValueType::F32 => cursor.skip(4),
        ValueType::U64 | ValueType::I64 | ValueType::F64 => cursor.skip(8),
        ValueType::String => {
            cursor.string()?;
            Ok(())
        }
        ValueType::Array => unreachable!(),
    }
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .ok_or(Error::IntegerOverflow)
}

#[derive(Clone, Copy, Debug)]
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    const fn at(bytes: &'a [u8], position: usize) -> Self {
        Self { bytes, position }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(Error::IntegerOverflow)?;
        let out = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::UnexpectedEof)?;
        self.position = end;
        Ok(out)
    }
    fn skip(&mut self, len: usize) -> Result<()> {
        self.take(len).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<&'a str> {
        let len = usize::try_from(self.u64()?).map_err(|_| Error::IntegerOverflow)?;
        core::str::from_utf8(self.take(len)?).map_err(|_| Error::InvalidUtf8)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GGUF");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes());
        b.extend_from_slice(&3u64.to_le_bytes());
        string(&mut b, "general.alignment");
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&16u32.to_le_bytes());
        string(&mut b, "general.name");
        b.extend_from_slice(&8u32.to_le_bytes());
        string(&mut b, "Nova-1B");
        string(&mut b, "tokenizer.ids");
        b.extend_from_slice(&9u32.to_le_bytes());
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&3u64.to_le_bytes());
        for n in [1u32, 2, 3] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        string(&mut b, "blk.0.weight");
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&32u64.to_le_bytes());
        b.extend_from_slice(&4u64.to_le_bytes());
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        while b.len() % 16 != 0 {
            b.push(0);
        }
        b.extend_from_slice(&[0xA5; 32]);
        b
    }

    #[test]
    fn parses_header_metadata_and_tensor() {
        let bytes = fixture();
        let file = Gguf::parse(&bytes).unwrap();
        assert_eq!(file.tensor_count(), 1);
        assert_eq!(file.metadata_count(), 3);
        assert_eq!(file.alignment(), 16);
        assert_eq!(
            file.find_metadata("general.name").unwrap(),
            Some(Value::Scalar(Scalar::String("Nova-1B")))
        );
        let t = file.find_tensor("blk.0.weight").unwrap().unwrap();
        assert_eq!(t.dimension(0), Some(32));
        assert_eq!(t.dimension(1), Some(4));
        assert_eq!(t.element_count().unwrap(), 128);
        assert_eq!(file.tensor_data(t, 32).unwrap(), &[0xA5; 32]);
    }
    #[test]
    fn iterates_array_without_allocation() {
        let bytes = fixture();
        let file = Gguf::parse(&bytes).unwrap();
        let Value::Array(a) = file.find_metadata("tokenizer.ids").unwrap().unwrap() else {
            panic!()
        };
        let got: Vec<u32> = a
            .iter()
            .map(|v| match v.unwrap() {
                Scalar::U32(x) => x,
                _ => 0,
            })
            .collect();
        assert_eq!(got, [1, 2, 3]);
    }
    #[test]
    fn rejects_truncated_and_wrong_version() {
        assert!(matches!(Gguf::parse(b"bad!"), Err(Error::InvalidMagic)));
        let mut h = fixture();
        h[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(Gguf::parse(&h), Err(Error::UnsupportedVersion(2))));
        let f = fixture();
        assert!(matches!(Gguf::parse(&f[..30]), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn required_tensor_validates_shape_and_type() {
        let bytes = fixture();
        let file = Gguf::parse(&bytes).unwrap();
        let tensor = file.require_tensor("blk.0.weight", &[32, 4], 8).unwrap();
        assert_eq!(tensor.element_count().unwrap(), 128);
        assert_eq!(
            file.require_tensor("blk.0.weight", &[4, 32], 8),
            Err(Error::ShapeMismatch)
        );
        assert_eq!(
            file.require_tensor("blk.0.weight", &[32, 4], 2),
            Err(Error::InvalidTensor)
        );
        assert_eq!(
            file.require_tensor("missing", &[32], 8),
            Err(Error::MissingTensor)
        );
    }
}
