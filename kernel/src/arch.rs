use crate::{
    process::{self, Ring3Proof, UserAddressSpace, UserModeSelectors},
    user_space::CurrentUserSpace,
};
use bootloader_api::BootInfo;
use core::{
    arch::{asm, global_asm},
    ptr,
};
use x86_64::{
    PrivilegeLevel, VirtAddr,
    instructions::{
        segmentation::{CS, DS, ES, SS, Segment},
        tables::load_tss,
    },
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable},
        idt::InterruptDescriptorTable,
        tss::TaskStateSegment,
    },
};

#[repr(align(16))]
struct KernelStack([u8; 32 * 1024]);

static mut KERNEL_STACK: KernelStack = KernelStack([0; 32 * 1024]);
static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();
static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();
static mut PROOF: Ring3Proof = Ring3Proof::Waiting;
static mut CRASH_WAITING: bool = false;
static mut RUNTIME_BOOT_INFO: *const BootInfo = core::ptr::null();
static mut RUNTIME_USER_CODE: u16 = 0;
static mut RUNTIME_USER_DATA: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompiledRunError {
    RuntimeUnavailable,
    Compile,
    AddressSpace,
    Prepare,
    Activate,
    Rejected,
}

global_asm!(
    r#"
.section .bss
.align 8
nova_kernel_return_rsp: .quad 0
nova_kernel_return_rip: .quad 0
.global nova_user_yields
nova_user_yields: .quad 0
.global nova_scheduler_active
nova_scheduler_active: .byte 0
.global nova_scheduler_irq_switch
nova_scheduler_irq_switch: .byte 0
.align 8
.global nova_scheduler_current_context
nova_scheduler_current_context: .quad 0
.global nova_preempt_kernel_cr3
nova_preempt_kernel_cr3: .quad 0
.section .text
.global nova_enter_ring3
nova_enter_ring3:
    mov [rip + nova_kernel_return_rsp], rsp
    lea rax, [rip + .Lring3_return]
    mov [rip + nova_kernel_return_rip], rax
    push rdi
    push rsi
    // Keep maskable IRQs disabled for this bounded proof. The scheduler will
    // enable them only after PIC/APIC vectors are installed.
    push 0x2
    push rdx
    push rcx
    iretq
.Lring3_return:
    ret

.global nova_enter_preempt_ring3
nova_enter_preempt_ring3:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [rip + nova_kernel_return_rsp], rsp
    lea rax, [rip + .Lpreempt_return]
    mov [rip + nova_kernel_return_rip], rax
    push rdi
    push rsi
    push 0x202
    push rdx
    push rcx
    iretq
.Lpreempt_return:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

.global nova_int80_stub
nova_int80_stub:
    cmp rax, 1
    je .Lring3_process_exit
    cmp rax, 2
    je .Lring3_yield
    cmp rax, 4
    je .Lring3_object_write
    cmp rax, 8
    je .Lring3_channel_send
    cmp rax, 9
    je .Lring3_channel_receive
.Lring3_legacy_exit:
    mov r8, rsi
    mov rsi, rax
    mov rdx, rdi
    mov rdi, [rsp + 8]
    mov rcx, r8
    and rsp, -16
    call nova_ring3_exit
    test al, al
    jz .Lring3_halt
    mov rsp, [rip + nova_kernel_return_rsp]
    jmp [rip + nova_kernel_return_rip]
.Lring3_halt:
    cli
1:  hlt
    jmp 1b
.Lring3_process_exit:
    cmp byte ptr [rip + nova_scheduler_active], 0
    je .Lring3_legacy_exit
    mov r8, rsi
    mov rsi, rax
    mov rdx, rdi
    mov rdi, [rsp + 8]
    mov rcx, r8
    push rax
    mov byte ptr [rip + nova_scheduler_irq_switch], 0
    call nova_scheduler_exit_from_ring3
    test rax, rax
    jz .Lscheduler_syscall_complete
    jmp .Lload_context
.Lring3_yield:
    inc qword ptr [rip + nova_user_yields]
    iretq
.Lring3_object_write:
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    push r10
    push r11
    mov rdi, [rsp + 56]
    mov rsi, [rsp + 48]
    sub rsp, 8
    call nova_ring3_object_write
    add rsp, 8
    pop r11
    pop r10
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    iretq
.Lring3_channel_send:
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    push r10
    push r11
    sub rsp, 8
    call nova_ring3_channel_send
    add rsp, 8
    pop r11
    pop r10
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    iretq
.Lring3_channel_receive:
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    push r10
    push r11
    sub rsp, 8
    call nova_ring3_channel_receive
    add rsp, 8
    pop r11
    pop r10
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    iretq

.section .bss
.align 8
.global nova_timer_ticks
nova_timer_ticks: .quad 0
.section .text
.global nova_timer_stub
nova_timer_stub:
    push rax
    inc qword ptr [rip + nova_timer_ticks]
    cmp byte ptr [rip + nova_scheduler_active], 0
    je .Ltimer_eoi
.Lscheduler_timer:
    mov byte ptr [rip + nova_scheduler_irq_switch], 1
    mov rax, [rip + nova_scheduler_current_context]
    test rax, rax
    jz .Lscheduler_complete
.Lsave_context:
    mov [rax + 0], r15
    mov [rax + 8], r14
    mov [rax + 16], r13
    mov [rax + 24], r12
    mov [rax + 32], r11
    mov [rax + 40], r10
    mov [rax + 48], r9
    mov [rax + 56], r8
    mov [rax + 64], rbp
    mov [rax + 72], rdi
    mov [rax + 80], rsi
    mov [rax + 88], rdx
    mov [rax + 96], rcx
    mov [rax + 104], rbx
    mov rdx, [rsp]
    mov [rax + 112], rdx
    mov rdx, [rsp + 8]
    mov [rax + 120], rdx
    mov rdx, [rsp + 16]
    mov [rax + 128], rdx
    mov rdx, [rsp + 24]
    mov [rax + 136], rdx
    mov rdx, [rsp + 32]
    mov [rax + 144], rdx
    mov rdx, [rsp + 40]
    mov [rax + 152], rdx
    mov rdx, cr3
    mov [rax + 160], rdx
    call nova_scheduler_on_timer
    test rax, rax
    jz .Lscheduler_complete
.Lload_context:
    mov rdx, [rax + 120]
    mov [rsp + 8], rdx
    mov rdx, [rax + 128]
    mov [rsp + 16], rdx
    mov rdx, [rax + 136]
    mov [rsp + 24], rdx
    mov rdx, [rax + 144]
    mov [rsp + 32], rdx
    mov rdx, [rax + 152]
    mov [rsp + 40], rdx
    mov rdx, [rax + 160]
    mov cr3, rdx
    mov r15, [rax + 0]
    mov r14, [rax + 8]
    mov r13, [rax + 16]
    mov r12, [rax + 24]
    mov r11, [rax + 32]
    mov r10, [rax + 40]
    mov r9, [rax + 48]
    mov r8, [rax + 56]
    mov rbp, [rax + 64]
    mov rdi, [rax + 72]
    mov rsi, [rax + 80]
    mov rcx, [rax + 96]
    mov rbx, [rax + 104]
    mov rdx, [rax + 112]
    mov [rsp], rdx
    mov rdx, [rax + 88]
    cmp byte ptr [rip + nova_scheduler_irq_switch], 0
    jne .Ltimer_eoi
    pop rax
    iretq
.Lscheduler_complete:
    mov byte ptr [rip + nova_scheduler_active], 0
    mov rax, [rip + nova_preempt_kernel_cr3]
    mov cr3, rax
    mov al, 0x20
    out 0x20, al
    mov rsp, [rip + nova_kernel_return_rsp]
    jmp [rip + nova_kernel_return_rip]
.Lscheduler_syscall_complete:
    mov byte ptr [rip + nova_scheduler_active], 0
    mov rax, [rip + nova_preempt_kernel_cr3]
    mov cr3, rax
    mov rsp, [rip + nova_kernel_return_rsp]
    jmp [rip + nova_kernel_return_rip]
.Ltimer_eoi:
    mov al, 0x20
    out 0x20, al
    pop rax
    iretq

.global nova_invalid_opcode_stub
nova_invalid_opcode_stub:
    mov rdi, [rsp + 8]
    and rsp, -16
    call nova_ring3_invalid_opcode
    test al, al
    jz .Linvalid_opcode_halt
    mov rsp, [rip + nova_kernel_return_rsp]
    jmp [rip + nova_kernel_return_rip]
.Linvalid_opcode_halt:
    cli
2:  hlt
    jmp 2b
"#
);

