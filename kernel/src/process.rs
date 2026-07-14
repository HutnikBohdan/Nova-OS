//! Nova's first x86-64 userspace launch primitive.
//!
//! This module deliberately does not borrow the bootloader's address space as a
//! "process".  A caller must provide a fresh page table through
//! [`UserAddressSpace`].  The image builder maps a read/execute code page, a
//! non-executable read/write stack and a non-executable read/write exchange
//! page.  The unmapped page below the stack is the guard page.

use core::arch::asm;
use memory_core::{PAGE_SIZE, Permissions};
use runtime_core::syscall::SyscallNumber;

pub const USER_CODE_BASE: u64 = 0x0000_0000_0040_0000;
pub const USER_EXCHANGE_BASE: u64 = 0x0000_0000_0060_0000;
pub const USER_STACK_GUARD: u64 = 0x0000_7fff_ffff_a000;
pub const USER_STACK_BASE: u64 = USER_STACK_GUARD + PAGE_SIZE;
pub const USER_STACK_PAGES: u16 = 4;
pub const USER_STACK_TOP: u64 = USER_STACK_BASE + PAGE_SIZE * USER_STACK_PAGES as u64;
/// Nova ABI number imported from the architecture-neutral runtime contract.
pub const SYSCALL_PROCESS_EXIT: u64 = SyscallNumber::ProcessExit as u64;
/// A value which cannot be confused with an ordinary process exit argument.
pub const RING3_PROOF_MARKER: u64 = 0x4e4f_5641_5233_4f4b; // "NOVAR3OK"

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageRole {
    UserCode,
    UserStack,
    SyscallExchange,
}

impl PageRole {
    pub const fn writable(self) -> bool {
        matches!(self, Self::UserStack | Self::SyscallExchange)
    }

    pub const fn executable(self) -> bool {
        matches!(self, Self::UserCode)
    }

    /// Authoritative mapping permissions from `memory-core`.
    pub const fn permissions(self) -> Permissions {
        match self {
            Self::UserCode => Permissions::READ
                .union(Permissions::EXECUTE)
                .union(Permissions::USER),
            Self::UserStack | Self::SyscallExchange => Permissions::READ
                .union(Permissions::WRITE)
                .union(Permissions::USER),
        }
    }
}

/// Architecture-specific backing for a process address space.
///
/// # Safety
///
/// Implementations must create mappings with the USER bit set, must honour the
/// write/execute policy described by `role`, and must never map the guard page.
/// `activate` must switch CR3 to this address space while preserving the kernel,
/// GDT, IDT and syscall handler mappings required to return from ring 3.
pub unsafe trait UserAddressSpace {
    type Error;

