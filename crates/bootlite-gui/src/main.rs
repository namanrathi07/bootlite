use bootlite_core::{flash_and_verify, list_safe_usb_drives, UsbDrive};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const BG_DARK: Color32 = Color32::from_rgb(15, 23, 42);       // Slate 900
const CARD_BG: Color32 = Color32::from_rgb(30, 41, 59);       // Slate 800
const CARD_SELECTED: Color32 = Color32::from_rgb(30, 58, 138);// Blue 900
const BORDER_SUBTLE: Color32 = Color32::from_rgb(51, 65, 85); // Slate 700
const ACCENT_BLUE: Color32 = Color32::from_rgb(59, 130, 246); // Blue 500
const ACCENT_GREEN: Color32 = Color32::from_rgb(34, 197, 94); // Green 500
const ACCENT_AMBER: Color32 = Color32::from_rgb(245, 158, 11);// Amber 500
const ACCENT_RED: Color32 = Color32::from_rgb(239, 68, 68);   // Red 500
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 163, 184); // Slate 400

fn is_elevated() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::geteuid() == 0
    }
    #[cfg(windows)]
    {
        true
    }
}

fn relaunch_elevated() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe()?;
        let script = format!(
            "do shell script \"\\\"{}\\\" > /dev/null 2>&1 &\" with administrator privileges",
            exe.display()
        );
        let status = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status()?;
        if status.success() {
            std::process::exit(0);
        } else {
            anyhow::bail!("Administrator authentication was cancelled.");
        }
    }

    #[cfg(target_os = "linux")]
    {
        let exe = std::env::current_exe()?;
        let status = std::process::Command::new("pkexec").arg(exe).spawn();
        if status.is_ok() {
            std::process::exit(0);
        } else {
            anyhow::bail!("Failed to launch via pkexec.");
        }
    }

    #[cfg(target_os = "windows")]
    {
        Ok(())
    }
}

fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{:.1} MB", mb)
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([620.0, 560.0])
            .with_resizable(false)
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "BootLite — USB Image Flasher",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = BG_DARK;
            visuals.window_fill = CARD_BG;
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(BootLiteApp::new()))
        }),
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
    image_size: u64,
    drives: Vec<UsbDrive>,
    selected_drive_idx: Option<usize>,
    allow_virtual: bool,
    show_confirm_modal: bool,
    has_root: bool,
    cancel_flag: Arc<AtomicBool>,
    status: Arc<Mutex<JobStatus>>,
}

impl BootLiteApp {
    fn new() -> Self {
        let mut app = Self {
            image_path: None,
            image_size: 0,
            drives: Vec::new(),
            selected_drive_idx: None,
            allow_virtual: false,
            show_confirm_modal: false,
            has_root: is_elevated(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(JobStatus::Idle)),
        };
        app.refresh_drives();
        app
    }

