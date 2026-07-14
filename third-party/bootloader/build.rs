use std::path::{Path, PathBuf};
use std::process::Command;
const BOOTLOADER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn stage_path(package: &str, upstream_relative: &[&str]) -> PathBuf {
    if let Some(vendor) = std::env::var_os("NOVA_VENDOR_DIR") {
        let candidate = PathBuf::from(vendor).join(format!("{package}-{BOOTLOADER_VERSION}"));
        if candidate.exists() {
            return candidate;
        }
    }

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for component in upstream_relative {
        path.push(component);
    }
    path
}

fn use_isolated_stage_target(cmd: &mut Command, out_dir: &Path) {
    // Never point nested Cargo at the parent's target directory: the parent
    // holds its lock while this build script runs. Keeping the stage target
    // below OUT_DIR avoids that deadlock and stays inside the approved build
    // tree on locked-down Windows hosts.
    cmd.arg("--target-dir").arg(out_dir.join("stage-target"));
}

fn main() {
    #[cfg(not(feature = "uefi"))]
    fn uefi_main() {}
    #[cfg(not(feature = "bios"))]
    fn bios_main() {}

    // Nova intentionally builds stages sequentially. Upstream's parallel
    // `cargo install` calls contend on the same offline Cargo home/target and
    // can deadlock on Windows file locks during clean-clone builds.
    uefi_main();
    bios_main();
}

#[cfg(feature = "bios")]
fn bios_main() {
    let bios_boot_sector_path = build_bios_boot_sector();
    let bios_stage_2_path = build_bios_stage_2();
    let bios_stage_3_path = build_bios_stage_3();
    let bios_stage_4_path = build_bios_stage_4();

    println!(
        "cargo:rustc-env=BIOS_BOOT_SECTOR_PATH={}",
        bios_boot_sector_path.display()
    );
    println!(
        "cargo:rustc-env=BIOS_STAGE_2_PATH={}",
        bios_stage_2_path.display()
    );
    println!(
        "cargo:rustc-env=BIOS_STAGE_3_PATH={}",
        bios_stage_3_path.display()
    );
    println!(
        "cargo:rustc-env=BIOS_STAGE_4_PATH={}",
        bios_stage_4_path.display()
    );
}

#[cfg(feature = "uefi")]
fn uefi_main() {
    let uefi_path = build_uefi_bootloader();
    println!(
        "cargo:rustc-env=UEFI_BOOTLOADER_PATH={}",
        uefi_path.display()
    );
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "uefi")]
fn build_uefi_bootloader() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("install").arg("bootloader-x86_64-uefi");
    use_isolated_stage_target(&mut cmd, &out_dir);
    let local_path = stage_path("bootloader-x86_64-uefi", &["uefi"]);
    if local_path.exists() {
        // local build
        cmd.arg("--path").arg(&local_path);
        println!("cargo:rerun-if-changed={}", local_path.display());
    } else {
        cmd.arg("--version").arg(BOOTLOADER_VERSION);
    }
    cmd.arg("--locked");
    cmd.arg("--target").arg("x86_64-unknown-uefi");
    cmd.arg("-Zbuild-std=core")
        .arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(&out_dir);
    cmd.arg("-vv");
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    let status = cmd
        .status()
        .expect("failed to run cargo install for uefi bootloader");
    if status.success() {
        let path = out_dir.join("bin").join("bootloader-x86_64-uefi.efi");
        assert!(
            path.exists(),
            "uefi bootloader executable does not exist after building"
        );
        path
    } else {
        panic!("failed to build uefi bootloader");
    }
}