    fn map_zeroed_page(&mut self, virtual_address: u64, role: PageRole) -> Result<(), Self::Error>;
    fn write_user(&mut self, virtual_address: u64, bytes: &[u8]) -> Result<(), Self::Error>;
    fn seal_code_page(&mut self, virtual_address: u64) -> Result<(), Self::Error>;
    unsafe fn activate(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserModeSelectors {
    code: u16,
    data: u16,
}

impl UserModeSelectors {
    /// Accept selectors only when their requested privilege level is ring 3.
    pub const fn new(code: u16, data: u16) -> Option<Self> {
        if code & 3 == 3 && data & 3 == 3 && code & !7 != 0 && data & !7 != 0 {
            Some(Self { code, data })
        } else {
            None
        }
    }

    pub const fn code(self) -> u16 {
        self.code
    }
    pub const fn data(self) -> u16 {
        self.data
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareError<E> {
    InvalidSelectors,
    ImageTooLarge,
    Map(E),
    Write(E),
    Seal(E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedUserProcess {
    instruction_pointer: u64,
    stack_pointer: u64,
    selectors: UserModeSelectors,
    proof: Ring3Proof,
}

impl PreparedUserProcess {
    pub const fn instruction_pointer(&self) -> u64 {
        self.instruction_pointer
    }
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }
    pub const fn proof(&self) -> Ring3Proof {
        self.proof
    }
    pub const fn selectors(&self) -> UserModeSelectors {
        self.selectors
    }
}

/// Minimal first user program, encoded by Nova itself.
///
/// It invokes `ProcessExit(RING3_PROOF_MARKER, 0)` using `int 0x80`.  The final
/// `ud2` makes an incorrectly-returning syscall fail closed instead of running
/// into adjacent memory.
const FIRST_USER_IMAGE: &[u8] = &[
    0x48, 0xb8, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rax, 2 (yield)
    0xcd, 0x80, // int 0x80
    0x48, 0xb8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rax, 1
    0x48, 0xbf, 0x4b, 0x4f, 0x33, 0x52, 0x41, 0x56, 0x4f, 0x4e, // mov rdi, marker
    0x48, 0x31, 0xf6, // xor rsi, rsi
    0xcd, 0x80, // int 0x80
    0x0f, 0x0b, // ud2
];

const CRASH_USER_IMAGE: &[u8] = &[0x0f, 0x0b]; // ud2
const PREEMPT_USER_IMAGE: &[u8] = &[
    0x48, 0xb8, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rax, 0x600000
    0x48, 0xff, 0x00, // inc qword [rax]
    0xeb, 0xfb, // jmp to inc
    0x48, 0xb8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rax, 1
    0x48, 0xbf, 0x4b, 0x4f, 0x33, 0x52, 0x41, 0x56, 0x4f, 0x4e, // mov rdi, marker
    0x48, 0x31, 0xf6, // xor rsi, rsi
    0xcd, 0x80, // int 0x80
    0x0f, 0x0b, // ud2
];
pub const PREEMPT_EXIT_IP: u64 = USER_CODE_BASE + 15;

/// Build the first isolated process without activating it.
///
/// The function is safe because all privileged memory invariants belong to the
/// `unsafe UserAddressSpace` implementation.  No partially built image is ever
/// returned.
pub fn prepare_first_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    prepare_image(address_space, selectors, FIRST_USER_IMAGE)
}

pub fn prepare_crash_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    prepare_image(address_space, selectors, CRASH_USER_IMAGE)
}

pub fn prepare_preempt_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    prepare_image(address_space, selectors, PREEMPT_USER_IMAGE)
}

pub fn prepare_console_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
    message: &[u8],
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    let mut image = [0u8; 80];
    let mut at = 0;
    for immediate in [
        SyscallNumber::ObjectWrite as u64,
        USER_EXCHANGE_BASE,
        message.len() as u64,
    ] {
        image[at..at + 2].copy_from_slice(&[
            0x48,
            if at == 0 {
                0xb8
            } else if at == 10 {
                0xbf
            } else {
                0xbe
            },
        ]);
        image[at + 2..at + 10].copy_from_slice(&immediate.to_le_bytes());
        at += 10;
    }
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    image[at..at + 10].copy_from_slice(&[0x48, 0xb8, 2, 0, 0, 0, 0, 0, 0, 0]);
    at += 10;
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    image[at..at + 10].copy_from_slice(&[0x48, 0xb8, 1, 0, 0, 0, 0, 0, 0, 0]);
    at += 10;
    image[at..at + 2].copy_from_slice(&[0x48, 0xbf]);
    image[at + 2..at + 10].copy_from_slice(&RING3_PROOF_MARKER.to_le_bytes());
    at += 10;
    image[at..at + 3].copy_from_slice(&[0x48, 0x31, 0xf6]);
    at += 3;
    image[at..at + 4].copy_from_slice(&[0xcd, 0x80, 0x0f, 0x0b]);
    at += 4;
    let process = prepare_image(address_space, selectors, &image[..at])?;
    address_space
        .write_user(USER_EXCHANGE_BASE, message)
        .map_err(PrepareError::Write)?;
    Ok(process)
}

pub fn prepare_ipc_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
    message: &[u8],
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    let mut image = [0u8; 192];
    let mut at = 0;
    fn mov_imm(image: &mut [u8], at: &mut usize, opcode: u8, value: u64) {
        image[*at..*at + 2].copy_from_slice(&[0x48, opcode]);
        image[*at + 2..*at + 10].copy_from_slice(&value.to_le_bytes());
        *at += 10;
    }
    mov_imm(&mut image, &mut at, 0xb8, SyscallNumber::ChannelSend as u64);
    mov_imm(&mut image, &mut at, 0xbf, 42);
    mov_imm(&mut image, &mut at, 0xbe, USER_EXCHANGE_BASE);
    mov_imm(&mut image, &mut at, 0xba, message.len() as u64);
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    mov_imm(
        &mut image,
        &mut at,
        0xb8,
        SyscallNumber::ChannelReceive as u64,
    );
    mov_imm(&mut image, &mut at, 0xbf, USER_EXCHANGE_BASE + 64);
    mov_imm(&mut image, &mut at, 0xbe, 64);
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    image[at..at + 4].copy_from_slice(&[0x48, 0x83, 0xf8, message.len() as u8]);
    at += 4;
    image[at..at + 2].copy_from_slice(&[0x75, 0x38]);
    at += 2;
    mov_imm(&mut image, &mut at, 0xb8, SyscallNumber::ObjectWrite as u64);
    mov_imm(&mut image, &mut at, 0xbf, USER_EXCHANGE_BASE + 64);
    mov_imm(&mut image, &mut at, 0xbe, message.len() as u64);
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    mov_imm(&mut image, &mut at, 0xb8, SyscallNumber::ThreadYield as u64);
    image[at..at + 2].copy_from_slice(&[0xcd, 0x80]);
    at += 2;
    mov_imm(&mut image, &mut at, 0xb8, SyscallNumber::ProcessExit as u64);
    mov_imm(&mut image, &mut at, 0xbf, RING3_PROOF_MARKER);
    image[at..at + 3].copy_from_slice(&[0x48, 0x31, 0xf6]);
    at += 3;
    image[at..at + 4].copy_from_slice(&[0xcd, 0x80, 0x0f, 0x0b]);
    at += 4;
    let process = prepare_image(address_space, selectors, &image[..at])?;
    address_space
        .write_user(USER_EXCHANGE_BASE, message)
        .map_err(PrepareError::Write)?;
    Ok(process)
}

