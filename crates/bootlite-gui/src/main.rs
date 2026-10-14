#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bootlite_core::{flash_and_verify, list_safe_usb_drives, UsbDrive};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

// Studio Dark Palette (Linear / Raycast / Etcher inspired)
const BG_APP: Color32 = Color32::from_rgb(11, 15, 25);         // Deep Obsidian
const BG_CARD: Color32 = Color32::from_rgb(20, 27, 45);        // Elevated Slate
const BG_SELECTED: Color32 = Color32::from_rgb(23, 43, 97);    // Active Indigo Tint
const BORDER_DIM: Color32 = Color32::from_rgb(40, 52, 78);     // Subtle Hairline
const ACCENT_PRIMARY: Color32 = Color32::from_rgb(59, 130, 246);// Electric Blue
const ACCENT_GLOW: Color32 = Color32::from_rgb(96, 165, 250);  // Lighter Blue
const ACCENT_SUCCESS: Color32 = Color32::from_rgb(16, 185, 129);// Emerald
const ACCENT_WARN: Color32 = Color32::from_rgb(245, 158, 11);  // Amber
const ACCENT_DANGER: Color32 = Color32::from_rgb(239, 68, 68); // Crimson
const TEXT_MAIN: Color32 = Color32::from_rgb(248, 250, 252);   // Crisp White
const TEXT_SUB: Color32 = Color32::from_rgb(148, 163, 184);    // Muted Slate
const TEXT_DIM: Color32 = Color32::from_rgb(100, 116, 139);    // Dim Slate

fn is_elevated() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::geteuid() == 0
    }
    #[cfg(windows)]
    {
        std::process::Command::new("net")
            .arg("session")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
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
        let exe = std::env::current_exe()?;
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Start-Process -FilePath '{}' -Verb RunAs", exe.display()),
            ])
            .status()?;
        if status.success() {
            std::process::exit(0);
        } else {
            anyhow::bail!("Administrator UAC prompt was cancelled.");
        }
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

// Custom Vector Icon: Brand Lightning Badge
fn draw_brand_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(38.0, 38.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 10.0, Color32::from_rgb(29, 78, 216));
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, ACCENT_GLOW));

    let c = rect.center();
    let bolt = vec![
        Pos2::new(c.x + 2.0, c.y - 11.0),
        Pos2::new(c.x - 7.0, c.y + 1.0),
        Pos2::new(c.x - 1.0, c.y + 1.0),
        Pos2::new(c.x - 3.0, c.y + 11.0),
        Pos2::new(c.x + 7.0, c.y - 1.0),
        Pos2::new(c.x + 1.0, c.y - 1.0),
    ];
    p.add(Shape::convex_polygon(bolt, Color32::WHITE, Stroke::NONE));
}

// Custom Vector Icon: Optical ISO Disc
fn draw_disc_icon(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(42.0, 42.0), Sense::hover());
    let p = ui.painter();
    let tint = if active {
        Color32::from_rgb(30, 58, 138)
    } else {
        Color32::from_rgb(30, 41, 59)
    };
    let fg = if active { ACCENT_GLOW } else { TEXT_SUB };

    p.rect_filled(rect, 10.0, tint);
    let c = rect.center();
    p.circle_stroke(c, 12.0, Stroke::new(2.0_f32, fg));
    p.circle_stroke(c, 4.0, Stroke::new(1.5_f32, fg));
}

// Custom Vector Icon: USB Flash Drive
fn draw_usb_icon(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 40.0), Sense::hover());
    let p = ui.painter();
    let tint = if active {
        Color32::from_rgb(30, 58, 138)
    } else {
        Color32::from_rgb(30, 41, 59)
    };
    let fg = if active { ACCENT_GLOW } else { TEXT_SUB };

    p.rect_filled(rect, 10.0, tint);
    let c = rect.center();
    // USB connector head
    let head = Rect::from_center_size(Pos2::new(c.x, c.y - 8.0), Vec2::new(10.0, 6.0));
    p.rect_stroke(head, 1.0, Stroke::new(1.6_f32, fg));
    // USB body
    let body = Rect::from_center_size(Pos2::new(c.x, c.y + 3.0), Vec2::new(14.0, 16.0));
    p.rect_stroke(body, 3.0, Stroke::new(1.8_f32, fg));
    // Indicator LED dot
    p.circle_filled(
        Pos2::new(c.x, c.y + 6.0),
        2.0,
        if active { ACCENT_SUCCESS } else { fg },
    );
}

