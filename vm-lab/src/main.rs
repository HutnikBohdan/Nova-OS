#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod resources;
mod vm;

use config::{Firmware, VmConfig};
use eframe::egui;
use resources::ResourceLayout;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use vm::{LaunchFiles, VmRuntime};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([850.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Nova VM Lab",
        options,
        Box::new(|cc| Ok(Box::new(VmLab::new(cc)))),
    )
}

struct VmLab {
    config: VmConfig,
    runtime: VmRuntime,
    root: PathBuf,
    status: String,
    log_view: String,
    show_serial: bool,
}

impl VmLab {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let root = config::data_root();
        let _ = fs::create_dir_all(root.join("disks"));
        let mut config = config::load(&root.join("profile.json"));
        if config.disk_path.is_empty() {
            config.disk_path = root.join("disks/test-os.qcow2").display().to_string();
        }
        Self {
            config,
            runtime: VmRuntime::new(&root.join("logs")),
            root,
            status: "Готово. Виберіть ISO або відкрийте профіль Nova OS.".into(),
            log_view: String::new(),
            show_serial: true,
        }
    }

    fn resources(&self) -> Result<ResourceLayout, String> {
        ResourceLayout::discover(&self.config.resource_root)
    }

    fn nova_profile(&mut self) {
        self.config.name = "Nova OS Debug".into();
        self.config.media_path = self
            .resources()
            .map(|resources| resources.bios_image().display().to_string())
            .unwrap_or_default();
        self.config.disk_path.clear();
        self.config.memory_mb = 512;
        self.config.cpu_count = 1;
        self.config.firmware = Firmware::Bios;
        self.config.boot_from_media = true;
        self.config.temporary_snapshot = true;
        self.config.network = false;
        self.status = "Профіль Nova OS завантажено.".into();
    }

    fn create_disk(&mut self) {
        let result = self.resources().and_then(|resources| {
            vm::create_disk(
                &resources.qemu_img(),
                Path::new(&self.config.disk_path),
                self.config.disk_size_gb,
            )
        });
        match result {
            Ok(message) if message.is_empty() => self.status = "QCOW2-диск створено.".into(),
            Ok(message) => self.status = message,
            Err(error) => self.status = format!("Помилка диска: {error}"),
        }
    }

    fn stage_media(&self) -> Result<Option<PathBuf>, String> {
        let value = self.config.media_path.trim();
        if value.is_empty() {
            return Ok(None);
        }
        let source = PathBuf::from(value);
        if !source.exists() {
            return Err(format!("Файл не знайдено: {}", source.display()));
        }
        if source.to_string_lossy().is_ascii() {
            return Ok(Some(source));
        }
        let extension = source.extension().and_then(|v| v.to_str()).unwrap_or("img");
        let cache = self.root.join("cache");
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let target = cache.join(format!("boot-media.{extension}"));
        fs::copy(&source, &target)
            .map_err(|e| format!("Не вдалося скопіювати носій в ASCII cache: {e}"))?;
        Ok(Some(target))
    }

    fn mutable_disk(&self) -> Result<Option<PathBuf>, String> {
        let value = self.config.disk_path.trim();
        if value.is_empty() {
            return Ok(None);
        }
        let disk = PathBuf::from(value);
        if !disk.exists() {
            return Err("Спочатку створіть системний QCOW2-диск.".into());
        }
        if !disk.to_string_lossy().is_ascii() {
            return Err("QEMU на Windows не працює надійно з кириличним шляхом диска. Збережіть диск у запропонованій папці NovaVmLab.".into());
        }
        Ok(Some(disk))
    }

    fn stage_firmware(&self) -> Result<(PathBuf, Option<PathBuf>, Option<PathBuf>), String> {
        let source = self.resources()?.qemu_share();
        let share = self.root.join("qemu-share");
        fs::create_dir_all(&share).map_err(|e| e.to_string())?;
        for entry in fs::read_dir(&source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                let destination = share.join(entry.file_name());
                if !destination.exists() {
                    fs::copy(entry.path(), destination).map_err(|e| e.to_string())?;
                }
            }
        }
        if self.config.firmware == Firmware::Uefi {
            let code = share.join("edk2-x86_64-code.fd");
            let template = share.join("edk2-i386-vars.fd");
            let vars = self.root.join("uefi-vars.fd");
            if !vars.exists() {
                fs::copy(template, &vars).map_err(|e| e.to_string())?;
            }
            Ok((share, Some(code), Some(vars)))
        } else {
            Ok((share, None, None))
        }
    }

    fn start(&mut self) {
        let result = (|| {
            let qemu = self.resources()?.qemu();
            if !qemu.exists() {
                return Err(format!(
                    "QEMU не знайдено в переносному bundle: {}",
                    qemu.display()
                ));
            }
            let media = self.stage_media()?;
            let disk = self.mutable_disk()?;
            let (share, code, vars) = self.stage_firmware()?;
            let serial_log = self.runtime.serial_log.clone();
            let files = LaunchFiles {
                media: media.as_deref(),
                disk: disk.as_deref(),
                firmware_code: code.as_deref(),
                firmware_vars: vars.as_deref(),
                firmware_share: &share,
                serial_log: &serial_log,
            };
            vm::launch(&qemu, &self.config, &files, &mut self.runtime)
        })();
        self.status = match result {
            Ok(()) => "VM запущена. Вікно емулятора відкрито окремо.".into(),
            Err(error) => format!("Помилка запуску: {error}"),
        };
        let _ = config::save(&self.root.join("profile.json"), &self.config);
    }

    fn refresh_logs(&mut self) {
        let path = if self.show_serial {
            &self.runtime.serial_log
        } else {
            &self.runtime.error_log
        };
        self.log_view = vm::tail(path, 128 * 1024);
    }

    fn header(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Nova VM Lab");
            ui.separator();
            ui.label(if running {
                "● VM працює"
            } else {
                "○ VM зупинена"
            });
            if ui.button("Профіль Nova OS").clicked() {
                self.nova_profile();
            }
            if ui.button("Відкрити папку даних").clicked() {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(&self.root)
                    .spawn();
            }
        });
    }

    fn settings(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.heading("Віртуальний комп'ютер");
        ui.label("Назва");
        ui.text_edit_singleline(&mut self.config.name);
        ui.label("Каталог ресурсів (порожньо = каталог Nova-VM-Lab.exe)");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.config.resource_root);
            if ui.button("…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.config.resource_root = path.display().to_string();
            }
            if ui.button("Перевірити bundle").clicked() {
                self.status = match self.resources() {
                    Ok(resources) => {
                        let missing = resources.missing_bundle_files();
                        if missing.is_empty() {
                            format!("Bundle готовий: {}", resources.root.display())
                        } else {
                            let files = missing
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("Bundle неповний. Відсутні: {files}")
                        }
                    }
                    Err(error) => error,
                };
            }
        });
        ui.add_space(8.0);
        ui.label("ISO або завантажувальний IMG");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.config.media_path);
            if ui.button("…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("OS images", &["iso", "img", "raw"])
                    .pick_file()
            {
                self.config.media_path = path.display().to_string();
            }
        });
        ui.label("Системний QCOW2-диск");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.config.disk_path);
            if ui.button("…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("QEMU disk", &["qcow2"])
                    .save_file()
            {
                self.config.disk_path = path.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.config.disk_size_gb)
                    .range(1..=2048)
                    .suffix(" GB"),
            );
            if ui.button("Створити диск").clicked() {
                self.create_disk();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("RAM");
            ui.add(
                egui::DragValue::new(&mut self.config.memory_mb)
                    .range(256..=65536)
                    .suffix(" MB"),
            );
            ui.label("CPU");
            ui.add(egui::DragValue::new(&mut self.config.cpu_count).range(1..=32));
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.config.firmware, Firmware::Uefi, "UEFI");
            ui.radio_value(&mut self.config.firmware, Firmware::Bios, "BIOS");
        });
        ui.checkbox(
            &mut self.config.boot_from_media,
            "Завантажуватися з інсталяційного носія",
        );
        ui.checkbox(
            &mut self.config.temporary_snapshot,
            "Тимчасовий snapshot — не записувати зміни",
        );
        ui.checkbox(&mut self.config.network, "Мережа NAT");
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("▶ Запустити / встановити"))
                .clicked()
            {
                self.start();
            }
            if ui
                .add_enabled(running, egui::Button::new("■ Зупинити"))
                .clicked()
            {
                self.status = match self.runtime.stop() {
                    Ok(()) => "VM зупинена.".into(),
                    Err(error) => error,
                };
            }
            if ui
                .add_enabled(running, egui::Button::new("↻ Перезапустити"))
                .clicked()
            {
                let _ = self.runtime.stop();
                self.start();
            }
        });
        ui.add_space(10.0);
        ui.label(egui::RichText::new(&self.status).color(egui::Color32::LIGHT_BLUE));
        ui.separator();
        ui.small("Після інсталяції вимкніть завантаження з носія та перезапустіть VM. Snapshot-режим не записує зміни.");
    }

    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Діагностика");
            ui.selectable_value(&mut self.show_serial, true, "Serial log");
            ui.selectable_value(&mut self.show_serial, false, "QEMU errors");
            if ui.button("Очистити вигляд").clicked() {
                self.log_view.clear();
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.log_view)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(35)
                        .interactive(false),
                );
            });
    }
}

impl eframe::App for VmLab {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.runtime.is_running();
        self.refresh_logs();
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let running = self.runtime.is_running();
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            self.header(ui, running);
            ui.separator();
            ui.columns(2, |columns| {
                self.settings(&mut columns[0], running);
                self.diagnostics(&mut columns[1]);
            });
        });
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        let _ = config::save(&self.root.join("profile.json"), &self.config);
    }
}