pub fn prepare_compiled_process<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
    code: &[u8],
    entry_offset: usize,
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    const WRAPPER_LEN: usize = 32;
    if entry_offset >= code.len() || WRAPPER_LEN + code.len() > PAGE_SIZE as usize {
        return Err(PrepareError::ImageTooLarge);
    }
    let mut image = [0u8; PAGE_SIZE as usize];
    image[0] = 0xe8;
    let displacement = (WRAPPER_LEN + entry_offset) as i32 - 5;
    image[1..5].copy_from_slice(&displacement.to_le_bytes());
    image[5..8].copy_from_slice(&[0x48, 0x89, 0xc6]); // mov rsi, rax
    image[8..10].copy_from_slice(&[0x48, 0xb8]);
    image[10..18].copy_from_slice(&(SyscallNumber::ProcessExit as u64).to_le_bytes());
    image[18..20].copy_from_slice(&[0x48, 0xbf]);
    image[20..28].copy_from_slice(&RING3_PROOF_MARKER.to_le_bytes());
    image[28..32].copy_from_slice(&[0xcd, 0x80, 0x0f, 0x0b]);
    image[WRAPPER_LEN..WRAPPER_LEN + code.len()].copy_from_slice(code);
    prepare_image(address_space, selectors, &image[..WRAPPER_LEN + code.len()])
}

fn prepare_image<A: UserAddressSpace>(
    address_space: &mut A,
    selectors: UserModeSelectors,
    image: &[u8],
) -> Result<PreparedUserProcess, PrepareError<A::Error>> {
    address_space
        .map_zeroed_page(USER_CODE_BASE, PageRole::UserCode)
        .map_err(PrepareError::Map)?;
    address_space
        .map_zeroed_page(USER_EXCHANGE_BASE, PageRole::SyscallExchange)
        .map_err(PrepareError::Map)?;
    for page in 0..USER_STACK_PAGES as u64 {
        address_space
            .map_zeroed_page(USER_STACK_BASE + page * PAGE_SIZE, PageRole::UserStack)
            .map_err(PrepareError::Map)?;
    }
    address_space
        .write_user(USER_CODE_BASE, image)
        .map_err(PrepareError::Write)?;
    address_space
        .seal_code_page(USER_CODE_BASE)
        .map_err(PrepareError::Seal)?;

    Ok(PreparedUserProcess {
        instruction_pointer: USER_CODE_BASE,
        stack_pointer: USER_STACK_TOP - 8,
        selectors,
        proof: Ring3Proof::waiting(),
    })
}

