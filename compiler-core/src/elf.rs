use crate::{CodeImage, CompileError, ErrorCode, Span};

pub const ELF_BASE: u64 = 0x0040_0000;
pub const ELF_ENTRY: u64 = ELF_BASE + 0x1000;
const HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const CODE_OFFSET: usize = 0x1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfImage {
    pub len: usize,
    pub entry: u64,
    pub code_offset: usize,
    pub code_len: usize,
}

pub fn write_elf(output: &mut [u8], code: &CodeImage<'_, '_>) -> Result<ElfImage, CompileError> {
    let len = CODE_OFFSET
        .checked_add(code.bytes.len())
        .ok_or_else(small)?;
    if output.len() < len {
        return Err(small());
    }
    output[..len].fill(0);
    output[0..4].copy_from_slice(b"\x7fELF");
    output[4] = 2;
    output[5] = 1;
    output[6] = 1;
    put16(output, 16, 2);
    put16(output, 18, 62);
    put32(output, 20, 1);
    put64(output, 24, ELF_ENTRY + code.entry_offset as u64);
    put64(output, 32, HEADER_SIZE as u64);
    put64(output, 40, 0);
    put32(output, 48, 0);
    put16(output, 52, HEADER_SIZE as u16);
    put16(output, 54, PROGRAM_HEADER_SIZE as u16);
    put16(output, 56, 1);
    put16(output, 58, 0);
    put16(output, 60, 0);
    put16(output, 62, 0);
    let ph = HEADER_SIZE;
    put32(output, ph, 1);
    put32(output, ph + 4, 5);
    put64(output, ph + 8, 0);
    put64(output, ph + 16, ELF_BASE);
    put64(output, ph + 24, ELF_BASE);
    put64(output, ph + 32, len as u64);
    put64(output, ph + 40, len as u64);
    put64(output, ph + 48, 0x1000);
    output[CODE_OFFSET..len].copy_from_slice(code.bytes);
    Ok(ElfImage {
        len,
        entry: ELF_ENTRY + code.entry_offset as u64,
        code_offset: CODE_OFFSET,
        code_len: code.bytes.len(),
    })
}
fn put16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(out: &mut [u8], at: usize, value: u64) {
    out[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn small() -> CompileError {
    CompileError::new(
        ErrorCode::OutputTooSmall,
        Span::default(),
        "замалий вихідний буфер ELF",
    )
}
