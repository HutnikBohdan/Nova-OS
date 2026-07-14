use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, exit},
};

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "uefi".into());
    if mode == "image" {
        println!("UEFI image: {}", env!("UEFI_PATH"));
        println!("BIOS image: {}", env!("BIOS_PATH"));
        return;
    }
    if mode != "uefi" && mode != "bios" {
        eprintln!("Usage: cargo run -- [uefi|bios|image]");
        exit(2);
    }

    let local_qemu = "tools/qemu/qemu-system-x86_64.exe";
    let executable = if std::path::Path::new(local_qemu).exists() {
        local_qemu
    } else {
        "qemu-system-x86_64"
    };
    let mut qemu = Command::new(executable);
    qemu.args(["-m", "512M", "-serial", "stdio", "-no-reboot"]);
    let stage = env::temp_dir().join("nova-os-qemu");
    fs::create_dir_all(&stage).expect("failed to create QEMU staging directory");
    let image = if mode == "uefi" {
        env!("UEFI_PATH")
    } else {
        env!("BIOS_PATH")
    };
    let image = stage_file(Path::new(image), &stage, "nova-os.img");

    if Path::new(local_qemu).exists() {
        let share = stage.join("share");
        stage_qemu_share(Path::new("tools/qemu/share"), &share);
        qemu.arg("-L").arg(&share);
    }
    if mode == "uefi" {
        let ovmf = Prebuilt::fetch(Source::LATEST, "target/ovmf").expect("failed to obtain OVMF");
        let code = stage_file(
            &ovmf.get_file(Arch::X64, FileType::Code),
            &stage,
            "ovmf-code.fd",
        );
        let vars = stage_file(
            &ovmf.get_file(Arch::X64, FileType::Vars),
            &stage,
            "ovmf-vars.fd",
        );
        qemu.arg("-drive")
            .arg(format!("format=raw,file={}", image.display()));
        qemu.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=0,file={},readonly=on",
            code.display()
        ));
        qemu.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=1,file={},snapshot=on",
            vars.display()
        ));
    } else {
        qemu.arg("-drive")
            .arg(format!("format=raw,file={}", image.display()));
    }
    let status = qemu.status().expect("qemu-system-x86_64 was not found");
    exit(status.code().unwrap_or(1));
}

fn stage_file(source: &Path, directory: &Path, name: &str) -> PathBuf {
    let destination = directory.join(name);
    fs::copy(source, &destination)
        .unwrap_or_else(|error| panic!("failed to stage {}: {error}", source.display()));
    destination
}

fn stage_qemu_share(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("failed to stage QEMU firmware directory");
    for entry in fs::read_dir(source).expect("failed to read local QEMU firmware") {
        let entry = entry.expect("failed to read QEMU firmware entry");
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let target = destination.join(entry.file_name());
            fs::copy(entry.path(), target).expect("failed to stage QEMU firmware file");
        }
    }
}
