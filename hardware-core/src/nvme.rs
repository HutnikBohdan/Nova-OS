use crate::ParseError;

pub const ADMIN_IDENTIFY: u8 = 0x06;
pub const IO_FLUSH: u8 = 0x00;
pub const IO_WRITE: u8 = 0x01;
pub const IO_READ: u8 = 0x02;

/// NVMe submission queue entry (64 bytes, NVMe 2.x section 4.2).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Submission {
    pub cdw0: u32,
    pub namespace_id: u32,
    pub reserved: [u32; 2],
    pub metadata: u64,
    pub data_ptr1: u64,
    pub data_ptr2: u64,
    pub command_specific: [u32; 6],
}

impl Submission {
    pub const fn new(opcode: u8, command_id: u16, namespace_id: u32) -> Self {
        Self {
            cdw0: opcode as u32 | ((command_id as u32) << 16),
            namespace_id,
            reserved: [0; 2],
            metadata: 0,
            data_ptr1: 0,
            data_ptr2: 0,
            command_specific: [0; 6],
        }
    }

    pub const fn identify(command_id: u16, data: u64, namespace_id: u32, cns: u8) -> Self {
        let mut cmd = Self::new(ADMIN_IDENTIFY, command_id, namespace_id);
        cmd.data_ptr1 = data;
        cmd.command_specific[0] = cns as u32;
        cmd
    }

    pub const fn read(
        command_id: u16,
        namespace_id: u32,
        data: u64,
        lba: u64,
        blocks: u16,
    ) -> Self {
        let mut cmd = Self::new(IO_READ, command_id, namespace_id);
        cmd.data_ptr1 = data;
        cmd.command_specific[0] = lba as u32;
        cmd.command_specific[1] = (lba >> 32) as u32;
        cmd.command_specific[2] = blocks.saturating_sub(1) as u32;
        cmd
    }
}

/// NVMe completion queue entry (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub command_specific: u32,
    pub reserved: u32,
    pub submission_head: u16,
    pub submission_queue_id: u16,
    pub command_id: u16,
    pub status: u16,
}

impl Completion {
    pub const fn phase(self) -> bool {
        self.status & 1 != 0
    }
    pub const fn status_code(self) -> u16 {
        self.status >> 1
    }
    pub const fn success(self) -> bool {
        self.status_code() == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueCursor {
    pub index: u16,
    pub phase: bool,
    pub depth: u16,
}

impl QueueCursor {
    pub const fn new(depth: u16) -> Result<Self, ParseError> {
        if depth < 2 {
            return Err(ParseError::InvalidLength);
        }
        Ok(Self {
            index: 0,
            phase: true,
            depth,
        })
    }

    pub fn consume(&mut self, completion: &Completion) -> bool {
        if completion.phase() != self.phase {
            return false;
        }
        self.index += 1;
        if self.index == self.depth {
            self.index = 0;
            self.phase = !self.phase;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LbaFormat {
    pub metadata_size: u16,
    pub data_size_log2: u8,
    pub relative_performance: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamespaceInfo {
    pub size_blocks: u64,
    pub capacity_blocks: u64,
    pub utilization_blocks: u64,
    pub formatted_lba_index: u8,
    pub format: LbaFormat,
}

impl NamespaceInfo {
    /// Parses the standardized fields of a 4096-byte Identify Namespace page.
    pub fn parse(page: &[u8]) -> Result<Self, ParseError> {
        if page.len() < 192 {
            return Err(ParseError::Truncated);
        }
        let read_u64 = |o: usize| u64::from_le_bytes(page[o..o + 8].try_into().unwrap());
        let formatted_lba_index = page[26] & 0x0f;
        let formats = (page[25] & 0x0f) as usize + 1;
        if formatted_lba_index as usize >= formats {
            return Err(ParseError::InvalidValue);
        }
        let offset = 128 + formatted_lba_index as usize * 4;
        let raw = u32::from_le_bytes(page[offset..offset + 4].try_into().unwrap());
        let format = LbaFormat {
            metadata_size: raw as u16,
            data_size_log2: ((raw >> 16) & 0xff) as u8,
            relative_performance: ((raw >> 24) & 0x03) as u8,
        };
        if !(9..=31).contains(&format.data_size_log2) {
            return Err(ParseError::InvalidValue);
        }
        Ok(Self {
            size_blocks: read_u64(0),
            capacity_blocks: read_u64(8),
            utilization_blocks: read_u64(16),
            formatted_lba_index,
            format,
        })
    }

    pub const fn block_size(self) -> u64 {
        1u64 << self.format.data_size_log2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structures_have_hardware_sizes() {
        assert_eq!(core::mem::size_of::<Submission>(), 64);
        assert_eq!(core::mem::size_of::<Completion>(), 16);
    }

    #[test]
    fn builds_read_command() {
        let c = Submission::read(7, 3, 0x1000, 0x1234_5678_9abc_def0, 8);
        assert_eq!(c.cdw0, 0x0007_0002);
        assert_eq!(c.command_specific[0], 0x9abc_def0);
        assert_eq!(c.command_specific[1], 0x1234_5678);
        assert_eq!(c.command_specific[2], 7);
    }

    #[test]
    fn completion_cursor_wraps_phase() {
        let mut q = QueueCursor::new(2).unwrap();
        let ready = Completion {
            status: 1,
            ..Completion::default()
        };
        assert!(q.consume(&ready));
        assert!(q.consume(&ready));
        assert_eq!((q.index, q.phase), (0, false));
        assert!(!q.consume(&ready));
    }

    #[test]
    fn parses_namespace() {
        let mut p = [0u8; 4096];
        p[0..8].copy_from_slice(&1000u64.to_le_bytes());
        p[8..16].copy_from_slice(&900u64.to_le_bytes());
        p[16..24].copy_from_slice(&12u64.to_le_bytes());
        p[25] = 0;
        p[26] = 0;
        p[128..132].copy_from_slice(&(9u32 << 16).to_le_bytes());
        let n = NamespaceInfo::parse(&p).unwrap();
        assert_eq!(n.block_size(), 512);
        assert_eq!(n.capacity_blocks, 900);
    }
}