    fn set_image(&mut self, path: PathBuf) {
        self.image_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.image_path = Some(path);
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
            phase: "Preparing disk...".into(),
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

        if !is_running {
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped.first() {
                if let Some(path) = &file.path {
                    self.set_image(path.clone());
                }
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG_DARK).inner_margin(20.0))
            .show(ctx, |ui| {
                // HEADER BAR
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("⚡ BootLite")
                                    .size(22.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.label(
                                RichText::new("v0.1.1")
                                    .size(11.0)
                                    .color(ACCENT_BLUE)
                                    .monospace(),
                            );
                        });
                        ui.label(
                            RichText::new("Zero-bloat raw USB flasher with SHA-256 read-back verification")
                                .size(12.5)
                                .color(TEXT_MUTED),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.has_root {
                            egui::Frame::default()
                                .fill(Color32::from_rgb(20, 83, 45))
                                .rounding(6.0)
                                .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("● Admin Ready")
                                            .size(12.0)
                                            .strong()
                                            .color(ACCENT_GREEN),
                                    );
                                });
                        } else if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🔒 Unlock Admin")
                                        .size(12.0)
                                        .color(ACCENT_AMBER),
                                )
                                .fill(Color32::from_rgb(69, 26, 3))
                                .rounding(6.0),
                            )
                            .clicked()
                        {
                            if let Err(e) = relaunch_elevated() {
                                *self.status.lock().unwrap() = JobStatus::Error(e.to_string());
                            }
                        }
                    });
                });

                ui.add_space(16.0);

                // STEP 1 CARD: SOURCE IMAGE
                egui::Frame::default()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .rounding(10.0)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("STEP 1 · SOURCE IMAGE")
                                    .size(11.5)
                                    .strong()
                                    .color(ACCENT_BLUE),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new("Supports .iso, .img, .dmg, .gz (Drag & Drop)")
                                        .size(11.0)
                                        .color(TEXT_MUTED),
                                );
                            });
                        });
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!is_running, |ui| {
                                let btn = egui::Button::new(
                                    RichText::new("📁 Choose Image...")
                                        .size(13.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(ACCENT_BLUE)
                                .rounding(6.0)
                                .min_size(Vec2::new(140.0, 34.0));

                                if ui.add(btn).clicked() {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("Disk Images", &["iso", "img", "dmg", "raw", "gz"])
                                        .pick_file()
                                    {
                                        self.set_image(path);
                                    }
                                }
                            });

                            ui.add_space(8.0);

                            match &self.image_path {
                                Some(p) => {
                                    let file_name = p
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| p.display().to_string());
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(file_name)
                                                .size(13.5)
                                                .strong()
                                                .color(Color32::WHITE),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{}  •  {}",
                                                format_bytes(self.image_size),
                                                p.display()
                                            ))
                                            .size(11.5)
                                            .color(TEXT_MUTED),
                                        );
                                    });
                                }
                                None => {
                                    ui.label(
                                        RichText::new("No image selected — drop a file anywhere here")
                                            .size(13.0)
                                            .color(TEXT_MUTED),
                                    );
                                }
                            }
                        });
                    });

                ui.add_space(12.0);

                // STEP 2 CARD: TARGET USB DRIVE
                egui::Frame::default()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .rounding(10.0)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("STEP 2 · TARGET USB DRIVE")
                                    .size(11.5)
                                    .strong()
                                    .color(ACCENT_BLUE),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(!is_running, |ui| {
                                    if ui.small_button("⟳ Refresh").clicked() {
                                        self.refresh_drives();
                                    }
                                    if ui
                                        .checkbox(&mut self.allow_virtual, "Virtual test disks")
                                        .changed()
                                    {
                                        self.refresh_drives();
                                    }
                                });
                            });
                        });
                        ui.add_space(8.0);

                        if self.drives.is_empty() {
                            egui::Frame::default()
                                .fill(BG_DARK)
                                .rounding(8.0)
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("🔌 No removable USB drives detected")
                                                .size(13.5)
                                                .color(TEXT_MUTED),
                                        );
                                        ui.label(
                                            RichText::new("Insert a USB flash drive or enable 'Virtual test disks' above.")
                                                .size(11.5)
                                                .color(TEXT_MUTED),
                                        );
                                    });
                                });
                        } else {
                            egui::ScrollArea::vertical()
                                .max_height(115.0)
                                .show(ui, |ui| {
                                    for (idx, d) in self.drives.iter().enumerate() {
                                        let is_selected = self.selected_drive_idx == Some(idx);
                                        let fill = if is_selected { CARD_SELECTED } else { BG_DARK };
                                        let border = if is_selected { ACCENT_BLUE } else { BORDER_SUBTLE };

                                        let resp = egui::Frame::default()
                                            .fill(fill)
                                            .stroke(Stroke::new(1.5, border))
                                            .rounding(8.0)
                                            .inner_margin(10.0)
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    let icon = if is_selected { "◉" } else { "○" };
                                                    ui.label(
                                                        RichText::new(icon)
                                                            .size(15.0)
                                                            .color(if is_selected { ACCENT_BLUE } else { TEXT_MUTED }),
                                                    );
                                                    ui.vertical(|ui| {
                                                        ui.label(
                                                            RichText::new(format!("{} ({})", d.model, d.id))
                                                                .size(13.5)
                                                                .strong()
                                                                .color(Color32::WHITE),
                                                        );
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{}  •  Bus: {}",
                                                                d.raw_device_path.display(),
                                                                d.bus
                                                            ))
                                                            .size(11.5)
                                                            .color(TEXT_MUTED),
                                                        );
                                                    });
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(egui::Align::Center),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new(format_bytes(d.size_bytes))
                                                                    .size(13.5)
                                                                    .strong()
                                                                    .color(ACCENT_GREEN),
                                                            );
                                                        },
                                                    );
                                                });
                                            })
                                            .response;

                                        if !is_running && resp.interact(egui::Sense::click()).clicked() {
                                            self.selected_drive_idx = Some(idx);
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                        }
                    });

                ui.add_space(14.0);

                // STEP 3: ACTION & TELEMETRY
                let can_flash = !is_running
                    && self.image_path.is_some()
                    && self.selected_drive_idx.is_some()
                    && !self.drives.is_empty();

                ui.horizontal(|ui| {
                    ui.add_enabled_ui(can_flash, |ui| {
                        let btn = egui::Button::new(
                            RichText::new("⚡ Flash & Verify Drive")
                                .size(15.5)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(if can_flash { ACCENT_BLUE } else { BORDER_SUBTLE })
                        .rounding(8.0)
                        .min_size(Vec2::new(230.0, 42.0));

                        if ui.add(btn).clicked() {
                            if !self.has_root {
                                if let Err(e) = relaunch_elevated() {
                                    *self.status.lock().unwrap() = JobStatus::Error(e.to_string());
                                }
                            } else {
                                self.show_confirm_modal = true;
                            }
                        }
                    });

                    if is_running {
                        let cancel_btn = egui::Button::new(
                            RichText::new("✖ Cancel").size(14.0).color(Color32::WHITE),
                        )
                        .fill(ACCENT_RED)
                        .rounding(8.0)
                        .min_size(Vec2::new(110.0, 42.0));

                        if ui.add(cancel_btn).clicked() {
                            self.cancel_flag.store(true, Ordering::Relaxed);
                        }
                    }
                });

                ui.add_space(12.0);

                // TELEMETRY / STATUS BAR
                match current_status {
                    JobStatus::Idle => {}
                    JobStatus::Running {
                        phase,
                        progress,
                        speed_mbps,
                        bytes_done,
                        total_bytes,
                    } => {
                        egui::Frame::default()
                            .fill(CARD_BG)
                            .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                            .rounding(8.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&phase).size(13.0).strong().color(Color32::WHITE),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(format!("{:.1} MB/s", speed_mbps))
                                                    .size(13.0)
                                                    .strong()
                                                    .color(ACCENT_BLUE)
                                                    .monospace(),
                                            );
                                        },
                                    );
                                });
                                ui.add_space(6.0);
                                let msg = format!(
                                    "{:.1}% ({} / {})",
                                    progress * 100.0,
                                    format_bytes(bytes_done),
                                    format_bytes(total_bytes)
                                );
                                ui.add(
                                    egui::ProgressBar::new(progress)
                                        .text(msg)
                                        .fill(ACCENT_BLUE)
                                        .animate(true),
                                );
                            });
                    }
                    JobStatus::Success(hash) => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(20, 83, 45))
                            .stroke(Stroke::new(1.0, ACCENT_GREEN))
                            .rounding(8.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("✔ Flash & SHA-256 Read-Back Verification Complete!")
                                        .size(13.5)
                                        .strong()
                                        .color(ACCENT_GREEN),
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.monospace(
                                        RichText::new(format!("SHA-256: {}", hash))
                                            .size(11.0)
                                            .color(Color32::WHITE),
                                    );
                                    if ui.small_button("📋 Copy").clicked() {
                                        ctx.copy_text(hash);
                                    }
                                });
                            });
                    }
                    JobStatus::Error(err) => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(69, 10, 10))
                            .stroke(Stroke::new(1.0, ACCENT_RED))
                            .rounding(8.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("✖ {}", err))
                                        .size(12.5)
                                        .color(Color32::from_rgb(252, 165, 165)),
                                );
                            });
                    }
                }
            });

        // CONFIRMATION MODAL
        if self.show_confirm_modal {
            egui::Window::new("⚠ Confirm Disk Erase")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if let Some(idx) = self.selected_drive_idx {
                        if let Some(d) = self.drives.get(idx) {
                            ui.label(
                                RichText::new(format!(
                                    "Permanently overwrite {} ({})?",
                                    d.model, d.id
                                ))
                                .size(14.0)
                                .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "All existing files and partitions on this drive will be destroyed.",
                                )
                                .color(ACCENT_AMBER),
                            );
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let yes_btn = egui::Button::new(
                            RichText::new("Yes, Erase & Flash")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_RED);

                        if ui.add(yes_btn).clicked() {
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
