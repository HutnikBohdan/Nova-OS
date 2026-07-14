use std::{
    env,
    path::{Path, PathBuf},
};

pub const RESOURCE_ROOT_ENV: &str = "NOVA_VM_LAB_HOME";

/// Stable, relocatable on-disk contract for a Nova VM Lab bundle.
///
/// Nova-VM-Lab.exe
/// qemu/qemu-system-x86_64.exe
/// qemu/qemu-img.exe
/// qemu/share/*
/// images/nova-os-bios.img
/// images/nova-os-uefi.img
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLayout {
    pub root: PathBuf,
}

impl ResourceLayout {
    pub fn discover(configured_root: &str) -> Result<Self, String> {
        let executable = env::current_exe()
            .map_err(|error| format!("Cannot locate Nova VM Lab executable: {error}"))?;
        Ok(Self::resolve(
            &executable,
            configured_root,
            env::var_os(RESOURCE_ROOT_ENV).as_deref(),
        ))
    }

    fn resolve(
        executable: &Path,
        configured_root: &str,
        env_root: Option<&std::ffi::OsStr>,
    ) -> Self {
        let configured_root = configured_root.trim();
        let root = if !configured_root.is_empty() {
            PathBuf::from(configured_root)
        } else if let Some(root) = env_root.filter(|root| !root.is_empty()) {
            PathBuf::from(root)
        } else {
            executable
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        };
        Self { root }
    }

    pub fn qemu(&self) -> PathBuf {
        self.root.join("qemu/qemu-system-x86_64.exe")
    }

    pub fn qemu_img(&self) -> PathBuf {
        self.root.join("qemu/qemu-img.exe")
    }

    pub fn qemu_share(&self) -> PathBuf {
        self.root.join("qemu/share")
    }

    pub fn bios_image(&self) -> PathBuf {
        self.root.join("images/nova-os-bios.img")
    }

    pub fn uefi_image(&self) -> PathBuf {
        self.root.join("images/nova-os-uefi.img")
    }

    pub fn missing_bundle_files(&self) -> Vec<PathBuf> {
        [
            self.qemu(),
            self.qemu_img(),
            self.qemu_share().join("edk2-x86_64-code.fd"),
            self.qemu_share().join("edk2-i386-vars.fd"),
            self.bios_image(),
            self.uefi_image(),
        ]
        .into_iter()
        .filter(|path| !path.exists())
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsStr,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn defaults_to_executable_directory() {
        let layout = ResourceLayout::resolve(
            Path::new("C:/portable/Nova-VM-Lab/Nova-VM-Lab.exe"),
            "",
            None,
        );
        assert_eq!(layout.root, PathBuf::from("C:/portable/Nova-VM-Lab"));
        assert_eq!(
            layout.qemu(),
            PathBuf::from("C:/portable/Nova-VM-Lab/qemu/qemu-system-x86_64.exe")
        );
        assert_eq!(
            layout.bios_image(),
            PathBuf::from("C:/portable/Nova-VM-Lab/images/nova-os-bios.img")
        );
    }

    #[test]
    fn configured_root_has_priority_over_environment() {
        let layout = ResourceLayout::resolve(
            Path::new("C:/app/Nova-VM-Lab.exe"),
            "D:/configured",
            Some(OsStr::new("E:/environment")),
        );
        assert_eq!(layout.root, PathBuf::from("D:/configured"));
    }

    #[test]
    fn environment_can_override_executable_directory() {
        let layout = ResourceLayout::resolve(
            Path::new("C:/app/Nova-VM-Lab.exe"),
            "",
            Some(OsStr::new("E:/portable-bundle")),
        );
        assert_eq!(layout.root, PathBuf::from("E:/portable-bundle"));
    }

    #[test]
    fn bundle_check_uses_the_documented_layout() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos();
        let root =
            env::temp_dir().join(format!("nova-vm-lab-layout-{}-{nonce}", std::process::id()));
        let layout = ResourceLayout { root: root.clone() };
        let required = [
            layout.qemu(),
            layout.qemu_img(),
            layout.qemu_share().join("edk2-x86_64-code.fd"),
            layout.qemu_share().join("edk2-i386-vars.fd"),
            layout.bios_image(),
            layout.uefi_image(),
        ];
        for path in required {
            fs::create_dir_all(path.parent().expect("required file must have a parent")).unwrap();
            fs::write(path, b"test").unwrap();
        }

        assert!(layout.missing_bundle_files().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
