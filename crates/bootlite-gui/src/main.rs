#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bootlite_core::{flash_and_verify_with_scheme, PartitionScheme,
    compute_image_sha256, eject_drive, inspect_image, is_image_on_drive,
    list_safe_usb_drives, normalize_sha256_input, reset_and_format_drive_with_scheme, windows_iso::WIN_ISO_OVERHEAD_BYTES, ImageMetadata, UsbDrive,
};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BG_APP: Color32 = Color32::from_rgb(11, 15, 25);
const BG_CARD: Color32 = Color32::from_rgb(20, 27, 45);
const BG_TERMINAL: Color32 = Color32::from_rgb(7, 10, 18);
const BG_SELECTED: Color32 = Color32::from_rgb(23, 43, 97);
const BORDER_DIM: Color32 = Color32::from_rgb(40, 52, 78);
const ACCENT_PRIMARY: Color32 = Color32::from_rgb(59, 130, 246);
const ACCENT_GLOW: Color32 = Color32::from_rgb(96, 165, 250);
const ACCENT_SUCCESS: Color32 = Color32::from_rgb(16, 185, 129);
const ACCENT_WARN: Color32 = Color32::from_rgb(245, 158, 11);
const ACCENT_DANGER: Color32 = Color32::from_rgb(239, 68, 68);
const TEXT_MAIN: Color32 = Color32::from_rgb(248, 250, 252);
const TEXT_SUB: Color32 = Color32::from_rgb(148, 163, 184);
const TEXT_DIM: Color32 = Color32::from_rgb(100, 116, 139);

struct SleepInhibitor {
    #[cfg(unix)]
    child: Option<std::process::Child>,
}

impl SleepInhibitor {
    fn acquire() -> Self {
        #[cfg(target_os = "macos")]
        {
            let pid = std::process::id().to_string();
            let child = std::process::Command::new("caffeinate")
                .args(["-di", "-w", &pid])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            Self { child }
        }
        #[cfg(target_os = "linux")]
        {
            let child = std::process::Command::new("systemd-inhibit")
                .args([
                    "--what=idle:sleep",
                    "--who=BootLite",
                    "--why=Flashing USB drive",
                    "sleep",
                    "3600",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            Self { child }
        }
        #[cfg(target_os = "windows")]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn SetThreadExecutionState(es_flags: u32) -> u32;
            }
            unsafe {
                let _ = SetThreadExecutionState(0x80000041);
            }
            Self {}
        }
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(target_os = "windows")]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn SetThreadExecutionState(es_flags: u32) -> u32;
            }
            unsafe {
                let _ = SetThreadExecutionState(0x80000000);
            }
        }
    }
}

fn is_elevated() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::geteuid() == 0
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn IsUserAnAdmin() -> i32;
        }
        unsafe { IsUserAnAdmin() != 0 }
    }
}

fn relaunch_elevated(image_arg: Option<&PathBuf>) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe()?;
        let script = match image_arg {
            Some(img) => format!(
                "do shell script \"\\\"{}\\\" \\\"{}\\\" > /dev/null 2>&1 &\" with administrator privileges",
                exe.display(),
                img.display()
            ),
            None => format!(
                "do shell script \"\\\"{}\\\" > /dev/null 2>&1 &\" with administrator privileges",
                exe.display()
            ),
        };
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
        let mut cmd = std::process::Command::new("pkexec");
        cmd.arg("env");
        for key in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            if let Ok(val) = std::env::var(key) {
                cmd.arg(format!("{}={}", key, val));
            }
        }
        cmd.arg(exe);
        if let Some(img) = image_arg {
            cmd.arg(img);
        }
        if cmd.spawn().is_ok() {
            std::process::exit(0);
        } else {
            anyhow::bail!("Failed to launch elevated session via pkexec.");
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        let exe = std::env::current_exe()?;
        let verb: Vec<u16> = std::ffi::OsStr::new("runas")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let file: Vec<u16> = exe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let params: Vec<u16> = image_arg
            .map(|p| format!("\"{}\"", p.display()))
            .unwrap_or_default()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let params_ptr = if image_arg.is_some() {
            params.as_ptr()
        } else {
            std::ptr::null()
        };
        let res = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params_ptr,
                std::ptr::null(),
                1,
            )
        };
        if res > 32 {
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

fn format_duration(secs: u64) -> String {
    let mins = secs / 60;
    let rem_secs = secs % 60;
    if mins >= 60 {
        let hours = mins / 60;
        format!("{:02}:{:02}:{:02}", hours, mins % 60, rem_secs)
    } else {
        format!("{:02}:{:02}", mins, rem_secs)
    }
}

fn now_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn push_log(logs: &Arc<Mutex<Vec<String>>>, msg: impl Into<String>) {
    let entry = format!("[{}] {}", now_timestamp(), msg.into());
    if let Ok(mut guard) = logs.lock() {
        guard.push(entry);
    }
}

fn os_io_mode_description() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Raw Character Device (/dev/rdiskN) + F_NOCACHE / F_FULLFSYNC"
    } else if cfg!(target_os = "linux") {
        "Linux /sys/block Direct + flock(LOCK_EX) udev Inhibitor + BLKFLSBUF"
    } else {
        "Windows Win32 PhysicalDrive + FindFirstVolumeW + IOCTL_DISK_CREATE_DISK"
    }
}

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
    let head = Rect::from_center_size(Pos2::new(c.x, c.y - 8.0), Vec2::new(10.0, 6.0));
    p.rect_stroke(head, 1.0, Stroke::new(1.6_f32, fg));
    let body = Rect::from_center_size(Pos2::new(c.x, c.y + 3.0), Vec2::new(14.0, 16.0));
    p.rect_stroke(body, 3.0, Stroke::new(1.8_f32, fg));
    p.circle_filled(
        Pos2::new(c.x, c.y + 6.0),
        2.0,
        if active { ACCENT_SUCCESS } else { fg },
    );
}

