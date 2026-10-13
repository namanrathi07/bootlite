use bootlite_core::{flash_and_verify, list_safe_usb_drives, UsbDrive};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 430.0])
            .with_resizable(false)
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "BootLite — USB Image Flasher",
        options,
        Box::new(|_cc| Ok(Box::new(BootLiteApp::new()))),
    )
}

#[derive(Clone)]
enum JobStatus {
    Idle,
    Running {
        phase: String,
        progress: f32,
        speed_mbps: f64,
        bytes_done: u64,
        total_bytes: u64,
    },
    Success(String),
    Error(String),
}

struct BootLiteApp {
    image_path: Option<PathBuf>,
    drives: Vec<UsbDrive>,
    selected_drive_idx: Option<usize>,
    allow_virtual: bool,
    show_confirm_modal: bool,
    cancel_flag: Arc<AtomicBool>,
    status: Arc<Mutex<JobStatus>>,
}

impl BootLiteApp {
    fn new() -> Self {
        let mut app = Self {
            image_path: None,
            drives: Vec::new(),
            selected_drive_idx: None,
            allow_virtual: false,
            show_confirm_modal: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(JobStatus::Idle)),
        };
        app.refresh_drives();
        app
    }

    fn refresh_drives(&mut self) {
        self.drives = list_safe_usb_drives(self.allow_virtual).unwrap_or_default();
        if self.drives.is_empty() {
            self.selected_drive_idx = None;
        } else if self.selected_drive_idx.is_none()
            || self.selected_drive_idx.unwrap() >= self.drives.len()
        {
            self.selected_drive_idx = Some(0);
        }
    }

    fn start_flash(&mut self) {
        let Some(image_path) = self.image_path.clone() else { return };
        let Some(idx) = self.selected_drive_idx else { return };
        let Some(drive) = self.drives.get(idx).cloned() else { return };

        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel_clone = Arc::clone(&self.cancel_flag);
        let status_clone = Arc::clone(&self.status);

        *status_clone.lock().unwrap() = JobStatus::Running {
            phase: "Starting...".into(),
            progress: 0.0,
            speed_mbps: 0.0,
            bytes_done: 0,
            total_bytes: 1,
        };

        thread::spawn(move || {
            let result = flash_and_verify(&image_path, &drive, Some(&cancel_clone), |prog| {
                let fraction = if prog.total_bytes > 0 {
                    (prog.bytes_processed as f32 / prog.total_bytes as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                *status_clone.lock().unwrap() = JobStatus::Running {
                    phase: prog.phase.to_string(),
                    progress: fraction,
                    speed_mbps: prog.speed_mbps,
                    bytes_done: prog.bytes_processed,
                    total_bytes: prog.total_bytes,
                };
            });

            match result {
                Ok(hash) => *status_clone.lock().unwrap() = JobStatus::Success(hash),
                Err(err) => *status_clone.lock().unwrap() = JobStatus::Error(format!("{:#}", err)),
            }
        });
    }
}

impl eframe::App for BootLiteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let current_status = self.status.lock().unwrap().clone();
        let is_running = matches!(current_status, JobStatus::Running { .. });

        if is_running {
            ctx.request_repaint();
        }

        // Handle Drag-and-Drop files onto the window
        if !is_running {
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped.first() {
                if let Some(path) = &file.path {
                    self.image_path = Some(path.clone());
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("BootLite");
            ui.label("Fast, zero-bloat USB image flasher with SHA-256 read-back verification.");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(10.0);

            // STEP 1: Select Image
            ui.strong("1. Source Image (.iso, .img, .dmg, .gz) — or Drag & Drop into Window");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!is_running, |ui| {
                    if ui.button("Browse Image...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Disk Images", &["iso", "img", "dmg", "raw", "gz"])
                            .pick_file()
                        {
                            self.image_path = Some(path);
                        }
                    }
                });

                match &self.image_path {
                    Some(p) => ui.label(p.display().to_string()),
                    None => ui.weak("Drop an image file here or click Browse..."),
                };
            });

            ui.add_space(16.0);

            // STEP 2: Select Target USB Drive
            ui.horizontal(|ui| {
                ui.strong("2. Target USB Drive");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!is_running, |ui| {
                        if ui.button("Refresh").clicked() {
                            self.refresh_drives();
                        }
                        if ui
                            .checkbox(&mut self.allow_virtual, "Show virtual test disks")
                            .changed()
                        {
                            self.refresh_drives();
                        }
                    });
                });
            });
            ui.add_space(4.0);

            ui.add_enabled_ui(!is_running, |ui| {
                if self.drives.is_empty() {
                    ui.weak("No removable USB drives found. Plug in a USB drive or enable virtual test disks.");
                } else {
                    let selected_label = self
                        .selected_drive_idx
                        .and_then(|i| self.drives.get(i))
                        .map(|d| {
                            format!(
                                "{} — {} ({:.1} MB) [{}]",
                                d.id,
                                d.model,
                                d.size_bytes as f64 / (1024.0 * 1024.0),
                                d.bus
                            )
                        })
                        .unwrap_or_else(|| "Select a drive".into());

                    egui::ComboBox::from_id_salt("drive_selector")
                        .width(520.0)
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (idx, d) in self.drives.iter().enumerate() {
                                let text = format!(
                                    "{} — {} ({:.1} MB) [{}]",
                                    d.id,
                                    d.model,
                                    d.size_bytes as f64 / (1024.0 * 1024.0),
                                    d.bus
                                );
                                ui.selectable_value(&mut self.selected_drive_idx, Some(idx), text);
                            }
                        });
                }
            });

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(14.0);

            // STEP 3: Flash / Cancel Controls
            let can_flash = !is_running
                && self.image_path.is_some()
                && self.selected_drive_idx.is_some()
                && !self.drives.is_empty();

            ui.horizontal(|ui| {
                ui.add_enabled_ui(can_flash, |ui| {
                    let btn = egui::Button::new(
                        egui::RichText::new("Flash & Verify Drive").size(15.0).strong(),
                    )
                    .min_size(egui::vec2(210.0, 38.0));

                    if ui.add(btn).clicked() {
                        self.show_confirm_modal = true;
                    }
                });

                if is_running {
                    let cancel_btn = egui::Button::new(
                        egui::RichText::new("Cancel").size(15.0),
                    )
                    .min_size(egui::vec2(100.0, 38.0));

                    if ui.add(cancel_btn).clicked() {
                        self.cancel_flag.store(true, Ordering::Relaxed);
                    }
                }
            });

            ui.add_space(14.0);

            // Progress & Status Display
            match current_status {
                JobStatus::Idle => {}
                JobStatus::Running {
                    phase,
                    progress,
                    speed_mbps,
                    bytes_done,
                    total_bytes,
                } => {
                    let msg = format!(
                        "{}: {:.1}% ({:.1} / {:.1} MB) @ {:.1} MB/s",
                        phase,
                        progress * 100.0,
                        bytes_done as f64 / (1024.0 * 1024.0),
                        total_bytes as f64 / (1024.0 * 1024.0),
                        speed_mbps
                    );
                    ui.add(egui::ProgressBar::new(progress).text(msg).animate(true));
                }
                JobStatus::Success(hash) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(40, 200, 90),
                        "✔ Flash & SHA-256 Verification Complete!",
                    );
                    ui.horizontal(|ui| {
                        ui.monospace(format!("SHA-256: {}", hash));
                        if ui.small_button("Copy Hash").clicked() {
                            ctx.copy_text(hash);
                        }
                    });
                }
                JobStatus::Error(err) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 80, 80),
                        format!("✖ Error: {}", err),
                    );
                }
            }
        });

        // Safety Confirmation Popup Modal
        if self.show_confirm_modal {
            egui::Window::new("Confirm Disk Erase")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if let Some(idx) = self.selected_drive_idx {
                        if let Some(d) = self.drives.get(idx) {
                            ui.label(format!(
                                "Are you sure you want to permanently overwrite {} ({})?",
                                d.id, d.model
                            ));
                            ui.colored_label(
                                egui::Color32::from_rgb(240, 160, 40),
                                "All existing partitions and files on this drive will be erased.",
                            );
                        }
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Yes, Erase & Flash").clicked() {
                            self.show_confirm_modal = false;
                            self.start_flash();
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_confirm_modal = false;
                        }
                    });
                });
        }
    }
}
