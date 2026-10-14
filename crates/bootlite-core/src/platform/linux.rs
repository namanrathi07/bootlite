use crate::UsbDrive;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    model: Option<String>,
    size: Option<u64>,
    rm: Option<bool>,
    ro: Option<bool>,
    #[serde(rename = "type")]
    dev_type: Option<String>,
    tran: Option<String>,
    mountpoints: Option<Vec<Option<String>>>,
    children: Option<Vec<LsblkDevice>>,
}

impl LsblkDevice {
    fn has_system_mount(&self) -> bool {
        if let Some(mounts) = &self.mountpoints {
            for m in mounts.iter().flatten() {
                if m == "/" || m == "/boot" || m == "/boot/efi" || m == "/home" {
                    return true;
                }
            }
        }
        if let Some(children) = &self.children {
            for c in children {
                if c.has_system_mount() {
                    return true;
                }
            }
        }
        false
    }

    fn collect_mounted_paths(&self, out: &mut Vec<String>) {
        if let Some(mounts) = &self.mountpoints {
            if mounts.iter().flatten().next().is_some() {
                out.push(format!("/dev/{}", self.name));
            }
        }
        if let Some(children) = &self.children {
            for c in children {
                c.collect_mounted_paths(out);
            }
        }
    }
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let output = Command::new("lsblk")
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,MODEL,SIZE,RM,RO,TYPE,TRAN,MOUNTPOINTS",
        ])
        .output()
        .context("Failed to execute lsblk")?;

    let parsed: LsblkOutput =
        serde_json::from_slice(&output.stdout).context("Failed to parse lsblk JSON")?;

    let mut drives = Vec::new();
    for d in parsed.blockdevices {
        if d.has_system_mount() || d.ro.unwrap_or(false) {
            continue;
        }
        let dev_type = d.dev_type.as_deref().unwrap_or("");
        let tran = d.tran.as_deref().unwrap_or("");
        let is_usb = dev_type == "disk" && (tran.eq_ignore_ascii_case("usb") || d.rm.unwrap_or(false));
        let is_loop = dev_type == "loop";

        if !is_usb && !(allow_virtual && is_loop) {
            continue;
        }

        let raw_path = PathBuf::from(format!("/dev/{}", d.name));
        drives.push(UsbDrive {
            id: d.name,
            model: d
                .model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    if is_loop {
                        "Loopback Test Disk".to_string()
                    } else {
                        "USB Flash Drive".to_string()
                    }
                }),
            size_bytes: d.size.unwrap_or(0),
            raw_device_path: raw_path,
            bus: if is_loop {
                "VIRTUAL".to_string()
            } else {
                tran.to_uppercase()
            },
            is_system: false,
            is_virtual: is_loop,
        });
    }

    Ok(drives)
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    if drive.is_system {
        anyhow::bail!("Safety lock: Refusing to unmount system drive");
    }

    let output = Command::new("lsblk")
        .args(["-J", "-o", "NAME,MOUNTPOINTS", &drive.raw_device_path.display().to_string()])
        .output();

    if let Ok(out) = output {
        if let Ok(parsed) = serde_json::from_slice::<LsblkOutput>(&out.stdout) {
            let mut to_unmount = Vec::new();
            for dev in &parsed.blockdevices {
                dev.collect_mounted_paths(&mut to_unmount);
            }
            for part in to_unmount {
                let _ = Command::new("umount").arg(&part).status();
            }
        }
    }
    Ok(())
}

pub fn open_raw_device(drive: &UsbDrive) -> Result<File> {
    if drive.is_system {
        anyhow::bail!("Safety lock: Refusing to open system disk for writing");
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} with O_SYNC (Try running with sudo/pkexec)",
                drive.raw_device_path.display()
            )
        })
}

pub fn open_raw_device_read(drive: &UsbDrive) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} for verification read-back",
                drive.raw_device_path.display()
            )
        })
}

pub fn flush_and_sync(file: &mut File) -> Result<()> {
    file.sync_all()
        .context("Failed to flush Linux block device cache")
}
