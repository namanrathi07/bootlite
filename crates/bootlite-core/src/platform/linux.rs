use crate::UsbDrive;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;

// Linux ioctl codes (<linux/fs.h>)
const BLKFLSBUF: libc::c_ulong = 0x1261;
const BLKRRPART: libc::c_ulong = 0x125f;

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn get_system_mounted_devs() -> Vec<(String, String)> {
    let mut mounts = Vec::new();
    if let Ok(content) = fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(dev), Some(mnt)) = (parts.next(), parts.next()) {
                if dev.starts_with("/dev/") {
                    mounts.push((dev.to_string(), mnt.to_string()));
                }
            }
        }
    }
    mounts
}

fn is_critical_mountpoint(mnt: &str) -> bool {
    matches!(
        mnt,
        "/" | "/boot" | "/boot/efi" | "/home" | "/usr" | "/var" | "/nix"
    ) || mnt.starts_with("/boot/")
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let mounts = get_system_mounted_devs();
    let mut locked_prefixes = HashSet::new();

    for (dev, mnt) in &mounts {
        if is_critical_mountpoint(mnt) {
            locked_prefixes.insert(dev.clone());
        }
    }

    let mut drives = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return Ok(drives);
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("ram")
            || name.starts_with("zram")
            || name.starts_with("sr")
            || name.starts_with("dm-")
            || name.starts_with("md")
        {
            continue;
        }

        let dev_prefix = format!("/dev/{}", name);
        if locked_prefixes
            .iter()
            .any(|locked| locked == &dev_prefix || locked.starts_with(&dev_prefix))
        {
            continue;
        }

        let sys_path = entry.path();
        if read_trimmed(sys_path.join("ro")).as_deref() == Some("1") {
            continue;
        }

        let sectors = read_trimmed(sys_path.join("size"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let size_bytes = sectors.saturating_mul(512);
        if size_bytes == 0 {
            continue;
        }

        let is_loop = name.starts_with("loop");
        let canonical = fs::canonicalize(&sys_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_usb_bus = canonical.contains("/usb");
        let is_removable = read_trimmed(sys_path.join("removable")).as_deref() == Some("1");
        let is_mmc = name.starts_with("mmcblk") && is_removable;

        let is_usb = is_usb_bus || is_mmc || (is_removable && !is_loop);
        if !is_usb && !(allow_virtual && is_loop) {
            continue;
        }

        let vendor = read_trimmed(sys_path.join("device/vendor")).unwrap_or_default();
        let product = read_trimmed(sys_path.join("device/model")).unwrap_or_default();
        let combined = format!("{} {}", vendor, product).trim().to_string();
        let model = if !combined.is_empty() {
            combined
        } else if is_loop {
            "Loopback Virtual Disk".to_string()
        } else {
            "USB Flash Drive".to_string()
        };

        let bus = if is_loop {
            "VIRTUAL".to_string()
        } else if is_usb_bus {
            "USB".to_string()
        } else {
            "REMOVABLE".to_string()
        };

        drives.push(UsbDrive {
            raw_device_path: PathBuf::from(&dev_prefix),
            id: name,
            model,
            size_bytes,
            bus,
            is_system: false,
            is_virtual: is_loop,
        });
    }

    drives.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(drives)
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    if drive.is_system {
        anyhow::bail!("Safety lock: Refusing to unmount system drive");
    }

    let prefix = format!("/dev/{}", drive.id);
    for (dev, mnt) in get_system_mounted_devs() {
        if dev == prefix || dev.starts_with(&prefix) {
            if is_critical_mountpoint(&mnt) {
                anyhow::bail!("Safety lock: Target drive contains critical mountpoint {}", mnt);
            }
            let status = Command::new("umount").arg(&dev).status();
            if status.map(|s| !s.success()).unwrap_or(true) {
                let _ = Command::new("umount").args(["-l", &dev]).status();
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
        .custom_flags(libc::O_SYNC | libc::O_CLOEXEC)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} with O_SYNC (Click 'Unlock Admin Access' first)",
                drive.raw_device_path.display()
            )
        })
}

pub fn prepare_for_verify(file: &mut File) -> Result<()> {
    file.sync_all()
        .context("Failed to flush Linux block device buffers")?;
    let fd = file.as_raw_fd();
    unsafe {
        // Evict Linux kernel page cache so SHA-256 read-back reads physical USB flash sectors
        let _ = libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_DONTNEED);
        let _ = libc::ioctl(fd, BLKFLSBUF as _, 0);
    }
    Ok(())
}

pub fn finish_device(file: Option<File>) {
    if let Some(f) = file {
        unsafe {
            let _ = libc::ioctl(f.as_raw_fd(), BLKRRPART as _, 0);
        }
    }
}