fn draw_toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = Vec2::new(34.0, 18.0);
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
                let in_bolt = (ny >= -0.85
                    && ny <= 0.08
                    && nx >= -0.55 + (ny + 0.08) * 0.65
                    && nx <= 0.15 - (ny + 0.08) * 0.35)
                    || (ny >= -0.08
                        && ny <= 0.85
                        && nx <= 0.55 - (ny - 0.08) * 0.65
                        && nx >= -0.15 + (ny - 0.08) * 0.35);
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
    egui::IconData {
        rgba,
        width: w,
        height: h,
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 745.0])
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
            style.spacing.item_spacing = Vec2::new(10.0, 7.0);
            style.spacing.button_padding = Vec2::new(12.0, 6.5);
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
        elapsed_secs: u64,
        eta_secs: Option<u64>,
    },
    Success {
        hash: String,
        elapsed_secs: u64,
        ejected: bool,
    },
    ResetSuccess {
        drive_name: String,
        elapsed_secs: u64,
    },
    Error(String),
}

struct BootLiteApp {
    image_path: Option<PathBuf>,
    image_size: u64,
    image_meta: Option<ImageMetadata>,
    expected_sha256: String,
    drives: Vec<UsbDrive>,
    selected_drive_idx: Option<usize>,
    allow_virtual: bool,
    auto_eject: bool,
    partition_scheme: PartitionScheme,
    show_confirm_modal: bool,
    show_reset_modal: bool,
    show_exit_modal: bool,
    allow_close: bool,
    show_log_drawer: bool,
    has_root: bool,
    last_hotplug_poll: Instant,
    cancel_flag: Arc<AtomicBool>,
    status: Arc<Mutex<JobStatus>>,
    logs: Arc<Mutex<Vec<String>>>,
}

impl BootLiteApp {
    fn new() -> Self {
        let has_root = is_elevated();
        let logs = Arc::new(Mutex::new(Vec::new()));

        push_log(&logs, "BootLite v0.3.3 initialized — Hardware Safety Locks ACTIVE");
        push_log(&logs, format!("I/O Backend: {}", os_io_mode_description()));
        if has_root {
            push_log(&logs, "Privilege Check: Running with Administrator / Root raw disk access");
        } else {
            push_log(
                &logs,
                "Privilege Check: Running as standard user (Click 'Unlock Admin Access' to flash)",
            );
        }

        let mut app = Self {
            image_path: None,
            image_size: 0,
            image_meta: None,
            expected_sha256: String::new(),
            drives: Vec::new(),
            selected_drive_idx: None,
            allow_virtual: false,
            auto_eject: false,
            partition_scheme: PartitionScheme::Auto,
            show_confirm_modal: false,
            show_reset_modal: false,
            show_exit_modal: false,
            allow_close: false,
            show_log_drawer: true,
            has_root,
            last_hotplug_poll: Instant::now(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(JobStatus::Idle)),
            logs,
        };
        app.refresh_drives(true);
        if let Some(arg_path) = std::env::args_os().nth(1) {
            let p = PathBuf::from(arg_path);
            if p.is_file() {
                app.set_image(p);
            }
        }
        app
    }

    fn set_image(&mut self, path: PathBuf) {
        if path.is_dir() {
            push_log(
                &self.logs,
                format!(
                    "[WARN] Rejected directory '{}': Please select a disk image file (.iso, .img, .dmg, .gz, .bz2, .xz, .zst)",
                    path.display()
                ),
            );
            return;
        }

        self.image_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if self.image_size == 0 {
            push_log(
                &self.logs,
                format!("[ERROR] Selected file '{}' is empty (0 bytes)", path.display()),
            );
            return;
        }

        match inspect_image(&path) {
            Ok(meta) => {
                push_log(
                    &self.logs,
                    format!(
                        "Loaded '{}' ({}) — {} | Format: {}",
                        path.display(),
                        format_bytes(meta.file_size),
                        meta.scheme,
                        meta.compression
                    ),
                );
                if let Some(warn) = &meta.warning {
                    push_log(&self.logs, format!("[WARN] {}", warn));
                }
                self.image_meta = Some(meta);
            }
            Err(e) => {
                push_log(
                    &self.logs,
                    format!("[WARN] Could not inspect image header: {}", e),
                );
                self.image_meta = None;
            }
        }

        self.image_path = Some(path);
    }

