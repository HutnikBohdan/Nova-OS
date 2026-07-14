const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const PT_LOAD: u32 = 1;
const EM_X86_64: u16 = 62;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedVersion,
    UnsupportedMachine,
    UnsupportedType,
    InvalidHeaderSize,
    InvalidProgramHeader,
    TooManySegments,
    FileLargerThanMemory,
    InvalidAlignment,
    AddressOverflow,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentFlags(u32);

impl SegmentFlags {
    pub const EXECUTE: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const READ: Self = Self(4);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    pub file_offset: u64,
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
    pub flags: SegmentFlags,
}

impl LoadSegment {
    const EMPTY: Self = Self {
        file_offset: 0,
        virtual_address: 0,
        file_size: 0,
        memory_size: 0,
        alignment: 1,
        flags: SegmentFlags(0),
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadPlan<const N: usize> {
    pub entry: u64,
    segments: [LoadSegment; N],
    count: usize,
}

impl<const N: usize> LoadPlan<N> {
    pub fn parse(image: &[u8]) -> Result<Self, ElfError> {
        if image.len() < ELF_HEADER_SIZE {
            return Err(ElfError::Truncated);
        }
        if image[0..4] != *b"\x7fELF" {
            return Err(ElfError::BadMagic);
        }
        if image[4] != 2 {
            return Err(ElfError::UnsupportedClass);
        }
        if image[5] != 1 {
            return Err(ElfError::UnsupportedEndian);
        }
        if image[6] != 1 || read_u32(image, 20)? != 1 {
            return Err(ElfError::UnsupportedVersion);
        }
        if read_u16(image, 18)? != EM_X86_64 {
            return Err(ElfError::UnsupportedMachine);
        }
        if !matches!(read_u16(image, 16)?, 2 | 3) {
            return Err(ElfError::UnsupportedType);
        }
        if read_u16(image, 52)? as usize != ELF_HEADER_SIZE {
            return Err(ElfError::InvalidHeaderSize);
        }
        let ph_offset = read_u64(image, 32)? as usize;
        let ph_size = read_u16(image, 54)? as usize;
        let ph_count = read_u16(image, 56)? as usize;
        if ph_count != 0 && ph_size != PROGRAM_HEADER_SIZE {
            return Err(ElfError::InvalidProgramHeader);
        }
        let table_size = ph_size
            .checked_mul(ph_count)
            .ok_or(ElfError::InvalidProgramHeader)?;
        if ph_offset
            .checked_add(table_size)
            .filter(|end| *end <= image.len())
            .is_none()
        {
            return Err(ElfError::InvalidProgramHeader);
        }

        let mut plan = Self {
            entry: read_u64(image, 24)?,
            segments: [LoadSegment::EMPTY; N],
            count: 0,
        };
        for index in 0..ph_count {
            let at = ph_offset + index * ph_size;
            if read_u32(image, at)? != PT_LOAD {
                continue;
            }
            if plan.count == N {
                return Err(ElfError::TooManySegments);
            }
            let flags = read_u32(image, at + 4)?;
            if flags & !7 != 0 {
                return Err(ElfError::InvalidProgramHeader);
            }
            let file_offset = read_u64(image, at + 8)?;
            let virtual_address = read_u64(image, at + 16)?;
            let file_size = read_u64(image, at + 32)?;
            let memory_size = read_u64(image, at + 40)?;
            let alignment = read_u64(image, at + 48)?;
            if file_size > memory_size {
                return Err(ElfError::FileLargerThanMemory);
            }
            if alignment > 1 && !alignment.is_power_of_two() {
                return Err(ElfError::InvalidAlignment);
            }
            if alignment > 1 && file_offset % alignment != virtual_address % alignment {
                return Err(ElfError::InvalidAlignment);
            }
            file_offset
                .checked_add(file_size)
                .filter(|end| *end <= image.len() as u64)
                .ok_or(ElfError::Truncated)?;
            virtual_address
                .checked_add(memory_size)
                .ok_or(ElfError::AddressOverflow)?;
            plan.segments[plan.count] = LoadSegment {
                file_offset,
                virtual_address,
                file_size,
                memory_size,
                alignment,
                flags: SegmentFlags(flags),
            };
            plan.count += 1;
        }
        Ok(plan)
    }

    pub fn segments(&self) -> &[LoadSegment] {
        &self.segments[..self.count]
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, ElfError> {
    let raw: [u8; 2] = bytes
        .get(at..at + 2)
        .ok_or(ElfError::Truncated)?
        .try_into()
        .unwrap();
    Ok(u16::from_le_bytes(raw))
}
fn read_u32(bytes: &[u8], at: usize) -> Result<u32, ElfError> {
    let raw: [u8; 4] = bytes
        .get(at..at + 4)
        .ok_or(ElfError::Truncated)?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(raw))
}
fn read_u64(bytes: &[u8], at: usize) -> Result<u64, ElfError> {
    let raw: [u8; 8] = bytes
        .get(at..at + 8)
        .ok_or(ElfError::Truncated)?
        .try_into()
        .unwrap();
    Ok(u64::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_elf() -> [u8; 128] {
        let mut elf = [0u8; 128];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[6] = 1;
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        elf[20..24].copy_from_slice(&1u32.to_le_bytes());
        elf[24..32].copy_from_slice(&0x400000u64.to_le_bytes());
        elf[32..40].copy_from_slice(&64u64.to_le_bytes());
        elf[52..54].copy_from_slice(&(ELF_HEADER_SIZE as u16).to_le_bytes());
        elf[54..56].copy_from_slice(&(PROGRAM_HEADER_SIZE as u16).to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        let p = 64;
        elf[p..p + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        elf[p + 4..p + 8].copy_from_slice(&5u32.to_le_bytes());
        elf[p + 8..p + 16].copy_from_slice(&120u64.to_le_bytes());
        elf[p + 16..p + 24].copy_from_slice(&0x400000u64.to_le_bytes());
        elf[p + 32..p + 40].copy_from_slice(&8u64.to_le_bytes());
        elf[p + 40..p + 48].copy_from_slice(&16u64.to_le_bytes());
        elf[p + 48..p + 56].copy_from_slice(&1u64.to_le_bytes());
        elf
    }

    #[test]
    fn builds_checked_load_plan() {
        let elf = valid_elf();
        let plan = LoadPlan::<2>::parse(&elf).unwrap();
        assert_eq!(plan.entry, 0x400000);
        assert_eq!(plan.segments().len(), 1);
        assert!(plan.segments()[0].flags.contains(SegmentFlags::EXECUTE));
    }

    #[test]
    fn rejects_segment_outside_image() {
        let mut elf = valid_elf();
        elf[96..104].copy_from_slice(&16u64.to_le_bytes());
        assert_eq!(LoadPlan::<2>::parse(&elf), Err(ElfError::Truncated));
    }
}
