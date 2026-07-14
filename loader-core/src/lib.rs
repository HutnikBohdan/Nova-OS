#![no_std]
#![forbid(unsafe_code)]

//! Strict, allocation-free ELF64 process loader building blocks.

use core::cmp::min;

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const SECTION_HEADER_SIZE: usize = 64;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_TLS: u32 = 7;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;
const EV_CURRENT: u32 = 1;
const R_X86_64_64: u32 = 1;
const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;
const R_X86_64_RELATIVE: u32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedAbi,
    UnsupportedType,
    UnsupportedMachine,
    UnsupportedVersion,
    BadHeaderSize,
    BadProgramHeaderSize,
    BadSectionHeaderSize,
    IntegerOverflow,
    BadAlignment,
    BadSegment,
    WritableExecutable,
    TooManySegments,
    MissingLoadSegment,
    EntryNotExecutable,
    MissingDynamicTable,
    BadDynamicTable,
    MissingStringTable,
    MissingSymbolTable,
    BadString,
    BadSymbol,
    TooManyRelocations,
    UnsupportedRelocation,
    UnresolvedSymbol,
    RelocationOutOfRange,
    BufferTooSmall,
    InvalidAddressRange,
    NoAslrPlacement,
    BadTlsImage,
    SignatureRejected,
}

fn bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], Error> {
    let end = offset.checked_add(N).ok_or(Error::IntegerOverflow)?;
    data.get(offset..end)
        .ok_or(Error::Truncated)?
        .try_into()
        .map_err(|_| Error::Truncated)
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(bytes(data, offset)?))
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(bytes(data, offset)?))
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(bytes(data, offset)?))
}

fn i64_at(data: &[u8], offset: usize) -> Result<i64, Error> {
    Ok(i64::from_le_bytes(bytes(data, offset)?))
}

