use crate::UsbDrive;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const FILE_FLAG_WRITE_THROUGH: u32 = 0x80000000;
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;
const CREATE_NO_WINDOW: u32 = 0x08000000;

const IOCTL_DISK_DELETE_DRIVE_LAYOUT: u32 = 0x0007C100;
const IOCTL_DISK_UPDATE_PROPERTIES: u32 = 0x00070140;
const FSCTL_LOCK_VOLUME: u32 = 0x00090018;
const FSCTL_DISMOUNT_VOLUME: u32 = 0x00090020;
const FSCTL_ALLOW_EXTENDED_DASD_IO: u32 = 0x00090083;

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(
        h_device: RawHandle,
        dw_io_control_code: u32,
        lp_in_buffer: *const c_void,
        n_in_buffer_size: u32,
        lp_out_buffer: *mut c_void,
        n_out_buffer_size: u32,
        lp_bytes_returned: *mut u32,
        lp_overlapped: *mut c_void,
    ) -> i32;
}

fn ioctl_none(handle: RawHandle, code: u32) -> bool {
    let mut bytes_returned: u32 = 0;
    unsafe {
        DeviceIoControl(
            handle,
            code,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        ) != 0
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WinDisk {
    number: u32,
    friendly_name: Option<String>,
    size: u64,
    bus_type: Option<String>,
    is_system: Option<bool>,
    is_boot: Option<bool>,
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let ps_script = "Get-Disk | Select-Object Number, FriendlyName, Size, BusType, IsSystem, IsBoot | ConvertTo-Json -Compress";
    let output = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-Command", ps_script])
        .output()
        .context("Failed to execute PowerShell Get-Disk")?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    let disks: Vec<WinDisk> = if stdout.starts_with('[') {
        serde_json::from_str(&stdout).unwrap_or_default()
    } else if let Ok(single) = serde_json::from_str::<WinDisk>(&stdout) {
        vec![single]
    } else {
        Vec::new()
    };

    let mut drives = Vec::new();
    for d in disks {
        if d.is_system.unwrap_or(false) || d.is_boot.unwrap_or(false) || d.number == 0 {
            continue;
        }
        let bus = d.bus_type.unwrap_or_else(|| "Unknown".to_string());
        let is_usb = bus.eq_ignore_ascii_case("USB");
        let is_virtual = bus.eq_ignore_ascii_case("File Backed Virtual")
            || bus.eq_ignore_ascii_case("Spaces");

        if !is_usb && !(allow_virtual && is_virtual) {
            continue;
        }

        let id = format!("PhysicalDrive{}", d.number);
        let raw_path = PathBuf::from(format!(r"\\.\{}", id));
        drives.push(UsbDrive {
            id,
            model: d.friendly_name.unwrap_or_else(|| "USB Storage".to_string()),
            size_bytes: d.size,
            raw_device_path: raw_path,
            bus,
            is_system: false,
            is_virtual,
        });
    }

    Ok(drives)
}

fn extract_disk_number(drive: &UsbDrive) -> Option<u32> {
    drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    let Some(disk_num) = extract_disk_number(drive) else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to clean PhysicalDrive0 (system boot drive)");
    }

    // 1. Lock & dismount any mounted drive letters on this disk via Win32 FSCTLs
    let query_letters = format!(
        "(Get-Partition -DiskNumber {} -ErrorAction SilentlyContinue | Where-Object DriveLetter | ForEach-Object {{ $_.DriveLetter }})",
        disk_num
    );
    if let Ok(out) = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-Command", &query_letters])
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(ch) = line.trim().chars().next() {
                if ch.is_ascii_alphabetic() {
                    let vol_path = format!(r"\\.\{}:", ch);
                    if let Ok(vol_file) = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                        .open(&vol_path)
                    {
                        let h = vol_file.as_raw_handle();
                        let _ = ioctl_none(h, FSCTL_LOCK_VOLUME);
                        let _ = ioctl_none(h, FSCTL_DISMOUNT_VOLUME);
                    }
                }
            }
        }
    }

    // 2. Run diskpart clean to strip readonly attributes and destroy any hybrid ISO / EFI / GPT partitions
    if let Ok(mut child) = Command::new("diskpart")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let script = format!(
                "select disk {}\r\nattributes disk clear readonly\r\nclean\r\nrescan\r\nexit\r\n",
                disk_num
            );
            let _ = stdin.write_all(script.as_bytes());
        }
        let _ = child.wait();
    }

    Ok(())
}

pub fn open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.id == "PhysicalDrive0" || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to open PhysicalDrive0 for writing");
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_WRITE_THROUGH)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} for raw write (Ensure BootLite is running as Administrator)",
                drive.raw_device_path.display()
            )
        })?;

    let handle = file.as_raw_handle();
    let _ = ioctl_none(handle, FSCTL_ALLOW_EXTENDED_DASD_IO);
    let _ = ioctl_none(handle, IOCTL_DISK_DELETE_DRIVE_LAYOUT);
    let _ = ioctl_none(handle, IOCTL_DISK_UPDATE_PROPERTIES);

    Ok(file)
}

pub fn open_raw_device_read(drive: &UsbDrive) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} for verification read-back",
                drive.raw_device_path.display()
            )
        })?;

    let handle = file.as_raw_handle();
    let _ = ioctl_none(handle, FSCTL_ALLOW_EXTENDED_DASD_IO);

    Ok(file)
}

pub fn flush_and_sync(file: &mut File) -> Result<()> {
    file.sync_all()
        .context("Failed to flush Windows hardware buffers")?;
    let _ = ioctl_none(file.as_raw_handle(), IOCTL_DISK_UPDATE_PROPERTIES);
    Ok(())
}
