use crate::unattend::{generate_autopilot_cmd_launcher, generate_autopilot_ps1_script};
use crate::{FlashProgress, PartitionScheme, UsbDrive, VerifyMode};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const COPY_CHUNK: usize = 8 * 1024 * 1024; // 8 MB high-throughput streaming buffer
const FAT32_MAX_FILE_SIZE: u64 = 4_000_000_000; // Safe threshold (~3.72 GiB) for FAT32
pub const WIN_ISO_OVERHEAD_BYTES: u64 = 700_000_000; // ~700 MB overhead for UEFI boot partition

/// Retries filesystem operations up to 8 times (350ms apart) so transient Windows MountMgr delays,
/// Defender scans, or USB bus resets (`os error 3` ERROR_PATH_NOT_FOUND / `os error 5` / `os error 32`)
/// never abort a flash operation.
fn with_fs_retry<T, F>(mut op: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    let mut attempt = 0;
    loop {
        match op() {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempt += 1;
                if attempt >= 8 {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(350));
            }
        }
    }
}

fn collect_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = with_fs_retry(|| fs::read_dir(dir))
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = with_fs_retry(|| entry.metadata())?;
        if meta.is_dir() {
            collect_files(&path, out)?;
        } else if meta.is_file() {
            out.push((path, meta.len()));
        }
    }
    Ok(())
}

fn has_large_wim_file(iso_root: &Path) -> Result<bool> {
    let mut files = Vec::new();
    collect_files(iso_root, &mut files)?;
    Ok(files.iter().any(|(_, len)| *len >= FAT32_MAX_FILE_SIZE))
}

fn is_uefi_boot_file(rel_lower: &str) -> bool {
    rel_lower == "bootmgr"
        || rel_lower == "bootmgr.efi"
        || rel_lower.starts_with("boot/")
        || rel_lower.starts_with("efi/")
        || rel_lower == "sources/boot.wim"
}

fn copy_and_hash_file<F>(
    src: &Path,
    dst: &Path,
    file_len: u64,
    cancel_flag: Option<&AtomicBool>,
    buf: &mut [u8],
    stream_hasher: &mut Sha256,
    bytes_done: &mut u64,
    total_bytes: u64,
    start_time: Instant,
    label: &str,
    on_progress: &mut F,
) -> Result<[u8; 32]>
where
    F: FnMut(FlashProgress),
{
    if let Some(parent) = dst.parent() {
        with_fs_retry(|| fs::create_dir_all(parent)).with_context(|| {
            format!(
                "Failed to create USB directory '{}' (verify USB connection)",
                parent.display()
            )
        })?;
    }
    let mut reader = with_fs_retry(|| File::open(src))
        .with_context(|| format!("Failed to open ISO source file: {}", src.display()))?;
    let mut writer = with_fs_retry(|| File::create(dst))
        .with_context(|| format!("Failed to create USB file: {}", dst.display()))?;

    let mut file_hasher = Sha256::new();
    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Windows ISO build cancelled by user");
            }
        }
        let n = reader.read(buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        file_hasher.update(&buf[..n]);
        stream_hasher.update(&buf[..n]);
        *bytes_done += n as u64;

        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (*bytes_done as f64 / (1024.0 * 1024.0)) / elapsed;
        on_progress(FlashProgress {
            phase: label,
            bytes_processed: *bytes_done,
            total_bytes: total_bytes.max(*bytes_done),
            speed_mbps,
        });
    }

    if file_len >= 64 * 1024 * 1024 {
        let _ = writer.sync_all();
    }
    Ok(file_hasher.finalize().into())
}