// dummy implementation because docsrs builds have no network access.
// This will put an empty file in out_dir and return its path.
#[cfg(docsrs_dummy_build)]
#[cfg(feature = "uefi")]
fn build_uefi_bootloader() -> PathBuf {
    use std::fs::File;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out_dir.join("bootloader-dummy-bootloader-uefi");

    if File::create(&path).is_err() {
        panic!("Failed to create dummy uefi bootloader");
    }
    assert!(
        path.exists(),
        "uefi bootloader dummy file does not exist after file creation"
    );

    path
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "bios")]
fn build_bios_boot_sector() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("install").arg("bootloader-x86_64-bios-boot-sector");
    use_isolated_stage_target(&mut cmd, &out_dir);
    let local_path = stage_path(
        "bootloader-x86_64-bios-boot-sector",
        &["bios", "boot_sector"],
    );
    if local_path.exists() {
        // local build
        cmd.arg("--path").arg(&local_path);
        println!("cargo:rerun-if-changed={}", local_path.display());
    } else {
        cmd.arg("--version").arg(BOOTLOADER_VERSION);
    }
    cmd.arg("--locked");
    cmd.arg("--target")
        .arg("i386-code16-boot-sector.json")
        .arg("-Zjson-target-spec");
    cmd.arg("--profile").arg("stage-1");
    cmd.arg("-Zbuild-std=core")
        .arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(&out_dir);
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTC_WORKSPACE_WRAPPER"); // used by clippy
    let status = cmd
        .status()
        .expect("failed to run cargo install for bios bootsector");
    let elf_path = if status.success() {
        let path = out_dir
            .join("bin")
            .join("bootloader-x86_64-bios-boot-sector");
        assert!(
            path.exists(),
            "bios boot sector executable does not exist after building"
        );
        path
    } else {
        panic!("failed to build bios boot sector");
    };
    convert_elf_to_bin(elf_path)
}

// dummy implementation because docsrs builds have no network access.
// This will put an empty file in out_dir and return its path.
#[cfg(docsrs_dummy_build)]
#[cfg(feature = "bios")]
fn build_bios_boot_sector() -> PathBuf {
    use std::fs::File;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out_dir.join("bootloader-dummy-bios-boot-sector");

    if File::create(&path).is_err() {
        panic!("Failed to create dummy bios boot sector");
    }
    assert!(
        path.exists(),
        "bios boot sector dummy file does not exist after file creation"
    );

    path
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "bios")]
fn build_bios_stage_2() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("install").arg("bootloader-x86_64-bios-stage-2");
    use_isolated_stage_target(&mut cmd, &out_dir);
    let local_path = stage_path(
        "bootloader-x86_64-bios-stage-2",
        &["bios", "stage-2"],
    );
    if local_path.exists() {
        // local build
        cmd.arg("--path").arg(&local_path);
        println!("cargo:rerun-if-changed={}", local_path.display());
        println!(
            "cargo:rerun-if-changed={}",
            local_path.with_file_name("common").display()
        );
    } else {
        cmd.arg("--version").arg(BOOTLOADER_VERSION);
    }
    cmd.arg("--locked");
    cmd.arg("--target")
        .arg("i386-code16-stage-2.json")
        .arg("-Zjson-target-spec");
    cmd.arg("--profile").arg("stage-2");
    cmd.arg("-Zbuild-std=core")
        .arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(&out_dir);
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTC_WORKSPACE_WRAPPER"); // used by clippy
    let status = cmd
        .status()
        .expect("failed to run cargo install for bios second stage");
    let elf_path = if status.success() {
        let path = out_dir.join("bin").join("bootloader-x86_64-bios-stage-2");
        assert!(
            path.exists(),
            "bios second stage executable does not exist after building"
        );
        path
    } else {
        panic!("failed to build bios second stage");
    };
    convert_elf_to_bin(elf_path)
}

// dummy implementation because docsrs builds have no network access.
// This will put an empty file in out_dir and return its path.
#[cfg(docsrs_dummy_build)]
#[cfg(feature = "bios")]
fn build_bios_stage_2() -> PathBuf {
    use std::fs::File;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out_dir.join("bootloader-dummy-bios-stage-2");

    if File::create(&path).is_err() {
        panic!("Failed to create dummy bios second stage");
    }
    assert!(
        path.exists(),
        "bios second stage dummy file does not exist after file creation"
    );

    path
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "bios")]
fn build_bios_stage_3() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("install").arg("bootloader-x86_64-bios-stage-3");
    use_isolated_stage_target(&mut cmd, &out_dir);
    let local_path = stage_path(
        "bootloader-x86_64-bios-stage-3",
        &["bios", "stage-3"],
    );
    if local_path.exists() {
        // local build
        cmd.arg("--path").arg(&local_path);
        println!("cargo:rerun-if-changed={}", local_path.display());
    } else {
        cmd.arg("--version").arg(BOOTLOADER_VERSION);
    }
    cmd.arg("--locked");
    cmd.arg("--target")
        .arg("i686-stage-3.json")
        .arg("-Zjson-target-spec");
    cmd.arg("--profile").arg("stage-3");
    cmd.arg("-Zbuild-std=core")
        .arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(&out_dir);
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTC_WORKSPACE_WRAPPER"); // used by clippy
    let status = cmd
        .status()
        .expect("failed to run cargo install for bios stage-3");
    let elf_path = if status.success() {
        let path = out_dir.join("bin").join("bootloader-x86_64-bios-stage-3");
        assert!(
            path.exists(),
            "bios stage-3 executable does not exist after building"
        );
        path
    } else {
        panic!("failed to build bios stage-3");
    };
    convert_elf_to_bin(elf_path)
}