fn range(offset: u64, length: u64, total: usize) -> Result<core::ops::Range<usize>, Error> {
    let start = usize::try_from(offset).map_err(|_| Error::IntegerOverflow)?;
    let len = usize::try_from(length).map_err(|_| Error::IntegerOverflow)?;
    let end = start.checked_add(len).ok_or(Error::IntegerOverflow)?;
    if end > total {
        return Err(Error::Truncated);
    }
    Ok(start..end)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfKind {
    Executable,
    PositionIndependent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElfHeader {
    pub kind: ElfKind,
    pub entry: u64,
    pub program_offset: u64,
    pub program_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Permissions(u8);

impl Permissions {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXECUTE: Self = Self(4);
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for Permissions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
    pub permissions: Permissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlsTemplate {
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Program {
    pub header: ElfHeader,
    pub load_count: usize,
    pub loads: [Option<LoadSegment>; 16],
    pub dynamic: Option<(u64, u64)>,
    pub tls: Option<TlsTemplate>,
    pub lowest_vaddr: u64,
    pub highest_vaddr: u64,
}

impl Program {
    pub fn loads(&self) -> impl Iterator<Item = &LoadSegment> {
        self.loads[..self.load_count].iter().flatten()
    }

    pub fn image_span(&self) -> u64 {
        self.highest_vaddr - self.lowest_vaddr
    }

    pub fn runtime_entry(&self, load_bias: u64) -> Result<u64, Error> {
        self.header
            .entry
            .checked_add(load_bias)
            .ok_or(Error::IntegerOverflow)
    }
}

pub fn parse_elf64(image: &[u8]) -> Result<Program, Error> {
    if image.len() < ELF_HEADER_SIZE {
        return Err(Error::Truncated);
    }
    if image[0..4] != *b"\x7fELF" {
        return Err(Error::BadMagic);
    }
    if image[4] != 2 {
        return Err(Error::UnsupportedClass);
    }
    if image[5] != 1 {
        return Err(Error::UnsupportedEndian);
    }
    if image[6] != 1 || u32_at(image, 20)? != EV_CURRENT {
        return Err(Error::UnsupportedVersion);
    }
    if image[7] != 0 || image[8] != 0 {
        return Err(Error::UnsupportedAbi);
    }
    let kind = match u16_at(image, 16)? {
        ET_EXEC => ElfKind::Executable,
        ET_DYN => ElfKind::PositionIndependent,
        _ => return Err(Error::UnsupportedType),
    };
    if u16_at(image, 18)? != EM_X86_64 {
        return Err(Error::UnsupportedMachine);
    }
    if u16_at(image, 52)? as usize != ELF_HEADER_SIZE {
        return Err(Error::BadHeaderSize);
    }
    if u16_at(image, 54)? as usize != PROGRAM_HEADER_SIZE {
        return Err(Error::BadProgramHeaderSize);
    }
    let shnum = u16_at(image, 60)?;
    if shnum != 0 && u16_at(image, 58)? as usize != SECTION_HEADER_SIZE {
        return Err(Error::BadSectionHeaderSize);
    }
    let phoff = u64_at(image, 32)?;
    let phnum = u16_at(image, 56)?;
    let ph_size = u64::from(phnum)
        .checked_mul(PROGRAM_HEADER_SIZE as u64)
        .ok_or(Error::IntegerOverflow)?;
    range(phoff, ph_size, image.len())?;

    let header = ElfHeader {
        kind,
        entry: u64_at(image, 24)?,
        program_offset: phoff,
        program_count: phnum,
    };
    let mut program = Program {
        header,
        load_count: 0,
        loads: [None; 16],
        dynamic: None,
        tls: None,
        lowest_vaddr: u64::MAX,
        highest_vaddr: 0,
    };
    for index in 0..usize::from(phnum) {
        let off = usize::try_from(phoff)
            .map_err(|_| Error::IntegerOverflow)?
            .checked_add(
                index
                    .checked_mul(PROGRAM_HEADER_SIZE)
                    .ok_or(Error::IntegerOverflow)?,
            )
            .ok_or(Error::IntegerOverflow)?;
        let typ = u32_at(image, off)?;
        let flags = u32_at(image, off + 4)?;
        let file_offset = u64_at(image, off + 8)?;
        let vaddr = u64_at(image, off + 16)?;
        let file_size = u64_at(image, off + 32)?;
        let memory_size = u64_at(image, off + 40)?;
        let align = u64_at(image, off + 48)?;
        if typ == PT_LOAD || typ == PT_DYNAMIC || typ == PT_TLS {
            range(file_offset, file_size, image.len())?;
            if file_size > memory_size {
                return Err(Error::BadSegment);
            }
        }
        if typ == PT_LOAD {
            if program.load_count == program.loads.len() {
                return Err(Error::TooManySegments);
            }
            if flags & !(PF_R | PF_W | PF_X) != 0 {
                return Err(Error::BadSegment);
            }
            if align > 1 && (!align.is_power_of_two() || file_offset % align != vaddr % align) {
                return Err(Error::BadAlignment);
            }
            if flags & PF_W != 0 && flags & PF_X != 0 {
                return Err(Error::WritableExecutable);
            }
            let end = vaddr
                .checked_add(memory_size)
                .ok_or(Error::IntegerOverflow)?;
            if memory_size == 0 || end <= vaddr {
                return Err(Error::BadSegment);
            }
            let mut permissions = Permissions(0);
            if flags & PF_R != 0 {
                permissions = permissions | Permissions::READ;
            }
            if flags & PF_W != 0 {
                permissions = permissions | Permissions::WRITE;
            }
            if flags & PF_X != 0 {
                permissions = permissions | Permissions::EXECUTE;
            }
            program.loads[program.load_count] = Some(LoadSegment {
                file_offset,
                virtual_address: vaddr,
                file_size,
                memory_size,
                alignment: align,
                permissions,
            });
            program.load_count += 1;
            program.lowest_vaddr = min(program.lowest_vaddr, vaddr);
            program.highest_vaddr = program.highest_vaddr.max(end);
        } else if typ == PT_DYNAMIC {
            if program.dynamic.replace((vaddr, file_size)).is_some() {
                return Err(Error::BadDynamicTable);
            }
        } else if typ == PT_TLS {
            if program
                .tls
                .replace(TlsTemplate {
                    file_offset,
                    file_size,
                    memory_size,
                    alignment: align,
                })
                .is_some()
            {
                return Err(Error::BadTlsImage);
            }
            if align > 1 && (!align.is_power_of_two() || file_offset % align != vaddr % align) {
                return Err(Error::BadAlignment);
            }
        }
    }
    if program.load_count == 0 {
        return Err(Error::MissingLoadSegment);
    }
    for (index, left) in program.loads().enumerate() {
        let left_end = left
            .virtual_address
            .checked_add(left.memory_size)
            .ok_or(Error::IntegerOverflow)?;
        for right in program.loads().skip(index + 1) {
            let right_end = right
                .virtual_address
                .checked_add(right.memory_size)
                .ok_or(Error::IntegerOverflow)?;
            if left.virtual_address < right_end && right.virtual_address < left_end {
                return Err(Error::BadSegment);
            }
        }
    }
    let entry_ok = program.loads().any(|segment| {
        segment.permissions.contains(Permissions::EXECUTE)
            && header.entry >= segment.virtual_address
            && header.entry < segment.virtual_address.saturating_add(segment.memory_size)
    });
    if !entry_ok {
        return Err(Error::EntryNotExecutable);
    }
    Ok(program)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mapping {
    pub source_offset: u64,
    pub destination: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub permissions: Permissions,
}

pub fn mapping_plan(
    program: &Program,
    load_bias: u64,
    output: &mut [Mapping],
) -> Result<usize, Error> {
    if output.len() < program.load_count {
        return Err(Error::BufferTooSmall);
    }
    for (slot, segment) in output.iter_mut().zip(program.loads()) {
        let destination = segment
            .virtual_address
            .checked_add(load_bias)
            .ok_or(Error::IntegerOverflow)?;
        *slot = Mapping {
            source_offset: segment.file_offset,
            destination,
            file_size: segment.file_size,
            memory_size: segment.memory_size,
            permissions: segment.permissions,
        };
    }
    Ok(program.load_count)
}

fn file_offset_for_vaddr(program: &Program, vaddr: u64, size: u64) -> Result<usize, Error> {
    for load in program.loads() {
        let relative = match vaddr.checked_sub(load.virtual_address) {
            Some(v) => v,
            None => continue,
        };
        let end = relative.checked_add(size).ok_or(Error::IntegerOverflow)?;
        if end <= load.file_size {
            return usize::try_from(
                load.file_offset
                    .checked_add(relative)
                    .ok_or(Error::IntegerOverflow)?,
            )
            .map_err(|_| Error::IntegerOverflow);
        }
    }
    Err(Error::BadDynamicTable)
}

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct DynamicInfo {
    pub strtab: u64,
    pub strsz: u64,
    pub symtab: u64,
    pub syment: u64,
    pub rela: u64,
    pub relasz: u64,
    pub relaent: u64,
}

pub fn dynamic_info(image: &[u8], program: &Program) -> Result<DynamicInfo, Error> {
    let (address, size) = program.dynamic.ok_or(Error::MissingDynamicTable)?;
    if size % 16 != 0 {
        return Err(Error::BadDynamicTable);
    }
    let offset = file_offset_for_vaddr(program, address, size)?;
    let mut info = DynamicInfo::default();
    let mut terminated = false;
    for item in 0..usize::try_from(size / 16).map_err(|_| Error::IntegerOverflow)? {
        let at = offset + item * 16;
        let tag = i64_at(image, at)?;
        let value = u64_at(image, at + 8)?;
        match tag {
            0 => {
                terminated = true;
                break;
            }
            5 => info.strtab = value,
            6 => info.symtab = value,
            7 => info.rela = value,
            8 => info.relasz = value,
            9 => info.relaent = value,
            10 => info.strsz = value,
            11 => info.syment = value,
            _ => {}
        }
    }
    if !terminated {
        return Err(Error::BadDynamicTable);
    }
    if info.relasz != 0 && (info.rela == 0 || info.relaent != 24 || info.relasz % 24 != 0) {
        return Err(Error::BadDynamicTable);
    }
    Ok(info)
}

pub trait SymbolResolver {
    fn resolve(&mut self, name: &[u8]) -> Option<u64>;
}

fn symbol_name<'a>(
    image: &'a [u8],
    program: &Program,
    info: DynamicInfo,
    index: u32,
) -> Result<(&'a [u8], u64, u16), Error> {
    if info.symtab == 0 || info.syment != 24 {
        return Err(Error::MissingSymbolTable);
    }
    if info.strtab == 0 || info.strsz == 0 {
        return Err(Error::MissingStringTable);
    }
    let sym_address = info
        .symtab
        .checked_add(
            u64::from(index)
                .checked_mul(24)
                .ok_or(Error::IntegerOverflow)?,
        )
        .ok_or(Error::IntegerOverflow)?;
    let sym = file_offset_for_vaddr(program, sym_address, 24)?;
    let name_index = u64::from(u32_at(image, sym)?);
    if name_index >= info.strsz {
        return Err(Error::BadString);
    }
    let strings = file_offset_for_vaddr(program, info.strtab, info.strsz)?;
    let start = strings
        .checked_add(usize::try_from(name_index).map_err(|_| Error::IntegerOverflow)?)
        .ok_or(Error::IntegerOverflow)?;
    let limit = strings
        .checked_add(usize::try_from(info.strsz).map_err(|_| Error::IntegerOverflow)?)
        .ok_or(Error::IntegerOverflow)?;
    let tail = image.get(start..limit).ok_or(Error::Truncated)?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::BadString)?;
    Ok((
        &tail[..length],
        u64_at(image, sym + 8)?,
        u16_at(image, sym + 6)?,
    ))
}

fn write_u64(memory: &mut [u8], base: u64, address: u64, value: u64) -> Result<(), Error> {
    let relative = address
        .checked_sub(base)
        .ok_or(Error::RelocationOutOfRange)?;
    let start = usize::try_from(relative).map_err(|_| Error::RelocationOutOfRange)?;
    let end = start.checked_add(8).ok_or(Error::IntegerOverflow)?;
    memory
        .get_mut(start..end)
        .ok_or(Error::RelocationOutOfRange)?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub struct RelocationTarget<'a> {
    pub load_bias: u64,
    pub mapped_base: u64,
    pub memory: &'a mut [u8],
    pub max_relocations: usize,
}

pub fn apply_relocations(
    image: &[u8],
    program: &Program,
    info: DynamicInfo,
    target: RelocationTarget<'_>,
    resolver: &mut impl SymbolResolver,
) -> Result<usize, Error> {
    let count = usize::try_from(info.relasz / 24).map_err(|_| Error::IntegerOverflow)?;
    if count > target.max_relocations {
        return Err(Error::TooManyRelocations);
    }
    if count == 0 {
        return Ok(0);
    }
    let rela = file_offset_for_vaddr(program, info.rela, info.relasz)?;
    for index in 0..count {
        let at = rela + index * 24;
        let destination = u64_at(image, at)?
            .checked_add(target.load_bias)
            .ok_or(Error::IntegerOverflow)?;
        let relocation = u64_at(image, at + 8)?;
        let kind = relocation as u32;
        let symbol = (relocation >> 32) as u32;
        let addend = i64_at(image, at + 16)?;
        let value = match kind {
            R_X86_64_RELATIVE if symbol == 0 => target.load_bias.wrapping_add_signed(addend),
            R_X86_64_64 | R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT => {
                let (name, defined_value, section) = symbol_name(image, program, info, symbol)?;
                let resolved = if section != 0 {
                    defined_value
                        .checked_add(target.load_bias)
                        .ok_or(Error::IntegerOverflow)?
                } else {
                    resolver.resolve(name).ok_or(Error::UnresolvedSymbol)?
                };
                if kind == R_X86_64_64 {
                    resolved.wrapping_add_signed(addend)
                } else if addend == 0 {
                    resolved
                } else {
                    return Err(Error::UnsupportedRelocation);
                }
            }
            _ => return Err(Error::UnsupportedRelocation),
        };
        write_u64(&mut *target.memory, target.mapped_base, destination, value)?;
    }
    Ok(count)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressWindow {
    pub start: u64,
    pub end: u64,
    pub alignment: u64,
}

pub fn aslr_placement(
    program: &Program,
    window: AddressWindow,
    entropy: [u8; 32],
) -> Result<u64, Error> {
    if window.start >= window.end || window.alignment == 0 || !window.alignment.is_power_of_two() {
        return Err(Error::InvalidAddressRange);
    }
    let span = program.image_span();
    let earliest = window
        .start
        .checked_add(window.alignment - 1)
        .ok_or(Error::IntegerOverflow)?
        & !(window.alignment - 1);
    let last =
        window.end.checked_sub(span).ok_or(Error::NoAslrPlacement)? & !(window.alignment - 1);
    if earliest > last {
        return Err(Error::NoAslrPlacement);
    }
    let slots = (last - earliest) / window.alignment + 1;
    let mut random = 0xcbf29ce484222325u64;
    for byte in entropy {
        random = (random ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    let address = earliest + (random % slots) * window.alignment;
    address
        .checked_sub(program.lowest_vaddr)
        .ok_or(Error::NoAslrPlacement)
}

pub fn instantiate_tls(
    image: &[u8],
    template: TlsTemplate,
    output: &mut [u8],
) -> Result<usize, Error> {
    let memory_size = usize::try_from(template.memory_size).map_err(|_| Error::IntegerOverflow)?;
    if output.len() < memory_size {
        return Err(Error::BufferTooSmall);
    }
    let source = range(template.file_offset, template.file_size, image.len())?;
    let initialized = source.len();
    if initialized > memory_size {
        return Err(Error::BadTlsImage);
    }
    output[..initialized].copy_from_slice(&image[source]);
    output[initialized..memory_size].fill(0);
    Ok(memory_size)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuxEntry {
    pub kind: u64,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialStack {
    pub stack_pointer: u64,
    pub bytes_used: usize,
}

fn push_bytes(buffer: &mut [u8], cursor: &mut usize, value: &[u8]) -> Result<usize, Error> {
    *cursor = cursor
        .checked_sub(value.len())
        .ok_or(Error::BufferTooSmall)?;
    buffer[*cursor..*cursor + value.len()].copy_from_slice(value);
    Ok(*cursor)
}

fn push_word(buffer: &mut [u8], cursor: &mut usize, word: u64) -> Result<(), Error> {
    push_bytes(buffer, cursor, &word.to_le_bytes()).map(|_| ())
}

pub fn build_initial_stack(
    buffer: &mut [u8],
    virtual_top: u64,
    argv: &[&[u8]],
    env: &[&[u8]],
    auxv: &[AuxEntry],
    argv_addresses: &mut [u64],
    env_addresses: &mut [u64],
) -> Result<InitialStack, Error> {
    if argv_addresses.len() < argv.len() || env_addresses.len() < env.len() {
        return Err(Error::BufferTooSmall);
    }
    let mut cursor = buffer.len();
    for (index, value) in env.iter().enumerate().rev() {
        if value.contains(&0) {
            return Err(Error::BadString);
        }
        push_bytes(buffer, &mut cursor, &[0])?;
        let offset = push_bytes(buffer, &mut cursor, value)?;
        env_addresses[index] = virtual_top
            .checked_sub((buffer.len() - offset) as u64)
            .ok_or(Error::IntegerOverflow)?;
    }
    for (index, value) in argv.iter().enumerate().rev() {
        if value.contains(&0) {
            return Err(Error::BadString);
        }
        push_bytes(buffer, &mut cursor, &[0])?;
        let offset = push_bytes(buffer, &mut cursor, value)?;
        argv_addresses[index] = virtual_top
            .checked_sub((buffer.len() - offset) as u64)
            .ok_or(Error::IntegerOverflow)?;
    }
    // Align the final ABI entry stack (including argc, pointer vectors and auxv) to 16 bytes.
    let table_words = 1 + argv.len() + 1 + env.len() + 1 + (auxv.len() + 1) * 2;
    let final_cursor = cursor
        .checked_sub(table_words * 8)
        .ok_or(Error::BufferTooSmall)?
        & !15;
    buffer[final_cursor..cursor].fill(0);
    cursor = final_cursor + table_words * 8;
    // Build backwards in reverse logical order.
    push_word(buffer, &mut cursor, 0)?;
    push_word(buffer, &mut cursor, 0)?;
    for entry in auxv.iter().rev() {
        push_word(buffer, &mut cursor, entry.value)?;
        push_word(buffer, &mut cursor, entry.kind)?;
    }
    push_word(buffer, &mut cursor, 0)?;
    for address in env_addresses[..env.len()].iter().rev() {
        push_word(buffer, &mut cursor, *address)?;
    }
    push_word(buffer, &mut cursor, 0)?;
    for address in argv_addresses[..argv.len()].iter().rev() {
        push_word(buffer, &mut cursor, *address)?;
    }
    push_word(buffer, &mut cursor, argv.len() as u64)?;
    debug_assert_eq!(cursor, final_cursor);
    Ok(InitialStack {
        stack_pointer: virtual_top
            .checked_sub((buffer.len() - cursor) as u64)
            .ok_or(Error::IntegerOverflow)?,
        bytes_used: buffer.len() - cursor,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedImageMetadata<'a> {
    pub key_id: &'a [u8],
    pub digest: &'a [u8; 32],
    pub signature: &'a [u8],
    pub monotonic_version: u64,
}

pub trait ImageVerifier {
    fn verify(&mut self, image: &[u8], metadata: SignedImageMetadata<'_>) -> bool;
}

pub fn verify_signed_image(
    image: &[u8],
    metadata: SignedImageMetadata<'_>,
    verifier: &mut impl ImageVerifier,
) -> Result<(), Error> {
    if metadata.key_id.is_empty()
        || metadata.signature.is_empty()
        || !verifier.verify(image, metadata)
    {
        return Err(Error::SignatureRejected);
    }
    Ok(())
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(data: &mut [u8], at: usize, value: u64) {
        data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn elf(kind: u16) -> std::vec::Vec<u8> {
        let mut e = vec![0u8; 0x700];
        e[0..4].copy_from_slice(b"\x7fELF");
        e[4] = 2;
        e[5] = 1;
        e[6] = 1;
        put16(&mut e, 16, kind);
        put16(&mut e, 18, 62);
        put32(&mut e, 20, 1);
        put64(&mut e, 24, 0x401000);
        put64(&mut e, 32, 64);
        put16(&mut e, 52, 64);
        put16(&mut e, 54, 56);
        put16(&mut e, 56, 2);
        put16(&mut e, 58, 64);
        // RX load, congruent at page offset zero.
        put32(&mut e, 64, 1);
        put32(&mut e, 68, 5);
        put64(&mut e, 72, 0);
        put64(&mut e, 80, 0x400000);
        put64(&mut e, 96, 0x500);
        put64(&mut e, 104, 0x1800);
        put64(&mut e, 112, 0x1000);
        // RW load.
        put32(&mut e, 120, 1);
        put32(&mut e, 124, 6);
        put64(&mut e, 128, 0x300);
        put64(&mut e, 136, 0x403300);
        put64(&mut e, 152, 0x200);
        put64(&mut e, 160, 0x300);
        put64(&mut e, 168, 0x1000);
        e
    }

    #[test]
    fn accepts_strict_executable_and_builds_wx_plan() {
        let e = elf(ET_EXEC);
        let program = parse_elf64(&e).unwrap();
        assert_eq!(program.load_count, 2);
        let mut mappings = [Mapping {
            source_offset: 0,
            destination: 0,
            file_size: 0,
            memory_size: 0,
            permissions: Permissions(0),
        }; 2];
        assert_eq!(mapping_plan(&program, 0x100000, &mut mappings), Ok(2));
        assert_eq!(mappings[0].destination, 0x500000);
        assert!(!mappings[0].permissions.contains(Permissions::WRITE));
        assert!(!mappings[1].permissions.contains(Permissions::EXECUTE));
    }

    #[test]
    fn rejects_rwx_and_invalid_entry() {
        let mut e = elf(ET_EXEC);
        put32(&mut e, 68, 7);
        assert_eq!(parse_elf64(&e), Err(Error::WritableExecutable));
        let mut e = elf(ET_EXEC);
        put64(&mut e, 24, 0x403400);
        assert_eq!(parse_elf64(&e), Err(Error::EntryNotExecutable));
    }

    #[test]
    fn rejects_truncated_and_bad_alignment() {
        assert_eq!(parse_elf64(&[0; 16]), Err(Error::Truncated));
        let mut e = elf(ET_EXEC);
        put64(&mut e, 112, 24);
        assert_eq!(parse_elf64(&e), Err(Error::BadAlignment));
    }

    #[test]
    fn deterministic_aslr_is_aligned_and_in_window() {
        let program = parse_elf64(&elf(ET_DYN)).unwrap();
        let window = AddressWindow {
            start: 0x1_0000_0000,
            end: 0x1_1000_0000,
            alignment: 0x20_0000,
        };
        let one = aslr_placement(&program, window, [7; 32]).unwrap();
        let two = aslr_placement(&program, window, [7; 32]).unwrap();
        assert_eq!(one, two);
        assert_eq!((program.lowest_vaddr + one) % 0x20_0000, 0);
        assert!(program.highest_vaddr + one <= window.end);
    }

    #[test]
    fn tls_copies_template_and_zeros_bss() {
        let image = [1, 2, 3, 4];
        let template = TlsTemplate {
            file_offset: 1,
            file_size: 2,
            memory_size: 5,
            alignment: 8,
        };
        let mut output = [0xaa; 5];
        assert_eq!(instantiate_tls(&image, template, &mut output), Ok(5));
        assert_eq!(output, [2, 3, 0, 0, 0]);
    }

    fn word(buffer: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(buffer[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn stack_has_sysv_vectors_strings_and_alignment() {
        let mut buffer = [0u8; 256];
        let mut av = [0; 2];
        let mut ev = [0; 1];
        let built = build_initial_stack(
            &mut buffer,
            0x8000,
            &[b"nova", b"--safe"],
            &[b"LANG=uk_UA"],
            &[AuxEntry {
                kind: 9,
                value: 0x401000,
            }],
            &mut av,
            &mut ev,
        )
        .unwrap();
        assert_eq!(built.stack_pointer & 15, 0);
        let start = buffer.len() - built.bytes_used;
        assert_eq!(word(&buffer, start), 2);
        assert_eq!(word(&buffer, start + 8), av[0]);
        assert_eq!(word(&buffer, start + 32), ev[0]);
        assert_eq!(word(&buffer, start + 48), 9);
        assert_eq!(word(&buffer, start + 56), 0x401000);
    }

    #[test]
    fn stack_rejects_embedded_nul_and_small_output() {
        let mut buffer = [0; 32];
        let mut av = [0; 1];
        let mut ev = [];
        assert_eq!(
            build_initial_stack(&mut buffer, 0x1000, &[b"a\0b"], &[], &[], &mut av, &mut ev),
            Err(Error::BadString)
        );
        assert_eq!(
            build_initial_stack(
                &mut buffer,
                0x1000,
                &[b"long command argument"],
                &[],
                &[],
                &mut av,
                &mut ev
            ),
            Err(Error::BufferTooSmall)
        );
    }

    struct TestVerifier(bool);
    impl ImageVerifier for TestVerifier {
        fn verify(&mut self, image: &[u8], metadata: SignedImageMetadata<'_>) -> bool {
            self.0 && image == b"app" && metadata.monotonic_version == 4
        }
    }

    #[test]
    fn signature_policy_is_external_and_fail_closed() {
        let digest = [0; 32];
        let metadata = SignedImageMetadata {
            key_id: b"release",
            digest: &digest,
            signature: b"sig",
            monotonic_version: 4,
        };
        assert_eq!(
            verify_signed_image(b"app", metadata, &mut TestVerifier(true)),
            Ok(())
        );
        assert_eq!(
            verify_signed_image(b"app", metadata, &mut TestVerifier(false)),
            Err(Error::SignatureRejected)
        );
    }

    struct Resolver;
    impl SymbolResolver for Resolver {
        fn resolve(&mut self, name: &[u8]) -> Option<u64> {
            (name == b"nova_api").then_some(0xdead_beef)
        }
    }

    fn dynamic_elf() -> std::vec::Vec<u8> {
        let mut e = elf(ET_DYN);
        // Extend the disjoint RW load and add a dynamic header within it.
        put64(&mut e, 152, 0x300);
        put64(&mut e, 160, 0x400);
        put16(&mut e, 56, 3);
        let p = 176;
        put32(&mut e, p, PT_DYNAMIC);
        put32(&mut e, p + 4, 6);
        put64(&mut e, p + 8, 0x300);
        put64(&mut e, p + 16, 0x403300);
        put64(&mut e, p + 32, 128);
        put64(&mut e, p + 40, 128);
        put64(&mut e, p + 48, 8);
        // Dynamic tags.
        let tags = [
            (5, 0x403400),
            (10, 32),
            (6, 0x403440),
            (11, 24),
            (7, 0x403480),
            (8, 48),
            (9, 24),
        ];
        for (i, (tag, val)) in tags.iter().enumerate() {
            put64(&mut e, 0x300 + i * 16, *tag);
            put64(&mut e, 0x308 + i * 16, *val);
        }
        put64(&mut e, 0x370, 0);
        e[0x400..0x409].copy_from_slice(b"nova_api\0");
        // Symbol 1 undefined, named nova_api.
        put32(&mut e, 0x458, 0); // symbol 1 name index is 0
        // RELATIVE and GLOB_DAT.
        put64(&mut e, 0x480, 0x4034c0);
        put64(&mut e, 0x488, 8);
        put64(&mut e, 0x490, 0x55);
        put64(&mut e, 0x498, 0x4034c8);
        put64(&mut e, 0x4a0, (1u64 << 32) | 6);
        put64(&mut e, 0x4a8, 0);
        e
    }

    #[test]
    fn bounded_relative_and_symbol_relocations_apply() {
        let e = dynamic_elf();
        let program = parse_elf64(&e).unwrap();
        let info = dynamic_info(&e, &program).unwrap();
        let mut memory = [0u8; 0x100];
        assert_eq!(
            apply_relocations(
                &e,
                &program,
                info,
                RelocationTarget {
                    load_bias: 0x100000,
                    mapped_base: 0x5034c0,
                    memory: &mut memory,
                    max_relocations: 2,
                },
                &mut Resolver
            ),
            Ok(2)
        );
        assert_eq!(word(&memory, 0), 0x100055);
        assert_eq!(word(&memory, 8), 0xdead_beef);
        assert_eq!(
            apply_relocations(
                &e,
                &program,
                info,
                RelocationTarget {
                    load_bias: 0x100000,
                    mapped_base: 0x5034c0,
                    memory: &mut memory,
                    max_relocations: 1,
                },
                &mut Resolver
            ),
            Err(Error::TooManyRelocations)
        );
    }
}
