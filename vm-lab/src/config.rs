use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Firmware {
    Bios,
    Uefi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VmConfig {
    pub name: String,
    /// Optional bundle root. When empty, resources are resolved next to the executable.
    pub resource_root: String,
    pub media_path: String,
    pub disk_path: String,
    pub memory_mb: u32,
    pub cpu_count: u8,
    pub disk_size_gb: u32,
    pub firmware: Firmware,
    pub boot_from_media: bool,
    pub temporary_snapshot: bool,
    pub network: bool,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            name: "Test OS".into(),
            resource_root: String::new(),
            media_path: String::new(),
            disk_path: String::new(),
            memory_mb: 2048,
            cpu_count: 2,
            disk_size_gb: 32,
            firmware: Firmware::Uefi,
            boot_from_media: true,
            temporary_snapshot: false,
            network: true,
        }
    }
}

pub fn load(path: &Path) -> VmConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, config: &VmConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("NovaVmLab")
}