fn verify_usb_file_readback<F>(
    usb_file: &Path,
    expected_digest: &[u8; 32],
    cancel_flag: Option<&AtomicBool>,
    buf: &mut [u8],
    bytes_verified: &mut u64,
    total_verify_bytes: u64,
    start_time: Instant,
    on_progress: &mut F,
) -> Result<()>
where
    F: FnMut(FlashProgress),
{
    let mut reader = with_fs_retry(|| File::open(usb_file)).with_context(|| {
        format!(
            "Failed to open USB file for verification: {}",
            usb_file.display()
        )
    })?;
    let mut hasher = Sha256::new();

    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Verification cancelled by user");
            }
        }
        let n = reader.read(buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        *bytes_verified += n as u64;

        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (*bytes_verified as f64 / (1024.0 * 1024.0)) / elapsed;
        on_progress(FlashProgress {
            phase: "Phase 2/2: Verifying written Windows USB files...",
            bytes_processed: *bytes_verified,
            total_bytes: total_verify_bytes.max(*bytes_verified),
            speed_mbps,
        });
    }

    let actual: [u8; 32] = hasher.finalize().into();
    if &actual != expected_digest {
        anyhow::bail!(
            "SHA-256 read-back verification failed for USB file: {}",
            usb_file.display()
        );
    }
    Ok(())
}

fn fast_verify_spot_check(src_file: &Path, usb_file: &Path, file_len: u64) -> Result<()> {
    let meta = with_fs_retry(|| fs::metadata(usb_file)).with_context(|| {
        format!(
            "Missing written file on USB drive: {}",
            usb_file.display()
        )
    })?;
    if meta.len() != file_len {
        anyhow::bail!(
            "File size mismatch on USB for {} (expected {}, got {})",
            usb_file.display(),
            file_len,
            meta.len()
        );
    }
    if file_len == 0 {
        return Ok(());
    }

    let sample_size: usize = 512 * 1024;
    let mut src = with_fs_retry(|| File::open(src_file))?;
    let mut dst = with_fs_retry(|| File::open(usb_file))?;
    let mut b1 = vec![0u8; sample_size];
    let mut b2 = vec![0u8; sample_size];

    let offsets = if file_len <= (sample_size as u64) * 2 {
        vec![0u64]
    } else {
        vec![
            0u64,
            file_len / 2,
            file_len.saturating_sub(sample_size as u64),
        ]
    };

    for off in offsets {
        src.seek(SeekFrom::Start(off))?;
        dst.seek(SeekFrom::Start(off))?;
        let to_read = ((file_len - off) as usize).min(sample_size);
        src.read_exact(&mut b1[..to_read])?;
        dst.read_exact(&mut b2[..to_read])?;
        if b1[..to_read] != b2[..to_read] {
            anyhow::bail!(
                "Spot-check verification mismatch in {} at offset {}",
                usb_file.display(),
                off
            );
        }
    }
    Ok(())
}

