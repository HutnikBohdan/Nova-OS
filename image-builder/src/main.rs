use std::{env, fs, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let Some(kernel) = env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: nova-image-builder <kernel-elf> [output-directory]");
        return ExitCode::from(2);
    };
    let output = env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist"));
    if !kernel.is_file() {
        eprintln!("kernel not found: {}", kernel.display());
        return ExitCode::from(2);
    }
    if let Err(error) = fs::create_dir_all(&output) {
        eprintln!("failed to create {}: {error}", output.display());
        return ExitCode::FAILURE;
    }
    let bios = output.join("nova-os-bios.img");
    let uefi = output.join("nova-os-uefi.img");
    if let Err(error) = bootloader::BiosBoot::new(&kernel).create_disk_image(&bios) {
        eprintln!("failed to build BIOS image: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = bootloader::UefiBoot::new(&kernel).create_disk_image(&uefi) {
        eprintln!("failed to build UEFI image: {error}");
        return ExitCode::FAILURE;
    }
    println!("BIOS: {}", bios.display());
    println!("UEFI: {}", uefi.display());
    ExitCode::SUCCESS
}
