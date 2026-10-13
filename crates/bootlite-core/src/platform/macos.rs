use crate::UsbDrive;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize)]
struct DiskListPlist {
    #[serde(rename = "WholeDisks", default)]
    whole_disks: Vec<String>,
}

#[derive(Deserialize)]
struct DiskInfoPlist {
    #[serde(rename = "DeviceIdentifier", default)]
    device_identifier: String,
    #[serde(rename = "MediaName", default)]
    media_name: String,
    #[serde(rename = "BusProtocol", default)]
    bus_protocol: String,
    #[serde(rename = "TotalSize", default)]
    total_size: u64,
    #[serde(rename = "Internal", default)]
    internal: bool,
    #[serde(rename = "Removable", default)]
    removable: bool,
    #[serde(rename = "Ejectable", default)]
    ejectable: bool,
}

pub fn list_drives(include_virtual: bool) -> Result<Vec<UsbDrive>> {
    let list_out = Command::new("diskutil")
        .args(["list", "-plist"])
        .output()
        .context("Failed to execute diskutil list")?;

    let parsed: DiskListPlist = plist::from_bytes(&list_out.stdout)
        .context("Failed to parse diskutil plist")?;

    let mut drives = Vec::new();
    for disk_id in parsed.whole_disks {
        let info_out = Command::new("diskutil")
            .args(["info", "-plist", &disk_id])
            .output()?;

        let info: DiskInfoPlist = match plist::from_bytes(&info_out.stdout) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // HARD SAFETY LOCK: Never allow internal drives or Apple Fabric SSD containers
        if info.internal || info.bus_protocol.eq_ignore_ascii_case("Apple Fabric") {
            continue;
        }

        let is_disk_image = info.bus_protocol.eq_ignore_ascii_case("Disk Image");
        if is_disk_image && !include_virtual {
            continue;
        }

        let is_removable = info.removable || info.ejectable;
        if (is_removable || (include_virtual && is_disk_image)) && info.total_size > 0 {
            drives.push(UsbDrive {
                id: info.device_identifier.clone(),
                raw_device_path: PathBuf::from(format!("/dev/r{}", info.device_identifier)),
                model: if info.media_name.is_empty() {
                    "External Drive".into()
                } else {
                    info.media_name
                },
                bus: info.bus_protocol,
                size_bytes: info.total_size,
                is_system: false,
                is_virtual: is_disk_image,
            });
        }
    }
    Ok(drives)
}

pub fn prepare_and_open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        bail!("SAFETY ABORT: Refusing to open Apple Fabric internal drive!");
    }

    let status = Command::new("diskutil")
        .args(["unmountDisk", &drive.id])
        .status()
        .context("Failed to unmount macOS disk")?;

    if !status.success() {
        bail!("Could not unmount {}. Close any apps using the drive.", drive.id);
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {}. Run with sudo.",
                drive.raw_device_path.display()
            )
        })?;

    unsafe {
        libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1);
    }
    Ok(file)
}