// Custom iOS / macOS Style Sliding Toggle Switch
fn draw_toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
    let bg_color = if *on {
        ACCENT_PRIMARY
    } else {
        Color32::from_rgb(51, 65, 85)
    };
    let radius = 0.5 * rect.height();
    ui.painter().rect_filled(rect, radius, bg_color);
    let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
    let center = Pos2::new(circle_x, rect.center().y);
    ui.painter()
        .circle_filled(center, radius - 2.5, Color32::WHITE);
    response
}

// Pill Badge Helper
fn draw_pill(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    egui::Frame::default()
        .fill(bg)
        .rounding(5.0)
        .inner_margin(egui::Margin::symmetric(7.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).strong().color(fg));
        });
}


fn build_window_icon() -> egui::IconData {
    let (w, h) = (64, 64);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 - 31.5).abs();
            let dy = (y as f32 - 31.5).abs();
            let dist = ((dx / 28.0).powi(4) + (dy / 28.0).powi(4)).powf(0.25);
            if dist > 1.0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let nx = (x as f32 - 32.0) / 18.0;
                let ny = (y as f32 - 32.0) / 18.0;
                let in_bolt = (ny >= -0.85 && ny <= 0.08 && nx >= -0.55 + (ny + 0.08) * 0.65 && nx <= 0.15 - (ny + 0.08) * 0.35)
                    || (ny >= -0.08 && ny <= 0.85 && nx <= 0.55 - (ny - 0.08) * 0.65 && nx >= -0.15 + (ny - 0.08) * 0.35);
                if in_bolt {
                    rgba.extend_from_slice(&[255, 255, 255, 255]);
                } else if dist > 0.92 {
                    rgba.extend_from_slice(&[59, 130, 246, 255]);
                } else {
                    let t = 1.0 - (y as f32 / h as f32);
                    rgba.extend_from_slice(&[
                        (15.0 + t * 20.0) as u8,
                        (28.0 + t * 55.0) as u8,
                        (75.0 + t * 135.0) as u8,
                        255,
                    ]);
                }
            }
        }
    }
    egui::IconData { rgba, width: w, height: h }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 560.0])
            .with_resizable(false)
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(build_window_icon())),
        ..Default::default()
    };

    eframe::run_native(
        "BootLite",
        options,
        Box::new(|cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = Vec2::new(10.0, 8.0);
            style.spacing.button_padding = Vec2::new(12.0, 7.0);
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = BG_APP;
            visuals.window_fill = BG_CARD;
            visuals.window_rounding = 12.0.into();
            visuals.widgets.noninteractive.bg_fill = BG_CARD;
            visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 41, 59);
            visuals.widgets.inactive.rounding = 8.0.into();
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(51, 65, 85);
            visuals.widgets.hovered.rounding = 8.0.into();
            visuals.widgets.active.bg_fill = ACCENT_PRIMARY;
            visuals.widgets.active.rounding = 8.0.into();
            style.visuals = visuals;
            cc.egui_ctx.set_style(style);
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

    fn browse_image(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Disk Images", &["iso", "img", "dmg", "raw", "gz"])
            .pick_file()
        {
            self.set_image(path);
        }
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
            phase: "Unmounting & preparing target drive...".into(),
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
            .frame(egui::Frame::default().fill(BG_APP).inner_margin(24.0))
            .show(ctx, |ui| {
                // ─────────────────────────────────────────────────────────────
                // TOP NAVIGATION & BRAND HEADER
                // ─────────────────────────────────────────────────────────────
                ui.horizontal(|ui| {
                    draw_brand_logo(ui);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("BootLite")
                                    .size(20.0)
                                    .strong()
                                    .color(TEXT_MAIN),
                            );
                            draw_pill(
                                ui,
                                "v0.1.2",
                                Color32::from_rgb(30, 58, 138),
                                ACCENT_GLOW,
                            );
                        });
                        ui.label(
                            RichText::new("Raw Hardware USB Flasher & SHA-256 Verifier")
                                .size(12.0)
                                .color(TEXT_SUB),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.has_root {
                            draw_pill(
                                ui,
                                "ADMIN UNLOCKED",
                                Color32::from_rgb(6, 78, 59),
                                ACCENT_SUCCESS,
                            );
                        } else {
                            let btn = egui::Button::new(
                                RichText::new("Unlock Admin Access")
                                    .size(12.0)
                                    .strong()
                                    .color(ACCENT_WARN),
                            )
                            .fill(Color32::from_rgb(69, 26, 3))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(146, 64, 14)))
                            .rounding(7.0);

                            let resp = ui.add(btn);
                            if resp.hovered() {
                                ctx.set_cursor_icon(CursorIcon::PointingHand);
                            }
                            if resp.clicked() {
                                if let Err(e) = relaunch_elevated() {
                                    *self.status.lock().unwrap() = JobStatus::Error(e.to_string());
                                }
                            }
                        }
                    });
                });

                ui.add_space(16.0);

                // ─────────────────────────────────────────────────────────────
                // 3-STAGE PIPELINE STEPPER BAR
                // ─────────────────────────────────────────────────────────────
                let step1_done = self.image_path.is_some();
                let step2_done = self.selected_drive_idx.is_some();
                let step3_done = matches!(current_status, JobStatus::Success(_));

                egui::Frame::default()
                    .fill(Color32::from_rgb(15, 21, 36))
                    .stroke(Stroke::new(1.0_f32, BORDER_DIM))
                    .rounding(8.0)
                    .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                    .show(ui, |ui| {
                        ui.columns(3, |cols| {
                            cols[0].horizontal(|ui| {
                                let c = if step1_done { ACCENT_SUCCESS } else { ACCENT_PRIMARY };
                                draw_pill(ui, "STEP 1", c.gamma_multiply(0.2), c);
                                ui.label(
                                    RichText::new(if step1_done { "Image Ready" } else { "Select Image" })
                                        .size(12.0)
                                        .strong()
                                        .color(if step1_done { TEXT_MAIN } else { TEXT_SUB }),
                                );
                            });
                            cols[1].horizontal(|ui| {
                                let c = if step2_done {
                                    ACCENT_SUCCESS
                                } else if step1_done {
                                    ACCENT_PRIMARY
                                } else {
                                    TEXT_DIM
                                };
                                draw_pill(ui, "STEP 2", c.gamma_multiply(0.2), c);
                                ui.label(
                                    RichText::new(if step2_done { "Drive Selected" } else { "Target Drive" })
                                        .size(12.0)
                                        .strong()
                                        .color(if step2_done { TEXT_MAIN } else { TEXT_SUB }),
                                );
                            });
                            cols[2].horizontal(|ui| {
                                let c = if step3_done {
                                    ACCENT_SUCCESS
                                } else if is_running {
                                    ACCENT_PRIMARY
                                } else {
                                    TEXT_DIM
                                };
                                draw_pill(ui, "STEP 3", c.gamma_multiply(0.2), c);
                                ui.label(
                                    RichText::new(if step3_done {
                                        "Verified"
                                    } else if is_running {
                                        "Flashing..."
                                    } else {
                                        "Flash & Verify"
                                    })
                                    .size(12.0)
                                    .strong()
                                    .color(if step3_done || is_running { TEXT_MAIN } else { TEXT_SUB }),
                                );
                            });
                        });
                    });

                ui.add_space(14.0);

                // ─────────────────────────────────────────────────────────────
                // STEP 1: INTERACTIVE IMAGE DROPZONE CARD
                // ─────────────────────────────────────────────────────────────
                let has_img = self.image_path.is_some();
                let card1_border = if has_img {
                    Color32::from_rgb(37, 99, 235)
                } else {
                    BORDER_DIM
                };

                egui::Frame::default()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.2_f32, card1_border))
                    .rounding(12.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            draw_disc_icon(ui, has_img);
                            ui.add_space(6.0);

                            ui.vertical(|ui| {
                                match &self.image_path {
                                    Some(p) => {
                                        let name = p
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| p.display().to_string());
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(name)
                                                    .size(14.5)
                                                    .strong()
                                                    .color(TEXT_MAIN),
                                            );
                                            draw_pill(
                                                ui,
                                                &format_bytes(self.image_size),
                                                Color32::from_rgb(30, 58, 138),
                                                ACCENT_GLOW,
                                            );
                                        });
                                        ui.label(
                                            RichText::new(p.display().to_string())
                                                .size(11.5)
                                                .color(TEXT_SUB),
                                        );
                                    }
                                    None => {
                                        ui.label(
                                            RichText::new("Select Bootable OS Image")
                                                .size(14.5)
                                                .strong()
                                                .color(TEXT_MAIN),
                                        );
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("Drag & drop file anywhere or browse")
                                                    .size(12.0)
                                                    .color(TEXT_SUB),
                                            );
                                            draw_pill(ui, "ISO", Color32::from_rgb(30, 41, 59), TEXT_SUB);
                                            draw_pill(ui, "IMG", Color32::from_rgb(30, 41, 59), TEXT_SUB);
                                            draw_pill(ui, "DMG", Color32::from_rgb(30, 41, 59), TEXT_SUB);
                                            draw_pill(ui, "GZ", Color32::from_rgb(30, 41, 59), TEXT_SUB);
                                        });
                                    }
                                }
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(!is_running, |ui| {
                                    let label = if has_img { "Change File" } else { "Browse File..." };
                                    let btn_fill = if has_img {
                                        Color32::from_rgb(30, 41, 59)
                                    } else {
                                        ACCENT_PRIMARY
                                    };
                                    let btn = egui::Button::new(
                                        RichText::new(label)
                                            .size(13.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    )
                                    .fill(btn_fill)
                                    .rounding(8.0)
                                    .min_size(Vec2::new(118.0, 34.0));

                                    let resp = ui.add(btn);
                                    if resp.hovered() {
                                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    if resp.clicked() {
                                        self.browse_image();
                                    }
                                });
                            });
                        });
                    });

                ui.add_space(12.0);

                // ─────────────────────────────────────────────────────────────
                // STEP 2: TARGET USB DRIVE SELECTOR CARD
                // ─────────────────────────────────────────────────────────────
                egui::Frame::default()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.2_f32, BORDER_DIM))
                    .rounding(12.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("TARGET DESTINATION DRIVE")
                                    .size(11.0)
                                    .strong()
                                    .color(TEXT_SUB),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(!is_running, |ui| {
                                    let refresh_btn = egui::Button::new(
                                        RichText::new("Rescan Drives").size(11.5).color(TEXT_MAIN),
                                    )
                                    .fill(Color32::from_rgb(30, 41, 59))
                                    .rounding(6.0);

                                    let r_resp = ui.add(refresh_btn);
                                    if r_resp.hovered() {
                                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    if r_resp.clicked() {
                                        self.refresh_drives();
                                    }

                                    ui.add_space(8.0);
                                    if draw_toggle_switch(ui, &mut self.allow_virtual).changed() {
                                        self.refresh_drives();
                                    }
                                    ui.label(
                                        RichText::new("Virtual Disks")
                                            .size(11.5)
                                            .color(TEXT_SUB),
                                    );
                                });
                            });
                        });

                        ui.add_space(10.0);

                        if self.drives.is_empty() {
                            egui::Frame::default()
                                .fill(BG_APP)
                                .stroke(Stroke::new(1.0_f32, BORDER_DIM))
                                .rounding(10.0)
                                .inner_margin(18.0)
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("No External USB Drive Connected")
                                                .size(13.5)
                                                .strong()
                                                .color(TEXT_MAIN),
                                        );
                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new(
                                                "Plug in a removable USB flash drive — internal system disks are locked for safety.",
                                            )
                                            .size(11.5)
                                            .color(TEXT_SUB),
                                        );
                                    });
                                });
                        } else {
                            egui::ScrollArea::vertical()
                                .max_height(125.0)
                                .show(ui, |ui| {
                                    for (idx, d) in self.drives.iter().enumerate() {
                                        let is_selected = self.selected_drive_idx == Some(idx);
                                        let fill = if is_selected { BG_SELECTED } else { BG_APP };
                                        let border = if is_selected {
                                            ACCENT_PRIMARY
                                        } else {
                                            BORDER_DIM
                                        };

                                        let resp = egui::Frame::default()
                                            .fill(fill)
                                            .stroke(Stroke::new(1.5_f32, border))
                                            .rounding(10.0)
                                            .inner_margin(12.0)
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    draw_usb_icon(ui, is_selected);
                                                    ui.add_space(6.0);
                                                    ui.vertical(|ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(&d.model)
                                                                    .size(14.0)
                                                                    .strong()
                                                                    .color(TEXT_MAIN),
                                                            );
                                                            draw_pill(
                                                                ui,
                                                                &d.bus,
                                                                Color32::from_rgb(30, 41, 59),
                                                                TEXT_SUB,
                                                            );
                                                        });
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "Device: {} ({})",
                                                                d.id,
                                                                d.raw_device_path.display()
                                                            ))
                                                            .size(11.5)
                                                            .color(TEXT_SUB),
                                                        );
                                                    });

                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(egui::Align::Center),
                                                        |ui| {
                                                            draw_pill(
                                                                ui,
                                                                &format_bytes(d.size_bytes),
                                                                Color32::from_rgb(6, 78, 59),
                                                                ACCENT_SUCCESS,
                                                            );
                                                        },
                                                    );
                                                });
                                            })
                                            .response;

                                        let click_resp = resp.interact(Sense::click());
                                        if click_resp.hovered() && !is_running {
                                            ctx.set_cursor_icon(CursorIcon::PointingHand);
                                        }
                                        if click_resp.clicked() && !is_running {
                                            self.selected_drive_idx = Some(idx);
                                        }
                                        ui.add_space(6.0);
                                    }
                                });
                        }
                    });

                ui.add_space(14.0);

                // ─────────────────────────────────────────────────────────────
                // STEP 3: ACTION BAR & LIVE TELEMETRY
                // ─────────────────────────────────────────────────────────────
                let can_flash = !is_running
                    && self.image_path.is_some()
                    && self.selected_drive_idx.is_some()
                    && !self.drives.is_empty();

                match current_status {
                    JobStatus::Idle => {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    "Unbuffered raw I/O + full sector SHA-256 read-back verification",
                                )
                                .size(11.5)
                                .color(TEXT_DIM),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(can_flash, |ui| {
                                    let fill = if can_flash {
                                        ACCENT_PRIMARY
                                    } else {
                                        Color32::from_rgb(30, 41, 59)
                                    };
                                    let btn = egui::Button::new(
                                        RichText::new("Flash & Verify Drive")
                                            .size(14.5)
                                            .strong()
                                            .color(if can_flash { Color32::WHITE } else { TEXT_DIM }),
                                    )
                                    .fill(fill)
                                    .rounding(9.0)
                                    .min_size(Vec2::new(210.0, 42.0));

                                    let resp = ui.add(btn);
                                    if resp.hovered() && can_flash {
                                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    if resp.clicked() {
                                        if !self.has_root {
                                            if let Err(e) = relaunch_elevated() {
                                                *self.status.lock().unwrap() =
                                                    JobStatus::Error(e.to_string());
                                            }
                                        } else {
                                            self.show_confirm_modal = true;
                                        }
                                    }
                                });
                            });
                        });
                    }
                    JobStatus::Running {
                        phase,
                        progress,
                        speed_mbps,
                        bytes_done,
                        total_bytes,
                    } => {
                        egui::Frame::default()
                            .fill(BG_CARD)
                            .stroke(Stroke::new(1.2_f32, ACCENT_PRIMARY))
                            .rounding(10.0)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&phase).size(13.0).strong().color(TEXT_MAIN),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("Abort")
                                                            .size(11.5)
                                                            .strong()
                                                            .color(Color32::WHITE),
                                                    )
                                                    .fill(ACCENT_DANGER)
                                                    .rounding(6.0),
                                                )
                                                .clicked()
                                            {
                                                self.cancel_flag.store(true, Ordering::Relaxed);
                                            }
                                            ui.add_space(8.0);
                                            draw_pill(
                                                ui,
                                                &format!("{:.1} MB/s", speed_mbps),
                                                Color32::from_rgb(30, 58, 138),
                                                ACCENT_GLOW,
                                            );
                                        },
                                    );
                                });
                                ui.add_space(8.0);
                                let msg = format!(
                                    "{:.1}%  •  {} / {}",
                                    progress * 100.0,
                                    format_bytes(bytes_done),
                                    format_bytes(total_bytes)
                                );
                                ui.add(
                                    egui::ProgressBar::new(progress)
                                        .text(msg)
                                        .fill(ACCENT_PRIMARY)
                                        .animate(true),
                                );
                            });
                    }
                    JobStatus::Success(hash) => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(6, 46, 34))
                            .stroke(Stroke::new(1.2_f32, ACCENT_SUCCESS))
                            .rounding(10.0)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Flash & Read-Back SHA-256 Verification Passed")
                                            .size(13.5)
                                            .strong()
                                            .color(ACCENT_SUCCESS),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("Flash Another").clicked() {
                                                *self.status.lock().unwrap() = JobStatus::Idle;
                                            }
                                            if ui.small_button("Copy SHA-256").clicked() {
                                                ctx.copy_text(hash.clone());
                                            }
                                        },
                                    );
                                });
                                ui.add_space(4.0);
                                ui.monospace(
                                    RichText::new(format!("SHA-256: {}", hash))
                                        .size(11.0)
                                        .color(TEXT_MAIN),
                                );
                            });
                    }
                    JobStatus::Error(err) => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(55, 12, 12))
                            .stroke(Stroke::new(1.2_f32, ACCENT_DANGER))
                            .rounding(10.0)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Error: {}", err))
                                            .size(12.5)
                                            .color(Color32::from_rgb(252, 165, 165)),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("Dismiss").clicked() {
                                                *self.status.lock().unwrap() = JobStatus::Idle;
                                            }
                                        },
                                    );
                                });
                            });
                    }
                }
            });

        // SAFETY CONFIRMATION MODAL
        if self.show_confirm_modal {
            egui::Window::new("Confirm Target Drive Erase")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    if let Some(idx) = self.selected_drive_idx {
                        if let Some(d) = self.drives.get(idx) {
                            ui.label(
                                RichText::new(format!(
                                    "You are about to overwrite {} ({})",
                                    d.model, d.id
                                ))
                                .size(14.0)
                                .strong()
                                .color(TEXT_MAIN),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "All existing partitions and data on this device will be permanently erased.",
                                )
                                .size(12.0)
                                .color(ACCENT_WARN),
                            );
                        }
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let yes_btn = egui::Button::new(
                            RichText::new("Erase & Flash Now")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_DANGER)
                        .rounding(7.0);

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