fn copy_and_verify_windows_tree<F>(
    iso_root: &Path,
    efi_mount: Option<&Path>,
    data_mount: &Path,
    verify_mode: VerifyMode,
    unattend_xml: Option<&str>,
    cancel_flag: Option<&AtomicBool>,
    on_progress: &mut F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    let mut all_files = Vec::new();
    collect_files(iso_root, &mut all_files)?;
    if all_files.is_empty() {
        anyhow::bail!("Mounted Windows ISO appears empty or unreadable");
    }

    let is_dual = efi_mount.is_some();
    let mut total_write_bytes: u64 = 0;
    for (p, len) in &all_files {
        let rel = p
            .strip_prefix(iso_root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if is_dual {
            if is_uefi_boot_file(&rel) {
                total_write_bytes += *len;
            }
            if rel != "sources/boot.wim" {
                total_write_bytes += *len;
            }
        } else {
            total_write_bytes += *len;
        }
    }

    let mut buf = vec![0u8; COPY_CHUNK];
    let mut stream_hasher = Sha256::new();
    let mut written_targets: Vec<(PathBuf, PathBuf, u64, [u8; 32])> = Vec::new();
    let mut bytes_done: u64 = 0;
    let write_start = Instant::now();

    if let Some(efi_dir) = efi_mount {
        for (src_path, file_len) in &all_files {
            let rel_path = src_path.strip_prefix(iso_root).unwrap_or(src_path);
            let rel_lower = rel_path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();

            if is_uefi_boot_file(&rel_lower) {
                let dst_efi = efi_dir.join(rel_path);
                let digest = copy_and_hash_file(
                    src_path,
                    &dst_efi,
                    *file_len,
                    cancel_flag,
                    &mut buf,
                    &mut stream_hasher,
                    &mut bytes_done,
                    total_write_bytes,
                    write_start,
                    "Phase 1/2: Writing UEFI FAT32 Bootloader (boot.wim)...",
                    on_progress,
                )?;
                written_targets.push((src_path.clone(), dst_efi, *file_len, digest));
            }
        }
    }

    for (src_path, file_len) in &all_files {
        let rel_path = src_path.strip_prefix(iso_root).unwrap_or(src_path);
        let rel_lower = rel_path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();

        if is_dual && rel_lower == "sources/boot.wim" {
            continue;
        }

        let dst_data = data_mount.join(rel_path);
        let digest = copy_and_hash_file(
            src_path,
            &dst_data,
            *file_len,
            cancel_flag,
            &mut buf,
            &mut stream_hasher,
            &mut bytes_done,
            total_write_bytes,
            write_start,
            "Phase 1/2: Streaming Windows Setup & install.wim to USB...",
            on_progress,
        )?;
        written_targets.push((src_path.clone(), dst_data, *file_len, digest));
    }

    // Provision the \AutoPilot\ folder and hardware hash extraction scripts strictly on the USB drive
    for root_dir in [efi_mount, Some(data_mount)].into_iter().flatten() {
        let ap_dir = root_dir.join("AutoPilot");
        let _ = with_fs_retry(|| fs::create_dir_all(&ap_dir));
        let _ = with_fs_retry(|| {
            fs::write(
                ap_dir.join("Extract-AutoPilotHash.ps1"),
                generate_autopilot_ps1_script().as_bytes(),
            )
        });
        let _ = with_fs_retry(|| {
            fs::write(
                ap_dir.join("Extract-AutoPilotHash.cmd"),
                generate_autopilot_cmd_launcher().as_bytes(),
            )
        });
    }

    // Write autounattend.xml strictly to the USB root
    if let Some(xml) = unattend_xml {
        if let Some(efi_dir) = efi_mount {
            with_fs_retry(|| fs::write(efi_dir.join("autounattend.xml"), xml.as_bytes()))
                .context("Failed to write autounattend.xml to WINBOOT partition")?;
        }
        with_fs_retry(|| fs::write(data_mount.join("autounattend.xml"), xml.as_bytes()))
            .context("Failed to write autounattend.xml to WINSETUP partition")?;
    }

    match verify_mode {
        VerifyMode::None => {}
        VerifyMode::Fast => {
            on_progress(FlashProgress {
                phase: "Phase 2/2: Fast-Verifying Windows Bootloaders & WIM integrity...",
                bytes_processed: total_write_bytes,
                total_bytes: total_write_bytes,
                speed_mbps: 0.0,
            });
            for (src_path, usb_path, file_len, _) in &written_targets {
                let rel_lower = src_path
                    .strip_prefix(iso_root)
                    .unwrap_or(src_path)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                if *file_len >= 16 * 1024 * 1024
                    || rel_lower.ends_with(".efi")
                    || rel_lower == "bootmgr"
                    || rel_lower == "setup.exe"
                    || rel_lower == "boot/bcd"
                    || rel_lower == "efi/microsoft/boot/bcd"
                {
                    fast_verify_spot_check(src_path, usb_path, *file_len)?;
                }
            }
        }
        VerifyMode::Full => {
            let mut bytes_verified: u64 = 0;
            let verify_start = Instant::now();
            for (_, usb_path, _, expected_digest) in &written_targets {
                verify_usb_file_readback(
                    usb_path,
                    expected_digest,
                    cancel_flag,
                    &mut buf,
                    &mut bytes_verified,
                    total_write_bytes,
                    verify_start,
                    on_progress,
                )?;
            }
        }
    }

    Ok(format!("{:x}", stream_hasher.finalize()))
}

#[cfg(target_os = "macos")]
fn get_macos_slice_mountpoint(slice_dev: &str) -> Option<PathBuf> {
    for attempt in 0..30 {
        if attempt == 10 {
            let _ = Command::new("diskutil").args(["mount", slice_dev]).status();
        }
        if let Ok(out) = Command::new("diskutil").args(["info", slice_dev]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Some(val) = line.trim().strip_prefix("Mount Point:") {
                    let mp = val.trim();
                    if !mp.is_empty() && mp.starts_with('/') {
                        let p = PathBuf::from(mp);
                        if p.is_dir() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

#[cfg(target_os = "macos")]
pub fn flash_windows_iso_to_usb<F>(
    image_path: &Path,
    drive: &UsbDrive,
    _scheme: PartitionScheme,
    verify_mode: VerifyMode,
    unattend_xml: Option<&str>,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    if drive.bus.eq_ignore_ascii_case("Apple Fabric") || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to format internal system disk");
    }

    let iso_size = fs::metadata(image_path)?.len();
    if drive.size_bytes < iso_size + WIN_ISO_OVERHEAD_BYTES {
        anyhow::bail!(
            "Target USB drive ({} bytes) is too small for Windows Setup ISO ({} bytes + 700 MB boot overhead)",
            drive.size_bytes,
            iso_size
        );
    }

    on_progress(FlashProgress {
        phase: "Windows ISO: Mounting UDF optical image...",
        bytes_processed: 0,
        total_bytes: 100,
        speed_mbps: 0.0,
    });

    let attach_out = Command::new("hdiutil")
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noverify",
            &image_path.to_string_lossy(),
        ])
        .output()
        .context("Failed to execute hdiutil attach on Windows ISO")?;

    if !attach_out.status.success() {
        anyhow::bail!(
            "hdiutil failed to mount Windows ISO: {}",
            String::from_utf8_lossy(&attach_out.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&attach_out.stdout);
    let mut iso_mount_opt = None;
    let mut attached_dev = None;
    for line in stdout.lines() {
        if let Some(dev) = line.split_whitespace().next() {
            if dev.starts_with("/dev/disk") && attached_dev.is_none() {
                attached_dev = Some(dev.to_string());
            }
        }
        if let Some(idx) = line.find("/Volumes/") {
            iso_mount_opt = Some(PathBuf::from(line[idx..].trim()));
        } else if let Some(idx) = line.find("/private/tmp/") {
            iso_mount_opt = Some(PathBuf::from(line[idx..].trim()));
        }
    }

    let iso_mount = iso_mount_opt
        .ok_or_else(|| anyhow::anyhow!("Could not locate mounted volume path from hdiutil"))?;

    struct IsoUnmountGuard(Option<String>);
    impl Drop for IsoUnmountGuard {
        fn drop(&mut self) {
            if let Some(dev) = &self.0 {
                let _ = Command::new("hdiutil")
                    .args(["detach", "-force", dev])
                    .status();
            }
        }
    }
    let _iso_guard = IsoUnmountGuard(attached_dev);

    let needs_dual = has_large_wim_file(&iso_mount)?;
    let disk_node = format!("/dev/{}", drive.id);

    let stream_hash = if needs_dual {
        on_progress(FlashProgress {
            phase: "Windows ISO: Formatting USB (FAT32 UEFI + ExFAT Setup)...",
            bytes_processed: 5,
            total_bytes: 100,
            speed_mbps: 0.0,
        });

        let part_status = Command::new("diskutil")
            .args([
                "partitionDisk",
                &disk_node,
                "MBRFormat",
                "MS-DOS",
                "WINBOOT",
                "900M",
                "ExFAT",
                "WINSETUP",
                "R",
            ])
            .status()
            .context("Failed to partition USB drive for Windows Setup")?;

        if !part_status.success() {
            anyhow::bail!("diskutil partitionDisk failed while creating Windows UEFI layout");
        }

        let s1 = format!("/dev/{}s1", drive.id);
        let s2 = format!("/dev/{}s2", drive.id);
        let efi_mount = get_macos_slice_mountpoint(&s1)
            .ok_or_else(|| anyhow::anyhow!("Failed to locate mount point for {}", s1))?;
        let data_mount = get_macos_slice_mountpoint(&s2)
            .ok_or_else(|| anyhow::anyhow!("Failed to locate mount point for {}", s2))?;

        copy_and_verify_windows_tree(
            &iso_mount,
            Some(&efi_mount),
            &data_mount,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    } else {
        on_progress(FlashProgress {
            phase: "Windows ISO: Formatting USB as Single FAT32 (WINSETUP)...",
            bytes_processed: 5,
            total_bytes: 100,
            speed_mbps: 0.0,
        });

        let part_status = Command::new("diskutil")
            .args(["eraseDisk", "MS-DOS", "WINSETUP", "MBRFormat", &disk_node])
            .status()
            .context("Failed to format USB drive as FAT32")?;

        if !part_status.success() {
            anyhow::bail!("diskutil eraseDisk failed while creating FAT32 layout");
        }

        let s1 = format!("/dev/{}s1", drive.id);
        let data_mount = get_macos_slice_mountpoint(&s1)
            .ok_or_else(|| anyhow::anyhow!("Failed to locate mount point for {}", s1))?;

        copy_and_verify_windows_tree(
            &iso_mount,
            None,
            &data_mount,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    };

    let _ = Command::new("sync").status();
    let _ = Command::new("diskutil")
        .args(["unmountDisk", &disk_node])
        .status();

    Ok(stream_hash)
}

#[cfg(target_os = "windows")]
fn find_two_free_drive_letters() -> Result<(char, char)> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
    }
    let mask = unsafe { GetLogicalDrives() };
    let mut free = Vec::new();
    // Search downward from Z: to G: to avoid low drive letters reserved by card readers
    for i in (6..26u32).rev() {
        if (mask & (1 << i)) == 0 {
            free.push((b'A' + (i as u8)) as char);
            if free.len() == 2 {
                return Ok((free[0], free[1]));
            }
        }
    }
    anyhow::bail!(
        "No two free drive letters (G:..Z:) available on Windows to mount USB partitions"
    )
}

/// Waits for a newly formatted Windows USB partition (`expected_letter:\`) to come online in MountMgr.
/// If `diskpart` delayed or skipped the drive letter assignment (which causes `os error 3` ERROR_PATH_NOT_FOUND),
/// this function queries the partition by volume label (`WINBOOT` / `WINSETUP`) or disk number in PowerShell
/// and forces the drive letter mount before returning the verified root `PathBuf`.
#[cfg(target_os = "windows")]
fn wait_for_windows_usb_mount(
    disk_num: u32,
    vol_label: &str,
    expected_letter: char,
) -> Result<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let expected_root = PathBuf::from(format!(r"{}:\", expected_letter));
    for attempt in 0..24 {
        if expected_root.is_dir() {
            return Ok(expected_root);
        }
        if attempt == 4 || attempt == 12 {
            // Query Windows Storage / Volume Manager to see if the partition mounted under a different letter
            // or needs an explicit drive letter assignment
            let ps_fix = format!(
                "$vol = Get-Volume -FileSystemLabel '{}' -ErrorAction SilentlyContinue | Select-Object -First 1; \
                 if ($vol -and $vol.DriveLetter) {{ Write-Output $vol.DriveLetter }} \
                 else {{ \
                   $part = Get-Partition -DiskNumber {} -ErrorAction SilentlyContinue | Where-Object {{ $_.DriveLetter -eq $null -and $_.Size -gt 100MB }} | Select-Object -First 1; \
                   if ($part) {{ Set-Partition -InputObject $part -NewDriveLetter '{}' -ErrorAction SilentlyContinue; Write-Output '{}' }} \
                 }}",
                vol_label, disk_num, expected_letter, expected_letter
            );
            if let Ok(out) = Command::new("powershell")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["-NoProfile", "-Command", &ps_fix])
                .output()
            {
                if let Some(actual_char) = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .chars()
                    .find(|c| c.is_ascii_alphabetic())
                {
                    let alt_root = PathBuf::from(format!(r"{}:\", actual_char));
                    if alt_root.is_dir() {
                        return Ok(alt_root);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(350));
    }

    anyhow::bail!(
        "USB partition '{}' ({}:\\) did not come online after formatting. Please unplug and reconnect the USB drive.",
        vol_label,
        expected_letter
    )
}

#[cfg(target_os = "windows")]
pub fn flash_windows_iso_to_usb<F>(
    image_path: &Path,
    drive: &UsbDrive,
    scheme: PartitionScheme,
    verify_mode: VerifyMode,
    unattend_xml: Option<&str>,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let Some(disk_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        anyhow::bail!("Invalid Windows PhysicalDrive ID: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to format PhysicalDrive0");
    }

    let iso_size = fs::metadata(image_path)?.len();
    if drive.size_bytes < iso_size + WIN_ISO_OVERHEAD_BYTES {
        anyhow::bail!(
            "Target USB drive ({} bytes) is too small for Windows Setup ISO ({} bytes + 700 MB boot overhead)",
            drive.size_bytes,
            iso_size
        );
    }

    on_progress(FlashProgress {
        phase: "Windows ISO: Mounting source ISO & waiting for UDF filesystem...",
        bytes_processed: 0,
        total_bytes: 100,
        speed_mbps: 0.0,
    });

    let _ = crate::platform::unmount_drive(drive);
    crate::platform::finish_device(None);

    let iso_str = image_path.to_string_lossy().replace('\'', "''");
    // Mount ISO and poll until the UDF drive letter is live and readable (prevents os error 3 on source ISO)
    let mount_ps = format!(
        "$img = Mount-DiskImage -ImagePath '{}' -PassThru -ErrorAction Stop; \
         for ($i = 0; $i -lt 20; $i++) {{ \
             $dl = (Get-DiskImage -ImagePath '{}' | Get-Volume -ErrorAction SilentlyContinue | Where-Object {{ $_.DriveLetter }} | Select-Object -First 1).DriveLetter; \
             if ($dl -and (Test-Path \"$($dl):\\\")) {{ Write-Output $dl; break }}; \
             Start-Sleep -Milliseconds 300 \
         }}",
        iso_str, iso_str
    );
    let out = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-Command", &mount_ps])
        .output()
        .context("Failed to mount Windows ISO")?;

    let iso_letter = String::from_utf8_lossy(&out.stdout)
        .trim()
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .ok_or_else(|| anyhow::anyhow!("Failed to obtain mounted Windows ISO drive letter"))?;

    struct WinIsoGuard(String);
    impl Drop for WinIsoGuard {
        fn drop(&mut self) {
            let cmd = format!(
                "Dismount-DiskImage -ImagePath '{}' -ErrorAction SilentlyContinue",
                self.0
            );
            let _ = Command::new("powershell")
                .creation_flags(0x08000000)
                .args(["-NoProfile", "-Command", &cmd])
                .status();
        }
    }
    let _guard = WinIsoGuard(iso_str);

    let iso_root = PathBuf::from(format!(r"{}:\", iso_letter));
    let needs_dual = has_large_wim_file(&iso_root)?;
    let (efi_letter, data_letter) = find_two_free_drive_letters()?;

    // Stage 1: Clean stale partition tables in a dedicated diskpart pass first so VDS refreshes cleanly
    on_progress(FlashProgress {
        phase: "Windows ISO: Cleaning previous USB partition table...",
        bytes_processed: 2,
        total_bytes: 100,
        speed_mbps: 0.0,
    });
    if let Ok(mut clean_dp) = Command::new("diskpart")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = clean_dp.stdin.take() {
            let clean_script = format!(
                "select disk {}\r\nattributes disk clear readonly\r\nclean\r\nrescan\r\nexit\r\n",
                disk_num
            );
            let _ = stdin.write_all(clean_script.as_bytes());
        }
        let _ = clean_dp.wait();
    }
    std::thread::sleep(Duration::from_millis(600));

    let convert_cmd = match scheme {
        PartitionScheme::Mbr => "convert mbr",
        PartitionScheme::Auto | PartitionScheme::Gpt => "convert gpt",
    };
    let active_cmd = if matches!(scheme, PartitionScheme::Mbr) {
        "active\r\n"
    } else {
        ""
    };

    let stream_hash = if needs_dual {
        on_progress(FlashProgress {
            phase: "Windows ISO: Partitioning USB (FAT32 UEFI + NTFS Setup)...",
            bytes_processed: 4,
            total_bytes: 100,
            speed_mbps: 0.0,
        });

        let mut dp = Command::new("diskpart")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to start diskpart for Windows USB creation")?;

        if let Some(mut stdin) = dp.stdin.take() {
            let script = format!(
                "automount enable\r\n\
                 select disk {}\r\n\
                 {}\r\n\
                 create partition primary size=900\r\n\
                 format fs=fat32 label=\"WINBOOT\" quick\r\n\
                 {}\
                 assign letter={}\r\n\
                 create partition primary\r\n\
                 format fs=ntfs label=\"WINSETUP\" quick\r\n\
                 assign letter={}\r\n\
                 exit\r\n",
                disk_num, convert_cmd, active_cmd, efi_letter, data_letter
            );
            let _ = stdin.write_all(script.as_bytes());
        }
        let status = dp.wait()?;
        if !status.success() {
            anyhow::bail!("diskpart failed to create FAT32 + NTFS Windows boot layout");
        }

        // Wait & verify both USB partitions are mounted in Win32 namespace before copying files
        let efi_mount = wait_for_windows_usb_mount(disk_num, "WINBOOT", efi_letter)?;
        let data_mount = wait_for_windows_usb_mount(disk_num, "WINSETUP", data_letter)?;

        copy_and_verify_windows_tree(
            &iso_root,
            Some(&efi_mount),
            &data_mount,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    } else {
        on_progress(FlashProgress {
            phase: "Windows ISO: Formatting USB as FAT32 (WINSETUP)...",
            bytes_processed: 4,
            total_bytes: 100,
            speed_mbps: 0.0,
        });

        let create_part_cmd = if drive.size_bytes > 32 * 1024 * 1024 * 1024 {
            "create partition primary size=32000"
        } else {
            "create partition primary"
        };

        let mut dp = Command::new("diskpart")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to start diskpart for FAT32 Windows USB creation")?;

        if let Some(mut stdin) = dp.stdin.take() {
            let script = format!(
                "automount enable\r\n\
                 select disk {}\r\n\
                 {}\r\n\
                 {}\r\n\
                 format fs=fat32 label=\"WINSETUP\" quick\r\n\
                 {}\
                 assign letter={}\r\n\
                 exit\r\n",
                disk_num, convert_cmd, create_part_cmd, active_cmd, data_letter
            );
            let _ = stdin.write_all(script.as_bytes());
        }
        let status = dp.wait()?;
        if !status.success() {
            anyhow::bail!("diskpart failed to create FAT32 Windows boot layout");
        }

        let data_mount = wait_for_windows_usb_mount(disk_num, "WINSETUP", data_letter)?;
        copy_and_verify_windows_tree(
            &iso_root,
            None,
            &data_mount,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    };

    Ok(stream_hash)
}

#[cfg(target_os = "linux")]
pub fn flash_windows_iso_to_usb<F>(
    image_path: &Path,
    drive: &UsbDrive,
    scheme: PartitionScheme,
    verify_mode: VerifyMode,
    unattend_xml: Option<&str>,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    if drive.is_system {
        anyhow::bail!("Safety lock: Refusing to format system disk");
    }

    let iso_size = fs::metadata(image_path)?.len();
    if drive.size_bytes < iso_size + WIN_ISO_OVERHEAD_BYTES {
        anyhow::bail!(
            "Target USB drive ({} bytes) is too small for Windows Setup ISO ({} bytes + 700 MB boot overhead)",
            drive.size_bytes,
            iso_size
        );
    }

    crate::platform::unmount_drive(drive)?;

    let pid = std::process::id();
    let iso_mnt = PathBuf::from(format!("/tmp/bootlite_iso_{}", pid));
    let efi_mnt = PathBuf::from(format!("/tmp/bootlite_efi_{}", pid));
    let data_mnt = PathBuf::from(format!("/tmp/bootlite_data_{}", pid));
    let _ = fs::create_dir_all(&iso_mnt);
    let _ = fs::create_dir_all(&efi_mnt);
    let _ = fs::create_dir_all(&data_mnt);

    struct LinuxMountGuard(Vec<PathBuf>);
    impl Drop for LinuxMountGuard {
        fn drop(&mut self) {
            let _ = Command::new("sync").status();
            for m in &self.0 {
                let _ = Command::new("umount").arg(m).status();
                let _ = fs::remove_dir_all(m);
            }
        }
    }
    let _guard = LinuxMountGuard(vec![iso_mnt.clone(), efi_mnt.clone(), data_mnt.clone()]);

    let m1 = Command::new("mount")
        .args([
            "-o",
            "loop,ro",
            &image_path.to_string_lossy(),
            &iso_mnt.to_string_lossy(),
        ])
        .status()
        .context("Failed to loop-mount Windows ISO on Linux")?;
    if !m1.success() {
        anyhow::bail!("Failed to mount Windows ISO image via loop device");
    }

    let needs_dual = has_large_wim_file(&iso_mnt)?;
    let dev = format!("/dev/{}", drive.id);
    let use_gpt = matches!(scheme, PartitionScheme::Gpt);
    let label_type = if use_gpt { "gpt" } else { "msdos" };
    let boot_flag = if use_gpt { "esp" } else { "boot" };

    on_progress(FlashProgress {
        phase: "Windows ISO: Partitioning USB (FAT32 EFI + NTFS/ExFAT)...",
        bytes_processed: 0,
        total_bytes: 100,
        speed_mbps: 0.0,
    });

    let stream_hash = if needs_dual {
        let status = Command::new("parted")
            .args([
                "-s",
                &dev,
                "mklabel",
                label_type,
                "mkpart",
                "primary",
                "fat32",
                "1MiB",
                "901MiB",
                "set",
                "1",
                boot_flag,
                "on",
                "mkpart",
                "primary",
                "ntfs",
                "901MiB",
                "100%",
            ])
            .status()
            .context("Failed to run parted for Windows USB layout")?;

        if !status.success() {
            anyhow::bail!("parted failed while creating partition layout on {}", dev);
        }

        let _ = Command::new("partprobe").arg(&dev).status();
        std::thread::sleep(Duration::from_millis(800));

        let (p1, p2) = if drive.id.starts_with("mmcblk") || drive.id.starts_with("loop") {
            (
                format!("/dev/{}p1", drive.id),
                format!("/dev/{}p2", drive.id),
            )
        } else {
            (format!("/dev/{}1", drive.id), format!("/dev/{}2", drive.id))
        };

        let _ = Command::new("mkfs.vfat")
            .args(["-F", "32", "-n", "WINBOOT", &p1])
            .status();

        let ntfs_ok = Command::new("mkfs.ntfs")
            .args(["-Q", "-L", "WINSETUP", &p2])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ntfs_ok {
            let exfat_ok = Command::new("mkfs.exfat")
                .args(["-n", "WINSETUP", &p2])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !exfat_ok {
                anyhow::bail!(
                    "Failed to format {} — please install ntfs-3g or exfatprogs",
                    p2
                );
            }
        }

        let m2 = Command::new("mount")
            .args([p1.as_str(), &efi_mnt.to_string_lossy()])
            .status()?;
        let m3 = Command::new("mount")
            .args([p2.as_str(), &data_mnt.to_string_lossy()])
            .status()?;

        if !m2.success() || !m3.success() {
            anyhow::bail!("Failed to mount target USB partitions on Linux");
        }

        copy_and_verify_windows_tree(
            &iso_mnt,
            Some(&efi_mnt),
            &data_mnt,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    } else {
        let status = Command::new("parted")
            .args([
                "-s",
                &dev,
                "mklabel",
                label_type,
                "mkpart",
                "primary",
                "fat32",
                "1MiB",
                "100%",
                "set",
                "1",
                boot_flag,
                "on",
            ])
            .status()
            .context("Failed to run parted for FAT32 Windows USB layout")?;

        if !status.success() {
            anyhow::bail!("parted failed while creating FAT32 layout on {}", dev);
        }

        let _ = Command::new("partprobe").arg(&dev).status();
        std::thread::sleep(Duration::from_millis(800));

        let p1 = if drive.id.starts_with("mmcblk") || drive.id.starts_with("loop") {
            format!("/dev/{}p1", drive.id)
        } else {
            format!("/dev/{}1", drive.id)
        };

        let _ = Command::new("mkfs.vfat")
            .args(["-F", "32", "-n", "WINSETUP", &p1])
            .status();

        let m3 = Command::new("mount")
            .args([p1.as_str(), &data_mnt.to_string_lossy()])
            .status()?;
        if !m3.success() {
            anyhow::bail!("Failed to mount FAT32 USB partition on Linux");
        }

        copy_and_verify_windows_tree(
            &iso_mnt,
            None,
            &data_mnt,
            verify_mode,
            unattend_xml,
            cancel_flag,
            &mut on_progress,
        )?
    };

    Ok(stream_hash)
}
