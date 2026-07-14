use std::{env, fs, path::PathBuf};

fn main() {
    // Cargo can otherwise consider the image publisher fresh when switching
    // between the production microkernel and the explicit legacy proof graph.
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LEGACY_MONOLITH_PROOFS");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GUARDIAN_FAULT_INJECTION");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is missing"));
    let kernel = PathBuf::from(
        env::var_os("CARGO_BIN_FILE_KERNEL_kernel").expect("kernel artifact is missing"),
    );

    let uefi = out_dir.join("nova-os-uefi.img");
    let bios = out_dir.join("nova-os-bios.img");
    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi)
        .expect("failed to create UEFI image");
    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios)
        .expect("failed to create BIOS image");

    let dist = PathBuf::from("dist");
    fs::create_dir_all(&dist).expect("failed to create dist");
    fs::copy(&uefi, dist.join("nova-os-uefi.img")).expect("failed to publish UEFI image");
    fs::copy(&bios, dist.join("nova-os-bios.img")).expect("failed to publish BIOS image");

    println!("cargo:rustc-env=UEFI_PATH={}", uefi.display());
    println!("cargo:rustc-env=BIOS_PATH={}", bios.display());
}
