use crate::UsbDrive;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let output = Command::new("diskutil")
        .arg("list")
        .output()
        .context("Failed to execute diskutil list")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut candidate_ids = Vec::new();

    for line in stdout.lines() {
        if line.starts_with("/dev/disk") && !line.contains("synthesized") {
            let is_external = line.contains("external");
            let is_disk_image = line.contains("disk image");
            if is_external || (allow_virtual && is_disk_image) {
                if let Some(dev) = line.split_whitespace().next() {
                    if let Some(id) = dev.strip_prefix("/dev/") {
                        candidate_ids.push((id.to_string(), is_disk_image));
                    }
                }
            }
        }
    }

    let mut drives = Vec::new();
    for (id, is_disk_image) in candidate_ids {
        let info_out = Command::new("diskutil")
            .args(["info", &id])
            .output()
            .context("Failed to execute diskutil info")?;
        let info = String::from_utf8_lossy(&info_out.stdout);

        let mut model = String::from("USB Storage");
        let mut bus = String::from("USB");
        let mut size_bytes: u64 = 0;
        let mut is_internal = false;
        let mut is_removable = false;

        for line in info.lines() {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("Device / Media Name:") {
                model = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("Protocol:") {
                bus = val.trim().to_string();
            } else if let Some(val) = trimmed.strip_prefix("Disk Size:") {
                if let Some(start) = val.find('(') {
                    if let Some(end) = val[start + 1..].find(" Bytes") {
                        let num_str = val[start + 1..start + 1 + end].trim();
                        size_bytes = num_str.parse::<u64>().unwrap_or(0);
                    }
                }
            } else if let Some(val) = trimmed.strip_prefix("Internal:") {
                if val.trim().eq_ignore_ascii_case("Yes") {
                    is_internal = true;
                }
            } else if let Some(val) = trimmed.strip_prefix("Removable Media:") {
                let v = val.trim();
                if v.eq_ignore_ascii_case("Removable") || v.eq_ignore_ascii_case("Yes") {
                    is_removable = true;
                }
            }
        }

        let is_sd_card = bus.eq_ignore_ascii_case("Secure Digital")
            || model.to_lowercase().contains("sd card");

        if is_internal && !is_disk_image && !(is_sd_card && is_removable) {
            continue;
        }
        if bus.eq_ignore_ascii_case("Apple Fabric") || size_bytes == 0 {
            continue;
        }

        drives.push(UsbDrive {
            raw_device_path: PathBuf::from(format!("/dev/r{}", id)),
            id,
            model,
            size_bytes,
            bus: if is_sd_card { "SDXC".to_string() } else { bus },
            is_system: false,
            is_virtual: is_disk_image,
        });
    }

    drives.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes).then_with(|| a.id.cmp(&b.id)));
    Ok(drives)
}

pub fn is_path_on_drive(path: &Path, drive: &UsbDrive) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    let Ok(out) = Command::new("df").arg(&canonical).output() else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let prefix = format!("/dev/{}", drive.id);
    for line in stdout.lines().skip(1) {
        if let Some(dev) = line.split_whitespace().next() {
            if dev == prefix || dev.starts_with(&format!("{}s", prefix)) {
                return true;
            }
        }
    }
    false
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to unmount internal system disk");
    }

    let status = Command::new("diskutil")
        .args(["unmountDisk", "force", &format!("/dev/{}", drive.id)])
        .status()
        .context("Failed to execute diskutil unmountDisk")?;

    if !status.success() {
        anyhow::bail!("diskutil failed to unmount /dev/{}", drive.id);
    }
    Ok(())
}

fn enable_nocache(file: &File) -> Result<()> {
    let fd = file.as_raw_fd();
    let ret = unsafe { libc::fcntl(fd, libc::F_NOCACHE, 1) };
    if ret == -1 {
        anyhow::bail!(
            "Failed to enable F_NOCACHE on raw descriptor: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

pub fn open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to open internal system disk for writing");
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} for writing (Click 'Unlock Admin Access' first)",
                drive.raw_device_path.display()
            )
        })?;

    enable_nocache(&file)?;
    Ok(file)
}

pub fn prepare_for_verify(file: &mut File) -> Result<()> {
    let fd = file.as_raw_fd();
    unsafe {
        let _ = libc::fcntl(fd, libc::F_FULLFSYNC, 0);
        let _ = libc::fcntl(fd, libc::F_NOCACHE, 1);
    }
    file.sync_all()
        .context("Failed to flush macOS hardware cache")
}

pub fn finish_device(_file: Option<File>) {}

pub fn eject_drive(drive: &UsbDrive) -> Result<()> {
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to eject internal system disk");
    }
    let status = Command::new("diskutil")
        .args(["eject", &format!("/dev/{}", drive.id)])
        .status()
        .context("Failed to execute diskutil eject")?;
    if !status.success() {
        anyhow::bail!("diskutil failed to eject /dev/{}", drive.id);
    }
    Ok(())
}

pub fn format_drive_exfat(drive: &UsbDrive, label: &str) -> Result<()> {
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to format internal system disk");
    }
    let status = Command::new("diskutil")
        .args([
            "eraseDisk",
            "ExFAT",
            label,
            "MBRFormat",
            &format!("/dev/{}", drive.id),
        ])
        .status()
        .context("Failed to execute diskutil eraseDisk")?;
    if !status.success() {
        anyhow::bail!("diskutil eraseDisk failed on /dev/{}", drive.id);
    }
    Ok(())
}
