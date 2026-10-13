use crate::UsbDrive;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs::{self, File};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Command;

const FILE_FLAG_WRITE_THROUGH: u32 = 0x80000000;
const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;

#[derive(Deserialize)]
struct WinDisk {
    #[serde(rename = "Number")]
    number: u32,
    #[serde(rename = "FriendlyName", default)]
    friendly_name: String,
    #[serde(rename = "Size", default)]
    size: u64,
    #[serde(rename = "BusType", default)]
    bus_type: String,
    #[serde(rename = "IsSystem", default)]
    is_system: bool,
    #[serde(rename = "IsBoot", default)]
    is_boot: bool,
}

pub fn list_drives(_include_virtual: bool) -> Result<Vec<UsbDrive>> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-Disk | Select-Object Number,FriendlyName,Size,BusType,IsSystem,IsBoot | ConvertTo-Json -Compress",
        ])
        .output()
        .context("Failed to query Windows disks via PowerShell")?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    let disks: Vec<WinDisk> = if stdout.trim_start().starts_with('[') {
        serde_json::from_str(&stdout).unwrap_or_default()
    } else {
        serde_json::from_str::<WinDisk>(&stdout)
            .map(|d| vec![d])
            .unwrap_or_default()
    };

    let mut result = Vec::new();
    for d in disks {
        let is_sys = d.is_system || d.is_boot || d.number == 0;
        let is_usb =
            d.bus_type.eq_ignore_ascii_case("USB") || d.bus_type.eq_ignore_ascii_case("SD");

        if is_usb && !is_sys && d.size > 0 {
            let id = format!("PhysicalDrive{}", d.number);
            result.push(UsbDrive {
                raw_device_path: PathBuf::from(format!(r"\\.\{}", id)),
                id,
                model: d.friendly_name,
                bus: d.bus_type,
                size_bytes: d.size,
                is_system: is_sys,
                is_virtual: false,
            });
        }
    }
    Ok(result)
}

pub fn prepare_and_open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.is_system {
        bail!("SAFETY ABORT: Refusing to open Windows system disk!");
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {}. Run terminal as Administrator.",
                drive.raw_device_path.display()
            )
        })?;

    Ok(file)
}