unsafe extern "C" {
    fn nova_enter_ring3(user_ss: u64, user_rsp: u64, user_cs: u64, user_rip: u64);
    fn nova_enter_preempt_ring3(user_ss: u64, user_rsp: u64, user_cs: u64, user_rip: u64);
    fn nova_int80_stub();
    fn nova_timer_stub();
    fn nova_invalid_opcode_stub();
    static nova_timer_ticks: u64;
    static nova_user_yields: u64;
    static mut nova_preempt_kernel_cr3: u64;
}

pub fn run_ring3_proof(boot_info: &BootInfo) -> Option<UserModeSelectors> {
    let Some(selectors) = init_tables() else {
        return None;
    };
    for _ in 0..2 {
        let Ok(mut address_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
            crate::serial::write_str("NOVA_RING3_MAP_FAILED\n");
            return None;
        };
        let Ok(process) = process::prepare_first_process(&mut address_space, selectors) else {
            crate::serial::write_str("NOVA_RING3_PREPARE_FAILED\n");
            return None;
        };
        unsafe {
            *ptr::addr_of_mut!(PROOF) = process.proof();
        }
        let selectors = process.selectors();
        if unsafe { address_space.activate() }.is_err() {
            crate::serial::write_str("NOVA_RING3_CR3_ACTIVATE_FAILED\n");
            return None;
        }
        unsafe {
            nova_enter_ring3(
                selectors.data() as u64,
                process.stack_pointer(),
                selectors.code() as u64,
                process.instruction_pointer(),
            );
        }
    }
    crate::serial::write_str("NOVA_TWO_ISOLATED_PROCESSES_OK\n");
    run_crash_isolation(boot_info, selectors);
    run_native_app(boot_info, selectors);
    Some(selectors)
}

