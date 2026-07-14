#![no_std]
#![no_main]

mod arch;
mod ata;
mod console;
mod fs;
mod heap;
mod ipc;
mod keyboard;
mod memory;
mod mouse;
mod network;
mod operator;
mod pci;
mod process;
mod scheduler;
mod serial;
mod services;
mod user_space;
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
    services::bootstrap();
    let devices = pci::discover();
    if devices > 0 {
        serial::write_str("NOVA_PCI_READY\n");
    }
    network::initialize(boot_info);
    virtio_block::initialize(boot_info);
    let mut filesystem = fs::RamFs::new();
    filesystem.seed();
    ata::initialize_storage(&mut filesystem);
    operator::guardian_boot_proof(&mut filesystem);
    if let Some(selectors) = arch::run_ring3_proof(boot_info) {
        arch::enable_runtime_interrupts(boot_info, selectors);
    }
    operator::self_hosting_boot_proof(&mut filesystem);

    if let Some(framebuffer) = boot_info.framebuffer.as_mut() {
        console::init(framebuffer);
        let (width, height) = console::dimensions();
        mouse::init(width, height);
    }
    serial::write_str("NOVA_DESKTOP_READY\n");
    serial::write_str("NOVA_OS_SHELL_READY\n");

    operator::run(&mut filesystem)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\nKERNEL PANIC: {info}");
    serial::write_str("NOVA_OS_KERNEL_PANIC\n");
    loop {
        x86_64::instructions::hlt();
    }
}
