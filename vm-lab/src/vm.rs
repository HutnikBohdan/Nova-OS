use crate::config::{Firmware, VmConfig};
use std::{
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub struct VmRuntime {
    pub child: Option<Child>,
    pub serial_log: PathBuf,
    pub error_log: PathBuf,
    pub last_exit: Option<i32>,
}

impl VmRuntime {
    pub fn new(log_dir: &Path) -> Self {
        let _ = fs::create_dir_all(log_dir);
        Self {
            child: None,
            serial_log: log_dir.join("serial.log"),
            error_log: log_dir.join("qemu-error.log"),
            last_exit: None,
        }
    }

    pub fn is_running(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.last_exit = status.code();
                self.child = None;
                false
            }
            Ok(None) => true,
            Err(_) => false,
        }
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            child.kill().map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

impl Drop for VmRuntime {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn create_disk(qemu_img: &Path, path: &Path, size_gb: u32) -> Result<String, String> {
    if path.exists() {
        return Err("Диск уже існує; виберіть інше ім'я.".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let output = Command::new(qemu_img)
        .args(["create", "-f", "qcow2"])
        .arg(path)
        .arg(format!("{}G", size_gb.clamp(1, 2048)))
        .output()
        .map_err(|e| format!("Не вдалося запустити qemu-img: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

pub struct LaunchFiles<'a> {
    pub media: Option<&'a Path>,
    pub disk: Option<&'a Path>,
    pub firmware_code: Option<&'a Path>,
    pub firmware_vars: Option<&'a Path>,
    pub firmware_share: &'a Path,
    pub serial_log: &'a Path,
}

pub fn build_args(config: &VmConfig, files: &LaunchFiles<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-name".into(),
        config.name.clone().into(),
        "-machine".into(),
        "q35,accel=tcg".into(),
        "-m".into(),
        config.memory_mb.clamp(256, 65536).to_string().into(),
        "-smp".into(),
        config.cpu_count.clamp(1, 32).to_string().into(),
        "-L".into(),
        files.firmware_share.as_os_str().into(),
        "-serial".into(),
        format!("file:{}", files.serial_log.display()).into(),
        "-display".into(),
        "sdl".into(),
        "-no-reboot".into(),
    ];

    if config.firmware == Firmware::Uefi {
        if let (Some(code), Some(vars)) = (files.firmware_code, files.firmware_vars) {
            args.extend([
                "-drive".into(),
                format!(
                    "if=pflash,format=raw,unit=0,file={},readonly=on",
                    code.display()
                )
                .into(),
                "-drive".into(),
                format!("if=pflash,format=raw,unit=1,file={}", vars.display()).into(),
            ]);
        }
    }

    if let Some(disk) = files.disk {
        args.extend([
            "-drive".into(),
            format!("file={},if=virtio,format=qcow2", disk.display()).into(),
        ]);
    }

    if let Some(media) = files.media {
        let is_iso = media
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"));
        if is_iso {
            args.extend([
                "-drive".into(),
                format!("file={},media=cdrom,readonly=on", media.display()).into(),
            ]);
            if config.boot_from_media {
                args.extend(["-boot".into(), "order=d,menu=on".into()]);
            }
        } else {
            args.extend([
                "-drive".into(),
                format!("file={},format=raw", media.display()).into(),
            ]);
            if config.boot_from_media {
                args.extend(["-boot".into(), "order=c,menu=on".into()]);
            }
        }
    }

    if config.temporary_snapshot {
        args.push("-snapshot".into());
    }
    if config.network {
        args.extend(["-nic".into(), "user,model=e1000e".into()]);
    } else {
        args.extend(["-nic".into(), "none".into()]);
    }
    args
}

pub fn launch(
    qemu: &Path,
    config: &VmConfig,
    files: &LaunchFiles<'_>,
    runtime: &mut VmRuntime,
) -> Result<(), String> {
    if runtime.is_running() {
        return Err("VM уже запущена.".into());
    }
    let _ = fs::remove_file(&runtime.serial_log);
    let _ = fs::remove_file(&runtime.error_log);
    let error = File::create(&runtime.error_log).map_err(|e| e.to_string())?;
    let mut command = Command::new(qemu);
    command.args(build_args(config, files));
    command.stdout(Stdio::null()).stderr(Stdio::from(error));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    runtime.child = Some(
        command
            .spawn()
            .map_err(|e| format!("Не вдалося запустити QEMU: {e}"))?,
    );
    Ok(())
}

pub fn tail(path: &Path, limit: usize) -> String {
    let Ok(data) = fs::read(path) else {
        return String::new();
    };
    let start = data.len().saturating_sub(limit);
    String::from_utf8_lossy(&data[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files<'a>(media: &'a Path, disk: &'a Path) -> LaunchFiles<'a> {
        LaunchFiles {
            media: Some(media),
            disk: Some(disk),
            firmware_code: None,
            firmware_vars: None,
            firmware_share: Path::new("share"),
            serial_log: Path::new("serial.log"),
        }
    }

    #[test]
    fn iso_is_attached_as_cdrom() {
        let config = VmConfig::default();
        let args = build_args(
            &config,
            &files(Path::new("installer.iso"), Path::new("system.qcow2")),
        );
        let text = args
            .iter()
            .map(|v| v.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("media=cdrom"));
        assert!(text.contains("order=d"));
        assert!(text.contains("format=qcow2"));
    }

    #[test]
    fn snapshot_and_no_network_are_explicit() {
        let config = VmConfig {
            temporary_snapshot: true,
            network: false,
            ..Default::default()
        };
        let args = build_args(
            &config,
            &files(Path::new("os.img"), Path::new("system.qcow2")),
        );
        assert!(args.iter().any(|v| v == "-snapshot"));
        assert!(args.iter().any(|v| v == "none"));
    }
}