    fn browse_image(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Bootable Disk Images",
                &[
                    "iso", "img", "dmg", "raw", "bin", "gz", "gzip", "bz2", "bzip2", "xz",
                    "lzma", "zst", "zstd",
                ],
            )
            .add_filter("All Files (*)", &["*"])
            .pick_file()
        {
            self.set_image(path);
        }
    }

    fn refresh_drives(&mut self, force_log: bool) {
        let prev_ids: Vec<String> = self.drives.iter().map(|d| d.id.clone()).collect();
        let selected_id = self
            .selected_drive_idx
            .and_then(|idx| self.drives.get(idx))
            .map(|d| d.id.clone());

        self.drives = list_safe_usb_drives(self.allow_virtual).unwrap_or_default();
        let new_ids: Vec<String> = self.drives.iter().map(|d| d.id.clone()).collect();
        let changed = prev_ids != new_ids;

        if self.drives.is_empty() {
            self.selected_drive_idx = None;
            if force_log || changed {
                push_log(
                    &self.logs,
                    format!(
                        "Bus scan: 0 external drives connected (virtual_disks={})",
                        self.allow_virtual
                    ),
                );
            }
        } else {
            if let Some(prev_sel) = selected_id {
                self.selected_drive_idx = self
                    .drives
                    .iter()
                    .position(|d| d.id == prev_sel)
                    .or(Some(0));
            } else {
                self.selected_drive_idx = Some(0);
            }

            if force_log || changed {
                push_log(
                    &self.logs,
                    format!(
                        "Bus scan: {} safe target drive(s) ready",
                        self.drives.len()
                    ),
                );
            }
        }
        self.last_hotplug_poll = Instant::now();
    }

    fn start_file_hash_check(&mut self) {
        let Some(image_path) = self.image_path.clone() else { return };
        let expected_opt = normalize_sha256_input(&self.expected_sha256);

        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel_clone = Arc::clone(&self.cancel_flag);
        let status_clone = Arc::clone(&self.status);
        let logs_clone = Arc::clone(&self.logs);

        push_log(
            &logs_clone,
            format!("Computing SHA-256 checksum of '{}'...", image_path.display()),
        );

        *status_clone.lock().unwrap() = JobStatus::Running {
            phase: "Computing file SHA-256...".into(),
            progress: 0.0,
            speed_mbps: 0.0,
            bytes_done: 0,
            total_bytes: 1,
            elapsed_secs: 0,
            eta_secs: None,
        };

        thread::spawn(move || {
            let start = Instant::now();
            let res = compute_image_sha256(&image_path, Some(&cancel_clone), |prog| {
                let fraction = (prog.bytes_processed as f32 / prog.total_bytes as f32).clamp(0.0, 1.0);
                let elapsed_secs = start.elapsed().as_secs();
                let eta_secs = if prog.speed_mbps > 0.1 && prog.total_bytes > prog.bytes_processed {
                    let rem_mb = (prog.total_bytes - prog.bytes_processed) as f64 / (1024.0 * 1024.0);
                    Some((rem_mb / prog.speed_mbps).round() as u64)
                } else {
                    None
                };
                *status_clone.lock().unwrap() = JobStatus::Running {
                    phase: prog.phase.to_string(),
                    progress: fraction,
                    speed_mbps: prog.speed_mbps,
                    bytes_done: prog.bytes_processed,
                    total_bytes: prog.total_bytes,
                    elapsed_secs,
                    eta_secs,
                };
            });

            match res {
                Ok(file_hash) => {
                    push_log(&logs_clone, format!("File SHA-256: {}", file_hash));
                    if let Some(exp) = expected_opt {
                        if exp == file_hash {
                            push_log(
                                &logs_clone,
                                "[OK] File SHA-256 matches your expected checksum!",
                            );
                            *status_clone.lock().unwrap() = JobStatus::Idle;
                        } else {
                            let msg = format!(
                                "File SHA-256 mismatch! Expected: {}, Actual: {}",
                                exp, file_hash
                            );
                            push_log(&logs_clone, format!("[ERROR] {}", msg));
                            *status_clone.lock().unwrap() = JobStatus::Error(msg);
                        }
                    } else {
                        push_log(
                            &logs_clone,
                            "[OK] File SHA-256 computed (paste a checksum above to compare).",
                        );
                        *status_clone.lock().unwrap() = JobStatus::Idle;
                    }
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    push_log(&logs_clone, format!("[ERROR] {}", msg));
                    *status_clone.lock().unwrap() = JobStatus::Error(msg);
                }
            }
        });
    }

    fn start_reset_drive(&mut self) {
        let Some(idx) = self.selected_drive_idx else { return };
        let Some(drive) = self.drives.get(idx).cloned() else { return };
        let scheme = self.partition_scheme;

        let status_clone = Arc::clone(&self.status);
        let logs_clone = Arc::clone(&self.logs);

        push_log(&logs_clone, "────────────────────────────────────────────────────────");
        push_log(
            &logs_clone,
            format!(
                "RESETTING & FORMATTING USB -> {} ({}) [{}]",
                drive.model,
                drive.id,
                format_bytes(drive.size_bytes)
            ),
        );

        *status_clone.lock().unwrap() = JobStatus::Running {
            phase: format!("Wiping & formatting {} as ExFAT...", drive.id),
            progress: 0.5,
            speed_mbps: 0.0,
            bytes_done: drive.size_bytes / 2,
            total_bytes: drive.size_bytes,
            elapsed_secs: 0,
            eta_secs: None,
        };

        thread::spawn(move || {
            let start = Instant::now();
            match reset_and_format_drive_with_scheme(&drive, "BOOTLITE", scheme) {
                Ok(()) => {
                    let elapsed = start.elapsed().as_secs();
                    push_log(
                        &logs_clone,
                        format!(
                            "[OK] Restored {} ({}) to full-capacity ExFAT volume 'BOOTLITE'",
                            drive.model, drive.id
                        ),
                    );
                    *status_clone.lock().unwrap() = JobStatus::ResetSuccess {
                        drive_name: format!("{} ({})", drive.model, drive.id),
                        elapsed_secs: elapsed,
                    };
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    push_log(&logs_clone, format!("[ERROR] {}", msg));
                    *status_clone.lock().unwrap() = JobStatus::Error(msg);
                }
            }
        });
    }

    fn start_flash(&mut self) {
        let Some(image_path) = self.image_path.clone() else { return };
        let Some(idx) = self.selected_drive_idx else { return };
        let Some(drive) = self.drives.get(idx).cloned() else { return };
        let expected_opt = normalize_sha256_input(&self.expected_sha256);
        let should_eject = self.auto_eject;
        let scheme = self.partition_scheme;

        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel_clone = Arc::clone(&self.cancel_flag);
        let status_clone = Arc::clone(&self.status);
        let logs_clone = Arc::clone(&self.logs);

        push_log(&logs_clone, "────────────────────────────────────────────────────────");
        push_log(
            &logs_clone,
            format!(
                "STARTING FLASH JOB -> Target: {} ({}) [{}]",
                drive.model,
                drive.id,
                format_bytes(drive.size_bytes)
            ),
        );
        push_log(
            &logs_clone,
            format!(
                "Target Raw Device Node: {} (Bus: {})",
                drive.raw_device_path.display(),
                drive.bus
            ),
        );
        push_log(
            &logs_clone,
            format!(
                "Unmounting volumes, acquiring exclusive lock & clearing stale Backup GPT on {}...",
                drive.id
            ),
        );

        *status_clone.lock().unwrap() = JobStatus::Running {
            phase: "Unmounting & preparing target drive...".into(),
            progress: 0.0,
            speed_mbps: 0.0,
            bytes_done: 0,
            total_bytes: 1,
            elapsed_secs: 0,
            eta_secs: None,
        };

        thread::spawn(move || {
            let _sleep_guard = SleepInhibitor::acquire();
            let job_start = Instant::now();
            let mut last_phase = String::new();
            let mut last_logged_bucket: u64 = 0;

            let result = flash_and_verify_with_scheme(&image_path, &drive, scheme, Some(&cancel_clone), |prog| {
                let fraction = if prog.total_bytes > 0 {
                    (prog.bytes_processed as f32 / prog.total_bytes as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                let elapsed_secs = job_start.elapsed().as_secs();
                let eta_secs = if prog.speed_mbps > 0.08 && prog.total_bytes > prog.bytes_processed {
                    let rem_mb =
                        (prog.total_bytes - prog.bytes_processed) as f64 / (1024.0 * 1024.0);
                    Some((rem_mb / prog.speed_mbps).round() as u64)
                } else {
                    None
                };

                if prog.phase != last_phase {
                    last_phase = prog.phase.to_string();
                    last_logged_bucket = 0;

                    if last_phase.contains("Writing") {
                        push_log(
                            &logs_clone,
                            format!(
                                "Opened {} in unbuffered raw mode (Sector 0 deferred until verification finishes)",
                                drive.raw_device_path.display()
                            ),
                        );
                    } else if last_phase.contains("Flushing") {
                        push_log(
                            &logs_clone,
                            "Write stream complete — flushing physical controller cache...",
                        );
                    } else if last_phase.contains("Verifying") {
                        push_log(
                            &logs_clone,
                            format!(
                                "Hardware cache flushed — verifying {} via physical sector SHA-256 read-back...",
                                format_bytes(prog.total_bytes)
                            ),
                        );
                    } else {
                        push_log(&logs_clone, format!("Phase: {}", last_phase));
                    }
                }

                let pct = (fraction * 100.0) as u64;
                let bucket = pct / 25;
                if bucket > last_logged_bucket && bucket <= 4 && prog.bytes_processed > 0 {
                    last_logged_bucket = bucket;
                    push_log(
                        &logs_clone,
                        format!(
                            "{} -> {}% ({} / {}) @ {:.1} MB/s",
                            last_phase,
                            bucket * 25,
                            format_bytes(prog.bytes_processed),
                            format_bytes(prog.total_bytes),
                            prog.speed_mbps
                        ),
                    );
                }

                *status_clone.lock().unwrap() = JobStatus::Running {
                    phase: prog.phase.to_string(),
                    progress: fraction,
                    speed_mbps: prog.speed_mbps,
                    bytes_done: prog.bytes_processed,
                    total_bytes: prog.total_bytes,
                    elapsed_secs,
                    eta_secs,
                };
            });

            let total_elapsed = job_start.elapsed().as_secs();
            match result {
                Ok(hash) => {
                    push_log(&logs_clone, format!("Verified SHA-256: {}", hash));
                    if let Some(exp) = expected_opt {
                        if exp != hash {
                            let err_msg = format!(
                                "Expected SHA-256 mismatch! Expected: {}, Flashed: {}",
                                exp, hash
                            );
                            push_log(&logs_clone, format!("[ERROR] {}", err_msg));
                            *status_clone.lock().unwrap() = JobStatus::Error(err_msg);
                            return;
                        } else {
                            push_log(
                                &logs_clone,
                                "[OK] Flashed stream matches your expected SHA-256 checksum!",
                            );
                        }
                    }

                    let mut ejected = false;
                    if should_eject {
                        if eject_drive(&drive).is_ok() {
                            ejected = true;
                            push_log(
                                &logs_clone,
                                format!("[OK] Safely powered off & ejected {}", drive.id),
                            );
                        }
                    }

                    push_log(
                        &logs_clone,
                        format!(
                            "[OK] Boot record committed & sector-by-sector verification matched in {}",
                            format_duration(total_elapsed)
                        ),
                    );
                    *status_clone.lock().unwrap() = JobStatus::Success {
                        hash,
                        elapsed_secs: total_elapsed,
                        ejected,
                    };
                }
                Err(err) => {
                    let err_msg = format!("{:#}", err);
                    push_log(&logs_clone, format!("[ERROR] {}", err_msg));
                    *status_clone.lock().unwrap() = JobStatus::Error(err_msg);
                }
            }
        });
    }
}

impl eframe::App for BootLiteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let current_status = self.status.lock().unwrap().clone();
        let is_running = matches!(current_status, JobStatus::Running { .. });
        let is_hovering_file = !is_running && ctx.input(|i| !i.raw.hovered_files.is_empty());

        if ctx.input(|i| i.viewport().close_requested()) && is_running && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_modal = true;
        }

        if is_running {
            ctx.request_repaint();
        } else {
            if self.last_hotplug_poll.elapsed() >= Duration::from_millis(2500) {
                self.refresh_drives(false);
            }
            ctx.request_repaint_after(Duration::from_millis(1000));

            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped.first() {
                if let Some(path) = &file.path {
                    self.set_image(path.clone());
                }
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG_APP).inner_margin(20.0))
            .show(ctx, |ui| {
                // HEADER
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
                                "v0.3.3",
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
                                if let Err(e) = relaunch_elevated(self.image_path.as_ref()) {
                                    *self.status.lock().unwrap() = JobStatus::Error(e.to_string());
                                }
                            }
                        }
                    });
                });

                ui.add_space(8.0);

                // 3-STAGE PIPELINE STEPPER BAR
                let step1_done = self.image_path.is_some();
                let step2_done = self.selected_drive_idx.is_some();
                let step3_done = matches!(current_status, JobStatus::Success { .. });

                egui::Frame::default()
                    .fill(Color32::from_rgb(15, 21, 36))
                    .stroke(Stroke::new(1.0_f32, BORDER_DIM))
                    .rounding(8.0)
                    .inner_margin(egui::Margin::symmetric(14.0, 7.0))
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

                ui.add_space(8.0);

                // STEP 1: INTERACTIVE IMAGE DROPZONE CARD + OPTIONAL SHA-256 CHECKSUM BAR
                let has_img = self.image_path.is_some();
                let has_warn = self
                    .image_meta
                    .as_ref()
                    .and_then(|m| m.warning.as_ref())
                    .is_some();

                let card1_border = if is_hovering_file {
                    ACCENT_GLOW
                } else if has_warn {
                    ACCENT_WARN
                } else if has_img {
                    Color32::from_rgb(37, 99, 235)
                } else {
                    BORDER_DIM
                };
                let card1_fill = if is_hovering_file {
                    BG_SELECTED
                } else {
                    BG_CARD
                };

                egui::Frame::default()
                    .fill(card1_fill)
                    .stroke(Stroke::new(1.4_f32, card1_border))
                    .rounding(12.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            draw_disc_icon(ui, has_img || is_hovering_file);
                            ui.add_space(6.0);

                            ui.vertical(|ui| {
                                if is_hovering_file {
                                    ui.label(
                                        RichText::new("Release Mouse to Load Bootable Disk Image")
                                            .size(14.0)
                                            .strong()
                                            .color(ACCENT_GLOW),
                                    );
                                    ui.label(
                                        RichText::new("Supports .iso, .img, .dmg, .gz, .bz2, .xz, and .zst")
                                            .size(12.0)
                                            .color(TEXT_MAIN),
                                    );
                                } else {
                                    match &self.image_path {
                                        Some(p) => {
                                            let name = p
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| p.display().to_string());
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(name)
                                                        .size(14.0)
                                                        .strong()
                                                        .color(TEXT_MAIN),
                                                );
                                                draw_pill(
                                                    ui,
                                                    &format_bytes(self.image_size),
                                                    Color32::from_rgb(30, 58, 138),
                                                    ACCENT_GLOW,
                                                );
                                                if let Some(meta) = &self.image_meta {
                                                    let (bg, fg) = if meta.warning.is_some() {
                                                        (Color32::from_rgb(69, 26, 3), ACCENT_WARN)
                                                    } else {
                                                        (Color32::from_rgb(6, 78, 59), ACCENT_SUCCESS)
                                                    };
                                                    draw_pill(ui, &meta.scheme, bg, fg);
                                                }
                                            });
                                            if let Some(warn) = self
                                                .image_meta
                                                .as_ref()
                                                .and_then(|m| m.warning.as_ref())
                                            {
                                                ui.label(
                                                    RichText::new(warn)
                                                        .size(11.0)
                                                        .color(ACCENT_WARN),
                                                );
                                            } else {
                                                ui.label(
                                                    RichText::new(p.display().to_string())
                                                        .size(11.5)
                                                        .color(TEXT_SUB),
                                                );
                                            }
                                        }
                                        None => {
                                            ui.label(
                                                RichText::new("Select Bootable OS Image")
                                                    .size(14.0)
                                                    .strong()
                                                    .color(TEXT_MAIN),
                                            );
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new("Drag & drop or browse")
                                                        .size(12.0)
                                                        .color(TEXT_SUB),
                                                );
                                                for ext in ["ISO", "IMG", "DMG", "GZ", "BZ2", "XZ", "ZST"] {
                                                    draw_pill(
                                                        ui,
                                                        ext,
                                                        Color32::from_rgb(30, 41, 59),
                                                        TEXT_SUB,
                                                    );
                                                }
                                            });
                                        }
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
                                    .min_size(Vec2::new(115.0, 34.0));

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

                        if has_img {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Expected SHA-256:")
                                        .size(11.0)
                                        .color(TEXT_SUB),
                                );
                                let edit = egui::TextEdit::singleline(&mut self.expected_sha256)
                                    .hint_text("Optional: paste official SHA-256 checksum to verify image...")
                                    .desired_width(330.0)
                                    .font(egui::TextStyle::Monospace);
                                ui.add_enabled(!is_running, edit);

                                ui.add_enabled_ui(!is_running, |ui| {
                                    if !self.expected_sha256.is_empty() && ui.small_button("Clear").clicked() {
                                        self.expected_sha256.clear();
                                    }
                                    if ui.small_button("Check File Hash").clicked() {
                                        self.start_file_hash_check();
                                    }
                                });
                            });
                        }
                    });

                ui.add_space(8.0);

                // STEP 2: TARGET USB DRIVE SELECTOR CARD + RESET / FORMAT USB UTILITY
                let is_compressed = self
                    .image_meta
                    .as_ref()
                    .map(|m| m.is_compressed)
                    .unwrap_or(false);
                let is_win_iso = self
                    .image_meta
                    .as_ref()
                    .map(|m| m.is_windows_udf_iso)
                    .unwrap_or(false);
                let required_bytes = if is_win_iso {
                    self.image_size + WIN_ISO_OVERHEAD_BYTES
                } else {
                    self.image_size
                };

                egui::Frame::default()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.2_f32, BORDER_DIM))
                    .rounding(12.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("TARGET DESTINATION DRIVE (AUTO-HOTPLUG)")
                                    .size(11.0)
                                    .strong()
                                    .color(TEXT_SUB),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(!is_running, |ui| {
                                    let refresh_btn = egui::Button::new(
                                        RichText::new("Rescan").size(11.5).color(TEXT_MAIN),
                                    )
                                    .fill(Color32::from_rgb(30, 41, 59))
                                    .rounding(6.0);

                                    if ui.add(refresh_btn).clicked() {
                                        self.refresh_drives(true);
                                    }

                                    if self.selected_drive_idx.is_some() && !self.drives.is_empty() {
                                        let reset_btn = egui::Button::new(
                                            RichText::new("Reset / Format USB")
                                                .size(11.5)
                                                .color(ACCENT_GLOW),
                                        )
                                        .fill(Color32::from_rgb(23, 37, 84))
                                        .rounding(6.0);

                                        if ui.add(reset_btn).clicked() {
                                            if !self.has_root {
                                                if let Err(e) =
                                                    relaunch_elevated(self.image_path.as_ref())
                                                {
                                                    *self.status.lock().unwrap() =
                                                        JobStatus::Error(e.to_string());
                                                }
                                            } else {
                                                self.show_reset_modal = true;
                                            }
                                        }
                                    }

                                    ui.add_space(4.0);
                                    if draw_toggle_switch(ui, &mut self.allow_virtual).changed() {
                                        self.refresh_drives(true);
                                    }
                                    ui.label(
                                        RichText::new("Virtual Disks")
                                            .size(11.0)
                                            .color(TEXT_SUB),
                                    );
                                });
                            });
                        });

                        ui.add_space(6.0);

                        if self.drives.is_empty() {
                            egui::Frame::default()
                                .fill(BG_APP)
                                .stroke(Stroke::new(1.0_f32, BORDER_DIM))
                                .rounding(10.0)
                                .inner_margin(12.0)
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("No External USB / SD Drive Connected")
                                                .size(13.0)
                                                .strong()
                                                .color(TEXT_MAIN),
                                        );
                                        ui.label(
                                            RichText::new(
                                                "Plug in a USB flash drive or SD card — BootLite will detect it automatically.",
                                            )
                                            .size(11.5)
                                            .color(TEXT_SUB),
                                        );
                                    });
                                });
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("drives_scroll")
                                .max_height(100.0)
                                .show(ui, |ui| {
                                    for (idx, d) in self.drives.iter().enumerate() {
                                        let is_selected = self.selected_drive_idx == Some(idx);
                                        let too_small = has_img
                                            && !is_compressed
                                            && required_bytes > d.size_bytes;
                                        let source_on_target = self
                                            .image_path
                                            .as_ref()
                                            .map(|p| is_image_on_drive(p, d))
                                            .unwrap_or(false);

                                        let fill = if is_selected { BG_SELECTED } else { BG_APP };
                                        let border = if too_small || source_on_target {
                                            ACCENT_DANGER
                                        } else if is_selected {
                                            ACCENT_PRIMARY
                                        } else {
                                            BORDER_DIM
                                        };

                                        let resp = egui::Frame::default()
                                            .fill(fill)
                                            .stroke(Stroke::new(1.5_f32, border))
                                            .rounding(10.0)
                                            .inner_margin(9.0)
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    draw_usb_icon(ui, is_selected);
                                                    ui.add_space(6.0);
                                                    ui.vertical(|ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                RichText::new(&d.model)
                                                                    .size(13.5)
                                                                    .strong()
                                                                    .color(TEXT_MAIN),
                                                            );
                                                            draw_pill(
                                                                ui,
                                                                &d.bus,
                                                                Color32::from_rgb(30, 41, 59),
                                                                TEXT_SUB,
                                                            );
                                                            if too_small {
                                                                draw_pill(
                                                                    ui,
                                                                    "TOO SMALL",
                                                                    Color32::from_rgb(127, 29, 29),
                                                                    Color32::from_rgb(252, 165, 165),
                                                                );
                                                            }
                                                            if source_on_target {
                                                                draw_pill(
                                                                    ui,
                                                                    "CONTAINS SOURCE ISO",
                                                                    Color32::from_rgb(127, 29, 29),
                                                                    Color32::from_rgb(252, 165, 165),
                                                                );
                                                            }
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
                                            push_log(
                                                &self.logs,
                                                format!(
                                                    "Selected target device: {} ({}) -> {}",
                                                    d.model,
                                                    d.id,
                                                    d.raw_device_path.display()
                                                ),
                                            );
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                        }
                    });

                ui.add_space(8.0);

                // STEP 3: ACTION BAR, AUTO-EJECT TOGGLE & LIVE TELEMETRY
                let selected_drive_valid = self
                    .selected_drive_idx
                    .and_then(|idx| self.drives.get(idx))
                    .map(|d| {
                        let fits = is_compressed || required_bytes <= d.size_bytes;
                        let not_same = self
                            .image_path
                            .as_ref()
                            .map(|p| !is_image_on_drive(p, d))
                            .unwrap_or(true);
                        fits && not_same
                    })
                    .unwrap_or(false);

                let can_flash = !is_running
                    && self.image_path.is_some()
                    && selected_drive_valid
                    && !self.drives.is_empty();

                match current_status {
                    JobStatus::Idle => {
                        ui.horizontal(|ui| {
                            draw_toggle_switch(ui, &mut self.auto_eject);
                            ui.label(
                                RichText::new("Auto-Eject")
                                    .size(11.5)
                                    .color(TEXT_SUB),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Scheme:")
                                    .size(11.5)
                                    .color(TEXT_SUB),
                            );
                            for (s, lbl) in [
                                (PartitionScheme::Auto, "Auto"),
                                (PartitionScheme::Gpt, "GPT"),
                                (PartitionScheme::Mbr, "MBR"),
                            ] {
                                let active = self.partition_scheme == s;
                                let btn = egui::Button::new(
                                    RichText::new(lbl)
                                        .size(11.0)
                                        .strong()
                                        .color(if active { Color32::WHITE } else { TEXT_SUB }),
                                )
                                .fill(if active {
                                    ACCENT_PRIMARY
                                } else {
                                    Color32::from_rgb(30, 41, 59)
                                })
                                .rounding(5.0);
                                if ui.add(btn).clicked() {
                                    self.partition_scheme = s;
                                }
                            }

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
                                    .min_size(Vec2::new(210.0, 40.0));

                                    let resp = ui.add(btn);
                                    if resp.hovered() && can_flash {
                                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    if resp.clicked() {
                                        if !self.has_root {
                                            if let Err(e) =
                                                relaunch_elevated(self.image_path.as_ref())
                                            {
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
                        elapsed_secs,
                        eta_secs,
                    } => {
                        egui::Frame::default()
                            .fill(BG_CARD)
                            .stroke(Stroke::new(1.2_f32, ACCENT_PRIMARY))
                            .rounding(10.0)
                            .inner_margin(12.0)
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
                                                push_log(
                                                    &self.logs,
                                                    "[WARN] User requested abort — stopping I/O stream...",
                                                );
                                                self.cancel_flag.store(true, Ordering::Relaxed);
                                            }
                                            ui.add_space(6.0);
                                            let eta_label = match eta_secs {
                                                Some(s) => format!("ETA {}", format_duration(s)),
                                                None => "ETA calculating...".to_string(),
                                            };
                                            draw_pill(
                                                ui,
                                                &eta_label,
                                                Color32::from_rgb(30, 41, 59),
                                                ACCENT_WARN,
                                            );
                                            draw_pill(
                                                ui,
                                                &format!("Elapsed {}", format_duration(elapsed_secs)),
                                                Color32::from_rgb(30, 41, 59),
                                                TEXT_SUB,
                                            );
                                            draw_pill(
                                                ui,
                                                &format!("{:.1} MB/s", speed_mbps),
                                                Color32::from_rgb(30, 58, 138),
                                                ACCENT_GLOW,
                                            );
                                        },
                                    );
                                });
                                ui.add_space(6.0);
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
                    JobStatus::Success {
                        hash,
                        elapsed_secs,
                        ejected,
                    } => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(6, 46, 34))
                            .stroke(Stroke::new(1.2_f32, ACCENT_SUCCESS))
                            .rounding(10.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    draw_pill(
                                        ui,
                                        "VERIFIED",
                                        Color32::from_rgb(6, 78, 59),
                                        ACCENT_SUCCESS,
                                    );
                                    let suffix = if ejected { " — Safely Ejected" } else { "" };
                                    ui.label(
                                        RichText::new(format!(
                                            "Flash & SHA-256 Read-Back Complete ({}){}",
                                            format_duration(elapsed_secs),
                                            suffix
                                        ))
                                        .size(13.0)
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
                    JobStatus::ResetSuccess {
                        drive_name,
                        elapsed_secs,
                    } => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(6, 46, 34))
                            .stroke(Stroke::new(1.2_f32, ACCENT_SUCCESS))
                            .rounding(10.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    draw_pill(
                                        ui,
                                        "RESTORED",
                                        Color32::from_rgb(6, 78, 59),
                                        ACCENT_SUCCESS,
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "{} formatted as full-capacity ExFAT ({})",
                                            drive_name,
                                            format_duration(elapsed_secs)
                                        ))
                                        .size(13.0)
                                        .strong()
                                        .color(ACCENT_SUCCESS),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("Done").clicked() {
                                                *self.status.lock().unwrap() = JobStatus::Idle;
                                            }
                                        },
                                    );
                                });
                            });
                    }
                    JobStatus::Error(err) => {
                        egui::Frame::default()
                            .fill(Color32::from_rgb(55, 12, 12))
                            .stroke(Stroke::new(1.2_f32, ACCENT_DANGER))
                            .rounding(10.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    draw_pill(
                                        ui,
                                        "FAILED",
                                        Color32::from_rgb(127, 29, 29),
                                        Color32::from_rgb(252, 165, 165),
                                    );
                                    ui.label(
                                        RichText::new(&err)
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

                ui.add_space(8.0);

                // RUFUS-STYLE BEHIND-THE-SCENES LIVE ENGINE LOG CONSOLE
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("LIVE HARDWARE & VERIFICATION LOG")
                            .size(10.5)
                            .strong()
                            .color(TEXT_SUB),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let toggle_text = if self.show_log_drawer {
                            "Hide Log"
                        } else {
                            "Show Log"
                        };
                        if ui.small_button(toggle_text).clicked() {
                            self.show_log_drawer = !self.show_log_drawer;
                        }
                        if ui.small_button("Clear").clicked() {
                            self.logs.lock().unwrap().clear();
                        }
                        if ui.small_button("Copy Log").clicked() {
                            let full_log = self.logs.lock().unwrap().join("\n");
                            ctx.copy_text(full_log);
                        }
                    });
                });

                if self.show_log_drawer {
                    egui::Frame::default()
                        .fill(BG_TERMINAL)
                        .stroke(Stroke::new(1.0_f32, BORDER_DIM))
                        .rounding(8.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("rufus_log_console")
                                .max_height(130.0)
                                .min_scrolled_height(130.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    let entries = self.logs.lock().unwrap().clone();
                                    for line in entries {
                                        let color = if line.contains("[OK]") {
                                            ACCENT_SUCCESS
                                        } else if line.contains("[ERROR]") {
                                            ACCENT_DANGER
                                        } else if line.contains("[WARN]") {
                                            ACCENT_WARN
                                        } else if line.contains("STARTING FLASH JOB")
                                            || line.contains("Verified SHA-256")
                                            || line.contains("RESETTING & FORMATTING")
                                        {
                                            ACCENT_GLOW
                                        } else {
                                            Color32::from_rgb(165, 180, 202)
                                        };
                                        ui.label(RichText::new(line).size(11.0).monospace().color(color));
                                    }
                                });
                        });
                }
            });

        // SAFETY CONFIRMATION MODAL (FLASH)
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

        // RESET / FORMAT USB MODAL
        if self.show_reset_modal {
            egui::Window::new("Reset & Format USB Drive to ExFAT")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    if let Some(idx) = self.selected_drive_idx {
                        if let Some(d) = self.drives.get(idx) {
                            ui.label(
                                RichText::new(format!(
                                    "Restore {} ({}) [{}] to full capacity?",
                                    d.model,
                                    d.id,
                                    format_bytes(d.size_bytes)
                                ))
                                .size(14.0)
                                .strong()
                                .color(TEXT_MAIN),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "This wipes all MBR, GPT, and Backup GPT boot tables and formats the drive as a clean ExFAT volume.",
                                )
                                .size(12.0)
                                .color(ACCENT_WARN),
                            );
                        }
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let fmt_btn = egui::Button::new(
                            RichText::new("Wipe & Format ExFAT")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_PRIMARY)
                        .rounding(7.0);

                        if ui.add(fmt_btn).clicked() {
                            self.show_reset_modal = false;
                            self.start_reset_drive();
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_reset_modal = false;
                        }
                    });
                });
        }

        // CLOSE-WHILE-FLASHING GUARD MODAL
        if self.show_exit_modal {
            egui::Window::new("Flash Operation in Progress")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(350.0);
                    ui.label(
                        RichText::new("BootLite is currently writing or verifying a USB drive.")
                            .size(13.5)
                            .strong()
                            .color(TEXT_MAIN),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Closing now will abort the operation and leave the USB drive unbootable.",
                        )
                        .size(12.0)
                        .color(ACCENT_WARN),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let abort_btn = egui::Button::new(
                            RichText::new("Abort Flash & Exit")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_DANGER)
                        .rounding(7.0);

                        if ui.add(abort_btn).clicked() {
                            self.cancel_flag.store(true, Ordering::Relaxed);
                            self.allow_close = true;
                            self.show_exit_modal = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Continue Flashing").clicked() {
                            self.show_exit_modal = false;
                        }
                    });
                });
        }
    }
}
