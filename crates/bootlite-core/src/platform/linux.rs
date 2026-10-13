use crate::UsbDrive;
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Command;

pub fn list_drives(include_virtual: bool) -> Result<Vec<UsbDrive>> {
    let mut drives = Vec::new();
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();

    for entry in fs::read_dir("/sys/block").context("Failed to read /sys/block")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with("ram") || name.starts_with("zram") {
            continue;
        }
        let is_loop = name.starts_with("loop");
        if is_loop && !include_virtual {
            continue;
        }

        let sys_path = entry.path();
        let removable = fs::read_to_string(sys_path.join("removable"))
            .unwrap_or_default()
            .trim()
            == "1";

        let sectors: u64 = fs::read_to_string(sys_path.join("size"))
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(0);
        let size_bytes = sectors * 512;

        let model = fs::read_to_string(sys_path.join("device/model"))
            .unwrap_or_else(|_| "USB/Loop Device".to_string())
            .trim()
            .to_string();

        let dev_path = format!("/dev/{}", name);
        let is_system = mounts.lines().any(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.len() >= 2
                && parts[0].starts_with(&dev_path)
                && matches!(parts[1], "/" | "/boot" | "/boot/efi" | "/home")
        });

        if (removable || (include_virtual && is_loop)) && !is_system && size_bytes > 0 {
            drives.push(UsbDrive {
                id: name,
                raw_device_path: PathBuf::from(dev_path),
                model,
                bus: if is_loop {
                    "Virtual Loop".into()
                } else {
                    "USB/Removable".into()
                },
                size_bytes,
                is_system,
                is_virtual: is_loop,
            });
        }
    }
    Ok(drives)
}

pub fn prepare_and_open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.is_system {
        bail!("SAFETY ABORT: Refusing to open Linux system disk!");
    }
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("umount /dev/{}?* 2>/dev/null || true", drive.id))
        .status();

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {}. Run with sudo.",
                drive.raw_device_path.display()
            )
        })?;

    Ok(file)
}