// dummy implementation because docsrs builds have no network access.
// This will put an empty file in out_dir and return its path.
#[cfg(docsrs_dummy_build)]
#[cfg(feature = "bios")]
fn build_bios_stage_3() -> PathBuf {
    use std::fs::File;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out_dir.join("bootloader-dummy-bios-stage-3");

    if File::create(&path).is_err() {
        panic!("Failed to create dummy bios stage-3");
    }
    assert!(
        path.exists(),
        "bios stage-3 dummy file does not exist after file creation"
    );

    path
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "bios")]
fn build_bios_stage_4() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("install").arg("bootloader-x86_64-bios-stage-4");
    use_isolated_stage_target(&mut cmd, &out_dir);
    let local_path = stage_path(
        "bootloader-x86_64-bios-stage-4",
        &["bios", "stage-4"],
    );
    if local_path.exists() {
        // local build
        cmd.arg("--path").arg(&local_path);
        println!("cargo:rerun-if-changed={}", local_path.display());
    } else {
        cmd.arg("--version").arg(BOOTLOADER_VERSION);
    }
    cmd.arg("--locked");
    cmd.arg("--target")
        .arg("x86_64-stage-4.json")
        .arg("-Zjson-target-spec");
    cmd.arg("--profile").arg("stage-4");
    cmd.arg("-Zbuild-std=core")
        .arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(&out_dir);
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTC_WORKSPACE_WRAPPER"); // used by clippy
    let status = cmd
        .status()
        .expect("failed to run cargo install for bios stage-4");
    let elf_path = if status.success() {
        let path = out_dir.join("bin").join("bootloader-x86_64-bios-stage-4");
        assert!(
            path.exists(),
            "bios stage-4 executable does not exist after building"
        );
        path
    } else {
        panic!("failed to build bios stage-4");
    };

    convert_elf_to_bin(elf_path)
}

// dummy implementation because docsrs builds have no network access.
// This will put an empty file in out_dir and return its path.
#[cfg(docsrs_dummy_build)]
#[cfg(feature = "bios")]
fn build_bios_stage_4() -> PathBuf {
    use std::fs::File;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out_dir.join("bootloader-dummy-bios-stage-4");

    if File::create(&path).is_err() {
        panic!("Failed to create dummy bios stage-4");
    }
    assert!(
        path.exists(),
        "bios stage-4 dummy file does not exist after file creation"
    );

    path
}

#[cfg(not(docsrs_dummy_build))]
#[cfg(feature = "bios")]
fn convert_elf_to_bin(elf_path: PathBuf) -> PathBuf {
    let flat_binary_path = elf_path.with_extension("bin");

    let llvm_tools = llvm_tools::LlvmTools::new().expect("failed to get llvm tools");
    let objcopy = llvm_tools
        .tool(&llvm_tools::exe("llvm-objcopy"))
        .expect("LlvmObjcopyNotFound");

    // convert first stage to binary
    let mut cmd = Command::new(objcopy);
    cmd.arg("-I").arg("elf64-x86-64");
    cmd.arg("-O").arg("binary");
    cmd.arg("--binary-architecture=i386:x86-64");
    cmd.arg(&elf_path);
    cmd.arg(&flat_binary_path);
    let output = cmd
        .output()
        .expect("failed to execute llvm-objcopy command");
    if !output.status.success() {
        panic!(
            "objcopy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    flat_binary_path
}