fn run_crash_isolation(boot_info: &BootInfo, selectors: UserModeSelectors) {
    let Ok(mut address_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(process) = process::prepare_crash_process(&mut address_space, selectors) else {
        return;
    };
    unsafe {
        *ptr::addr_of_mut!(CRASH_WAITING) = true;
    }
    if unsafe { address_space.activate() }.is_err() {
        return;
    }
    unsafe {
        nova_enter_ring3(
            selectors.data() as u64,
            process.stack_pointer(),
            selectors.code() as u64,
            process.instruction_pointer(),
        );
    }
}

fn run_native_app(boot_info: &BootInfo, selectors: UserModeSelectors) {
    const MESSAGE: &[u8] = "Нативна програма Nova працює\n".as_bytes();
    let Ok(mut address_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(process) = process::prepare_console_process(&mut address_space, selectors, MESSAGE)
    else {
        return;
    };
    unsafe {
        *ptr::addr_of_mut!(PROOF) = process.proof();
    }
    if unsafe { address_space.activate() }.is_err() {
        return;
    }
    unsafe {
        nova_enter_ring3(
            selectors.data() as u64,
            process.stack_pointer(),
            selectors.code() as u64,
            process.instruction_pointer(),
        );
    }
    crate::serial::write_str("NOVA_NATIVE_APP_PROCESS_OK\n");
    run_ipc_app(boot_info, selectors);
}

fn run_ipc_app(boot_info: &BootInfo, selectors: UserModeSelectors) {
    const MESSAGE: &[u8] = b"ping";
    let Ok(mut address_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(process) = process::prepare_ipc_process(&mut address_space, selectors, MESSAGE) else {
        return;
    };
    unsafe {
        *ptr::addr_of_mut!(PROOF) = process.proof();
    }
    if unsafe { address_space.activate() }.is_err() {
        return;
    }
    unsafe {
        nova_enter_ring3(
            selectors.data() as u64,
            process.stack_pointer(),
            selectors.code() as u64,
            process.instruction_pointer(),
        );
    }
    crate::serial::write_str("NOVA_RING3_IPC_ROUNDTRIP_OK\n");
    run_compiled_app(boot_info, selectors);
}

fn run_compiled_app(boot_info: &BootInfo, selectors: UserModeSelectors) {
    let mut workspace = compiler_core::Workspace::new();
    let mut elf = [0u8; 8192];
    let Ok(image) = compiler_core::compile(
        "fn add(a: i64, b: i64) -> i64 { return a + b; } fn main() -> i64 { nova_print_byte(42); nova_yield(); return add(40, 2); }",
        &mut workspace,
        &mut elf,
    ) else {
        return;
    };
    let code = &elf[image.code_offset..image.code_offset + image.code_len];
    let entry_offset = (image.entry - compiler_core::ELF_ENTRY) as usize;
    let Ok(mut address_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(process) =
        process::prepare_compiled_process(&mut address_space, selectors, code, entry_offset)
    else {
        return;
    };
    unsafe {
        *ptr::addr_of_mut!(PROOF) = process.proof();
    }
    if unsafe { address_space.activate() }.is_err() {
        return;
    }
    unsafe {
        nova_enter_ring3(
            selectors.data() as u64,
            process.stack_pointer(),
            selectors.code() as u64,
            process.instruction_pointer(),
        );
    }
    if unsafe { ptr::read_volatile(ptr::addr_of!(PROOF)) }
        == (Ring3Proof::Verified { exit_code: 42 })
    {
        crate::serial::write_str("NOVA_SELF_HOSTED_RING3_APP_OK\n");
    }
}

pub fn enable_runtime_interrupts(boot_info: &BootInfo, selectors: UserModeSelectors) {
    unsafe {
        RUNTIME_BOOT_INFO = boot_info as *const BootInfo;
        RUNTIME_USER_CODE = selectors.code();
        RUNTIME_USER_DATA = selectors.data();
    }
    unsafe {
        let idt = &mut *ptr::addr_of_mut!(IDT);
        idt[32].set_handler_addr(VirtAddr::new(nova_timer_stub as *const () as usize as u64));
        idt.load();
        remap_pic_and_start_pit();
        asm!("sti", options(nomem, nostack));
    }
    let start = timer_ticks();
    while timer_ticks().wrapping_sub(start) < 3 {
        x86_64::instructions::hlt();
    }
    crate::serial::write_str("NOVA_PIT_TIMER_OK\n");
    run_preemption_proof(boot_info, selectors);
}

/// Compiles NovaRust source inside Nova OS and launches the resulting native
/// code in a fresh ring-3 address space.
pub fn run_compiled_source(source: &str) -> Result<i32, CompiledRunError> {
    let (boot_info, selectors) = unsafe {
        let boot_info = RUNTIME_BOOT_INFO
            .as_ref()
            .ok_or(CompiledRunError::RuntimeUnavailable)?;
        let selectors = UserModeSelectors::new(RUNTIME_USER_CODE, RUNTIME_USER_DATA)
            .ok_or(CompiledRunError::RuntimeUnavailable)?;
        (boot_info, selectors)
    };
    let mut workspace = compiler_core::Workspace::new();
    let mut elf = [0u8; 8192];
    let image = compiler_core::compile(source, &mut workspace, &mut elf)
        .map_err(|_| CompiledRunError::Compile)?;
    let code = &elf[image.code_offset..image.code_offset + image.code_len];
    let entry_offset = (image.entry - compiler_core::ELF_ENTRY) as usize;
    let mut address_space =
        unsafe { CurrentUserSpace::new(boot_info) }.map_err(|_| CompiledRunError::AddressSpace)?;
    let process =
        process::prepare_compiled_process(&mut address_space, selectors, code, entry_offset)
            .map_err(|_| CompiledRunError::Prepare)?;
    unsafe {
        *ptr::addr_of_mut!(PROOF) = process.proof();
        address_space
            .activate()
            .map_err(|_| CompiledRunError::Activate)?;
        nova_enter_ring3(
            selectors.data() as u64,
            process.stack_pointer(),
            selectors.code() as u64,
            process.instruction_pointer(),
        );
    }
    match unsafe { ptr::read_volatile(ptr::addr_of!(PROOF)) } {
        Ring3Proof::Verified { exit_code } => {
            crate::serial::write_str("NOVA_DYNAMIC_BUILD_RING3_OK\n");
            Ok(exit_code)
        }
        _ => Err(CompiledRunError::Rejected),
    }
}

fn run_preemption_proof(boot_info: &BootInfo, selectors: UserModeSelectors) {
    let frames_before = crate::memory::available();
    let Ok(mut first_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(first) = process::prepare_preempt_process(&mut first_space, selectors) else {
        return;
    };
    let Ok(mut second_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(_second) = process::prepare_preempt_process(&mut second_space, selectors) else {
        return;
    };
    let Ok(mut third_space) = (unsafe { CurrentUserSpace::new(boot_info) }) else {
        return;
    };
    let Ok(third) = process::prepare_preempt_process(&mut third_space, selectors) else {
        return;
    };
    let Some(first_counter) = first_space.physical_address(process::USER_EXCHANGE_BASE) else {
        return;
    };
    let Some(second_counter) = second_space.physical_address(process::USER_EXCHANGE_BASE) else {
        return;
    };
    let Some(third_counter) = third_space.physical_address(process::USER_EXCHANGE_BASE) else {
        return;
    };
    let Some(offset) = boot_info.physical_memory_offset.into_option() else {
        return;
    };
    let (kernel_cr3, flags) = x86_64::registers::control::Cr3::read();
    let first_context = runtime_core::CpuContext {
        instruction_pointer: first.instruction_pointer(),
        code_segment: selectors.code() as u64,
        flags: 0x202,
        stack_pointer: first.stack_pointer(),
        stack_segment: selectors.data() as u64,
        cr3: first_space.level4_address() | flags.bits(),
        ..runtime_core::CpuContext::default()
    };
    let second_context = runtime_core::CpuContext {
        instruction_pointer: _second.instruction_pointer(),
        code_segment: selectors.code() as u64,
        flags: 0x202,
        stack_pointer: _second.stack_pointer(),
        stack_segment: selectors.data() as u64,
        cr3: second_space.level4_address() | flags.bits(),
        ..runtime_core::CpuContext::default()
    };
    let third_context = runtime_core::CpuContext {
        instruction_pointer: third.instruction_pointer(),
        code_segment: selectors.code() as u64,
        flags: 0x202,
        stack_pointer: third.stack_pointer(),
        stack_segment: selectors.data() as u64,
        cr3: third_space.level4_address() | flags.bits(),
        ..runtime_core::CpuContext::default()
    };
    let first_level4 = first_space.level4_address();
    let first_owner = first_space.into_owner();
    let second_owner = second_space.into_owner();
    let third_owner = third_space.into_owner();

    if !crate::scheduler::reset(true) {
        crate::serial::write_str("NOVA_PREEMPTIVE_CR3_SWITCH_FAILED\n");
        return;
    }
    if !crate::scheduler::install(
        runtime_core::ProcessId(1),
        runtime_core::ThreadId(1),
        first_context,
        first_owner,
    ) || !crate::scheduler::install(
        runtime_core::ProcessId(2),
        runtime_core::ThreadId(2),
        second_context,
        second_owner,
    ) || !crate::scheduler::install(
        runtime_core::ProcessId(3),
        runtime_core::ThreadId(3),
        third_context,
        third_owner,
    ) {
        let _ = crate::scheduler::reclaim_all();
        crate::serial::write_str("NOVA_PREEMPTIVE_CR3_SWITCH_FAILED\n");
        return;
    }

    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_preempt_kernel_cr3),
            kernel_cr3.start_address().as_u64() | flags.bits(),
        );
        let first_scheduled = crate::scheduler::start();
        if first_scheduled.is_null() {
            crate::serial::write_str("NOVA_PREEMPTIVE_CR3_SWITCH_FAILED\n");
            return;
        }
        x86_64::registers::control::Cr3::write(
            x86_64::structures::paging::PhysFrame::containing_address(x86_64::PhysAddr::new(
                first_level4,
            )),
            flags,
        );
        nova_enter_preempt_ring3(
            selectors.data() as u64,
            first.stack_pointer(),
            selectors.code() as u64,
            first.instruction_pointer(),
        );
    }
    let first_value = unsafe { ptr::read_volatile((offset + first_counter) as *const u64) };
    let second_value = unsafe { ptr::read_volatile((offset + second_counter) as *const u64) };
    let third_value = unsafe { ptr::read_volatile((offset + third_counter) as *const u64) };
    let scheduler_passed = crate::scheduler::proof_passed();
    let exited_frames_reclaimed = crate::scheduler::reclaimed_frames() > 0;
    let expected_reuse = crate::scheduler::retired_level4();
    crate::scheduler::stop();
    let frame_reused = if let Ok(reuse_space) = unsafe { CurrentUserSpace::new(boot_info) } {
        reuse_space.level4_address() == expected_reuse
    } else {
        false
    };
    let _ = crate::scheduler::reclaim_all();
    let all_frames_reclaimed = crate::memory::available() == frames_before;
    if first_value > 0
        && second_value > 0
        && third_value > 0
        && scheduler_passed
        && exited_frames_reclaimed
        && frame_reused
        && all_frames_reclaimed
    {
        crate::serial::write_str("NOVA_PROCESS_EXIT_FRAMES_RECLAIMED_OK\n");
        crate::serial::write_str("NOVA_ADDRESS_SPACE_FRAME_REUSE_OK\n");
        crate::serial::write_str("NOVA_SCHEDULER_3_PROCESS_OK\n");
        crate::serial::write_str("NOVA_SCHEDULER_EXIT_HANDOFF_OK\n");
        crate::serial::write_str("NOVA_FULL_CONTEXT_SWITCH_OK\n");
        crate::serial::write_str("NOVA_PREEMPTIVE_CR3_SWITCH_OK\n");
    } else {
        crate::serial::write_str("NOVA_PREEMPTIVE_CR3_SWITCH_FAILED\n");
    }
}

pub fn timer_ticks() -> u64 {
    unsafe { ptr::read_volatile(ptr::addr_of!(nova_timer_ticks)) }
}

unsafe fn remap_pic_and_start_pit() {
    unsafe {
        out_u8(0x20, 0x11);
        io_wait();
        out_u8(0xa0, 0x11);
        io_wait();
        out_u8(0x21, 0x20);
        io_wait();
        out_u8(0xa1, 0x28);
        io_wait();
        out_u8(0x21, 0x04);
        io_wait();
        out_u8(0xa1, 0x02);
        io_wait();
        out_u8(0x21, 0x01);
        io_wait();
        out_u8(0xa1, 0x01);
        io_wait();
        out_u8(0x21, 0xfe);
        out_u8(0xa1, 0xff);
        let divisor = 11_932u16;
        out_u8(0x43, 0x36);
        out_u8(0x40, divisor as u8);
        out_u8(0x40, (divisor >> 8) as u8);
    }
}

unsafe fn out_u8(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
    }
}

unsafe fn io_wait() {
    unsafe {
        asm!("out 0x80, al", in("al") 0u8, options(nomem, nostack));
    }
}

fn init_tables() -> Option<UserModeSelectors> {
    unsafe {
        let stack_start = ptr::addr_of!(KERNEL_STACK.0) as *const u8 as u64;
        (*ptr::addr_of_mut!(TSS)).privilege_stack_table[0] = VirtAddr::new(stack_start + 32 * 1024);
        let gdt = &mut *ptr::addr_of_mut!(GDT);
        let kernel_code = gdt.append(Descriptor::kernel_code_segment());
        let kernel_data = gdt.append(Descriptor::kernel_data_segment());
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        let tss = gdt.append(Descriptor::tss_segment(&*ptr::addr_of!(TSS)));
        gdt.load();
        CS::set_reg(kernel_code);
        DS::set_reg(kernel_data);
        ES::set_reg(kernel_data);
        SS::set_reg(kernel_data);
        load_tss(tss);
        let idt = &mut *ptr::addr_of_mut!(IDT);
        idt[6].set_handler_addr(VirtAddr::new(
            nova_invalid_opcode_stub as *const () as usize as u64,
        ));
        idt[0x80]
            .set_handler_addr(VirtAddr::new(nova_int80_stub as *const () as usize as u64))
            .set_privilege_level(PrivilegeLevel::Ring3);
        idt.load();
        UserModeSelectors::new(user_code.0 | 3, user_data.0 | 3)
    }
}

#[unsafe(no_mangle)]
extern "C" fn nova_ring3_invalid_opcode(saved_cs: u64) -> u8 {
    if !crate::user_space::restore_kernel_address_space() {
        return 0;
    }
    unsafe {
        if saved_cs & 3 == 3 && *ptr::addr_of!(CRASH_WAITING) {
            *ptr::addr_of_mut!(CRASH_WAITING) = false;
            crate::serial::write_str("NOVA_RING3_CRASH_ISOLATED_OK\n");
            1
        } else {
            crate::serial::write_str("NOVA_KERNEL_INVALID_OPCODE\n");
            0
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn nova_ring3_object_write(address: u64, length: u64) -> u64 {
    if length > 256 {
        return u64::MAX;
    }
    let Some(bytes) = checked_user_buffer(address, length) else {
        return u64::MAX;
    };
    let Ok(text) = core::str::from_utf8(bytes) else {
        return u64::MAX;
    };
    crate::serial::write_str("NOVA_APP_OUTPUT=");
    crate::serial::write_str(text);
    0
}

fn checked_user_buffer(address: u64, length: u64) -> Option<&'static mut [u8]> {
    let end = address.checked_add(length)?;
    let page_end = process::USER_EXCHANGE_BASE + memory_core::PAGE_SIZE;
    if length == 0 || address < process::USER_EXCHANGE_BASE || end > page_end {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts_mut(address as *mut u8, length as usize) })
}

#[unsafe(no_mangle)]
extern "C" fn nova_ring3_channel_send(tag: u64, address: u64, length: u64) -> u64 {
    let Some(bytes) = checked_user_buffer(address, length) else {
        return u64::MAX;
    };
    match crate::ipc::send(1, tag as u32, bytes) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

#[unsafe(no_mangle)]
extern "C" fn nova_ring3_channel_receive(address: u64, capacity: u64) -> u64 {
    let Some(output) = checked_user_buffer(address, capacity) else {
        return u64::MAX;
    };
    crate::ipc::receive(output)
        .map(|length| length as u64)
        .unwrap_or(u64::MAX)
}

#[unsafe(no_mangle)]
extern "C" fn nova_ring3_exit(saved_cs: u64, syscall: u64, marker: u64, exit_code: i32) -> u8 {
    if !crate::user_space::restore_kernel_address_space() {
        crate::serial::write_str("NOVA_RING3_CR3_RESTORE_FAILED\n");
        return 0;
    }
    unsafe {
        let current = *ptr::addr_of!(PROOF);
        let next = current.observe_exit(saved_cs, syscall, marker, exit_code);
        *ptr::addr_of_mut!(PROOF) = next;
        if matches!(next, Ring3Proof::Verified { .. }) {
            let yields = ptr::read_volatile(ptr::addr_of!(nova_user_yields));
            if yields == 0 {
                crate::serial::write_str("NOVA_RING3_YIELD_FAILED\n");
                return 0;
            }
            crate::serial::write_str("NOVA_RING3_YIELD_OK\n");
            crate::serial::write_str("NOVA_RING3_ISOLATED_CR3_OK\n");
            crate::serial::write_str("NOVA_RING3_PROCESS_OK\n");
            1
        } else {
            crate::serial::write_str("NOVA_RING3_PROCESS_REJECTED\n");
            0
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn nova_scheduler_exit_from_ring3(
    saved_cs: u64,
    syscall: u64,
    marker: u64,
    _exit_code: i32,
) -> *mut runtime_core::CpuContext {
    if saved_cs & 3 != 3
        || syscall != runtime_core::syscall::SyscallNumber::ProcessExit as u64
        || marker != process::RING3_PROOF_MARKER
    {
        crate::serial::write_str("NOVA_RING3_PROCESS_REJECTED\n");
        crate::scheduler::stop();
        return core::ptr::null_mut();
    }
    crate::scheduler::exit_current()
}