/// Kernel-side state machine proving that the exit originated at CPL3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ring3Proof {
    Waiting,
    Verified { exit_code: i32 },
    Rejected(ProofFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofFailure {
    NotRing3,
    WrongSyscall,
    WrongMarker,
}

impl Ring3Proof {
    pub const fn waiting() -> Self {
        Self::Waiting
    }

    /// `saved_cs` must be the CS pushed by the CPU's privilege transition.
    pub const fn observe_exit(
        self,
        saved_cs: u64,
        syscall_number: u64,
        marker: u64,
        exit_code: i32,
    ) -> Self {
        if !matches!(self, Self::Waiting) {
            return self;
        }
        if saved_cs & 3 != 3 {
            Self::Rejected(ProofFailure::NotRing3)
        } else if syscall_number != SYSCALL_PROCESS_EXIT {
            Self::Rejected(ProofFailure::WrongSyscall)
        } else if marker != RING3_PROOF_MARKER {
            Self::Rejected(ProofFailure::WrongMarker)
        } else {
            Self::Verified { exit_code }
        }
    }
}

/// Activate a prepared address space and enter CPL3 with `iretq`.
///
/// # Safety
///
/// Before calling, the kernel must install a DPL3 interrupt gate at vector
/// `0x80`, load the GDT that owns `process.selectors`, and ensure the TSS has a
/// valid ring-0 stack.  The address space must satisfy `UserAddressSpace`'s
/// contract.  This function does not return; the syscall handler owns exit.
pub unsafe fn launch_first_process<A: UserAddressSpace>(
    address_space: &mut A,
    process: PreparedUserProcess,
) -> Result<core::convert::Infallible, A::Error> {
    // SAFETY: delegated to the caller and the unsafe trait implementation.
    unsafe { address_space.activate()? };

    let user_ss = process.selectors.data as u64;
    let user_cs = process.selectors.code as u64;
    let user_rsp = process.stack_pointer;
    let user_rip = process.instruction_pointer;
    // IF=1; reserved bit 1 is set. IOPL remains zero, so userspace cannot use
    // port I/O even though interrupts become enabled after the transition.
    let rflags = 0x202u64;

    // SAFETY: the caller established every descriptor, mapping and IDT
    // invariant documented above. IRETQ performs the hardware CPL transition.
    unsafe {
        asm!(
            "cli",
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "iretq",
            user_ss = in(reg) user_ss,
            user_rsp = in(reg) user_rsp,
            rflags = in(reg) rflags,
            user_cs = in(reg) user_cs,
            user_rip = in(reg) user_rip,
            options(noreturn),
        )
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[derive(Default)]
    struct FakeSpace {
        mappings: Vec<(u64, PageRole)>,
        image: Vec<u8>,
        sealed: bool,
    }

    unsafe impl UserAddressSpace for FakeSpace {
        type Error = ();
        fn map_zeroed_page(&mut self, address: u64, role: PageRole) -> Result<(), ()> {
            self.mappings.push((address, role));
            Ok(())
        }
        fn write_user(&mut self, _: u64, bytes: &[u8]) -> Result<(), ()> {
            self.image.extend_from_slice(bytes);
            Ok(())
        }
        fn seal_code_page(&mut self, _: u64) -> Result<(), ()> {
            self.sealed = true;
            Ok(())
        }
        unsafe fn activate(&mut self) -> Result<(), ()> {
            Ok(())
        }
    }

    #[test]
    fn builder_separates_code_exchange_and_stack() {
        let mut space = FakeSpace::default();
        let selectors = UserModeSelectors::new(0x23, 0x1b).unwrap();
        let process = prepare_first_process(&mut space, selectors).unwrap();
        assert_eq!(process.instruction_pointer(), USER_CODE_BASE);
        assert_eq!(space.mappings.len(), 2 + USER_STACK_PAGES as usize);
        assert_eq!(space.mappings[0], (USER_CODE_BASE, PageRole::UserCode));
        assert_eq!(
            space.mappings[1],
            (USER_EXCHANGE_BASE, PageRole::SyscallExchange)
        );
        assert!(
            !space
                .mappings
                .iter()
                .any(|(address, _)| *address == USER_STACK_GUARD)
        );
        assert!(space.sealed);
        assert_eq!(space.image, FIRST_USER_IMAGE);
    }

    #[test]
    fn proof_requires_cpu_reported_cpl3_and_exact_marker() {
        let waiting = Ring3Proof::waiting();
        assert_eq!(
            waiting.observe_exit(0x08, 1, RING3_PROOF_MARKER, 0),
            Ring3Proof::Rejected(ProofFailure::NotRing3)
        );
        assert_eq!(
            waiting.observe_exit(0x23, 1, 0, 0),
            Ring3Proof::Rejected(ProofFailure::WrongMarker)
        );
        assert_eq!(
            waiting.observe_exit(0x23, 1, RING3_PROOF_MARKER, 7),
            Ring3Proof::Verified { exit_code: 7 }
        );
    }

    #[test]
    fn selectors_must_request_ring_three() {
        assert!(UserModeSelectors::new(0x08, 0x10).is_none());
        assert!(UserModeSelectors::new(0x23, 0x1b).is_some());
    }
}
