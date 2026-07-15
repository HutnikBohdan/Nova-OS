#![no_std]
#![no_main]

mod arch;
#[cfg(feature = "legacy-monolith-proofs")]
mod ata;
#[cfg(feature = "legacy-monolith-proofs")]
mod console;
#[cfg(feature = "legacy-monolith-proofs")]
mod fs;
mod heap;
mod ipc;
#[cfg(feature = "legacy-monolith-proofs")]
mod keyboard;
mod memory;
#[cfg(feature = "legacy-monolith-proofs")]
mod mouse;
#[cfg(feature = "legacy-monolith-proofs")]
mod network;
#[cfg(feature = "legacy-monolith-proofs")]
mod operator;
#[cfg(feature = "legacy-monolith-proofs")]
mod pci;
mod process;
mod scheduler;
mod serial;
#[cfg(feature = "legacy-monolith-proofs")]
mod services;
mod user_copy;
mod user_space;
#[cfg(feature = "legacy-monolith-proofs")]
mod virtio_block;

use bootloader_api::{BootInfo, BootloaderConfig, config::Mapping, entry_point};
use core::panic::PanicInfo;

const CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.kernel_stack_size = 256 * 1024;
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial::init();
    serial::write_str("NOVA_OS_BOOT_OK\n");
    memory::init(boot_info);
    #[cfg(feature = "legacy-monolith-proofs")]
    services::bootstrap();
    #[cfg(feature = "legacy-monolith-proofs")]
    let devices = pci::discover();
    #[cfg(feature = "legacy-monolith-proofs")]
    if devices > 0 {
        serial::write_str("NOVA_PCI_READY\n");
    }
    #[cfg(feature = "legacy-monolith-proofs")]
    network::initialize(boot_info);
    #[cfg(feature = "legacy-monolith-proofs")]
    virtio_block::initialize(boot_info);
    #[cfg(feature = "legacy-monolith-proofs")]
    let mut filesystem = fs::RamFs::new();
    #[cfg(feature = "legacy-monolith-proofs")]
    filesystem.seed();
    #[cfg(feature = "legacy-monolith-proofs")]
    ata::initialize_storage(&mut filesystem);
    #[cfg(feature = "legacy-monolith-proofs")]
    operator::guardian_boot_proof(&mut filesystem);
    if let Some(selectors) = arch::run_ring3_proof(boot_info) {
        arch::enable_runtime_interrupts(boot_info, selectors);
    }
    #[cfg(feature = "legacy-monolith-proofs")]
    operator::self_hosting_boot_proof(&mut filesystem);

    #[cfg(feature = "legacy-monolith-proofs")]
    if let Some(framebuffer) = boot_info.framebuffer.as_mut() {
        console::init(framebuffer);
        let (width, height) = console::dimensions();
        mouse::init(width, height);
    }
    #[cfg(feature = "legacy-monolith-proofs")]
    serial::write_str("NOVA_DESKTOP_READY\n");
    #[cfg(feature = "legacy-monolith-proofs")]
    serial::write_str("NOVA_OS_SHELL_READY\n");

    #[cfg(feature = "legacy-monolith-proofs")]
    {
        operator::run(&mut filesystem)
    }

    #[cfg(not(feature = "legacy-monolith-proofs"))]
    {
        serial::write_str("NOVA_MICROKERNEL_READY\n");
        loop {
            x86_64::instructions::hlt();
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial::write_fmt(format_args!("KERNEL PANIC: {info}\n"));
    serial::write_str("NOVA_OS_KERNEL_PANIC\n");
    loop {
        x86_64::instructions::hlt();
    }
}
