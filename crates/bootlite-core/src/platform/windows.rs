use crate::{PartitionScheme, UsbDrive};
use anyhow::{Context, Result};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const INVALID_HANDLE_VALUE: RawHandle = -1isize as RawHandle;

const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
const IOCTL_STORAGE_EJECT_MEDIA: u32 = 0x002D4808;
const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x000700A0;
const IOCTL_DISK_CREATE_DISK: u32 = 0x0007C058;
const IOCTL_DISK_DELETE_DRIVE_LAYOUT: u32 = 0x0007C100;
const IOCTL_DISK_UPDATE_PROPERTIES: u32 = 0x00070140;
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;
const FSCTL_LOCK_VOLUME: u32 = 0x00090018;
const FSCTL_DISMOUNT_VOLUME: u32 = 0x00090020;
const FSCTL_ALLOW_EXTENDED_DASD_IO: u32 = 0x00090083;

static LOCKED_VOLUMES: Mutex<Vec<File>> = Mutex::new(Vec::new());

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

    fn GetLogicalDrives() -> u32;
    fn FindFirstVolumeW(lpsz_volume_name: *mut u16, cch_buffer_length: u32) -> RawHandle;
    fn FindNextVolumeW(
        h_find_volume: RawHandle,
        lpsz_volume_name: *mut u16,
        cch_buffer_length: u32,
    ) -> i32;
    fn FindVolumeClose(h_find_volume: RawHandle) -> i32;
}

#[repr(C)]
struct CreateDiskRaw {
    partition_style: u32,
    data: [u8; 20],
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
    additional_parameters: [u8; 4],
}

#[repr(C)]
struct StorageDeviceDescriptorHeader {
    version: u32,
    size: u32,
    device_type: u8,
    device_type_modifier: u8,
    removable_media: u8,
    command_queueing: u8,
    vendor_id_offset: u32,
    product_id_offset: u32,
    product_revision_offset: u32,
    serial_number_offset: u32,
    bus_type: u32,
    raw_properties_length: u32,
}

#[repr(C)]
struct DiskGeometry {
    cylinders: i64,
    media_type: u32,
    tracks_per_cylinder: u32,
    sectors_per_track: u32,
    bytes_per_sector: u32,
}

#[repr(C)]
struct DiskGeometryEx {
    geometry: DiskGeometry,
    disk_size: i64,
    data: [u8; 256],
}

#[repr(C)]
struct DiskExtent {
    disk_number: u32,
    padding: u32,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
struct VolumeDiskExtents {
    number_of_disk_extents: u32,
    padding: u32,
    extents: [DiskExtent; 8],
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

fn extract_cstr(buf: &[u8], offset: u32) -> String {
    let off = offset as usize;
    if off == 0 || off >= buf.len() {
        return String::new();
    }
    let slice = &buf[off..];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).trim().to_string()
}

fn volume_path_belongs_to_disk(vol_path: &str, target_disk_num: u32) -> bool {
    let Ok(file) = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(vol_path)
    else {
        return false;
    };

    let mut extents = VolumeDiskExtents {
        number_of_disk_extents: 0,
        padding: 0,
        extents: unsafe { std::mem::zeroed() },
    };
    let mut returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            std::ptr::null(),
            0,
            &mut extents as *mut _ as *mut c_void,
            std::mem::size_of::<VolumeDiskExtents>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        ) != 0
    };

    if !ok {
        return false;
    }

    let count = (extents.number_of_disk_extents as usize).min(extents.extents.len());
    for i in 0..count {
        if extents.extents[i].disk_number == target_disk_num {
            return true;
        }
    }
    false
}

fn enumerate_disk_volume_paths(target_disk_num: u32) -> Vec<String> {
    let mut paths = Vec::new();

    let mut buf = [0u16; 260];
    let find_handle = unsafe { FindFirstVolumeW(buf.as_mut_ptr(), buf.len() as u32) };
    if find_handle != INVALID_HANDLE_VALUE {
        loop {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
            if len > 0 {
                let mut vol_str = String::from_utf16_lossy(&buf[..len]);
                if vol_str.ends_with('\\') {
                    vol_str.pop();
                }
                if volume_path_belongs_to_disk(&vol_str, target_disk_num) {
                    paths.push(vol_str);
                }
            }
            buf.fill(0);
            let next_ok =
                unsafe { FindNextVolumeW(find_handle, buf.as_mut_ptr(), buf.len() as u32) };
            if next_ok == 0 {
                break;
            }
        }
        unsafe {
            let _ = FindVolumeClose(find_handle);
        }
    }

    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if (mask & (1 << i)) != 0 {
            let letter = (b'A' + (i as u8)) as char;
            if letter == 'C' {
                continue;
            }
            let letter_path = format!(r"\\.\{}:", letter);
            if volume_path_belongs_to_disk(&letter_path, target_disk_num)
                && !paths.contains(&letter_path)
            {
                paths.push(letter_path);
            }
        }
    }

    paths
}

fn open_volume_rw(vol_path: &str) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(vol_path)
        .or_else(|_| {
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(vol_path)
        })
        .ok()
}

fn lock_and_dismount_volume(vol_path: &str, out_handles: &mut Vec<File>) {
    let Some(vol_file) = open_volume_rw(vol_path) else {
        return;
    };

    let h = vol_file.as_raw_handle();
    let _ = ioctl_none(h, FSCTL_ALLOW_EXTENDED_DASD_IO);

    let mut locked = false;
    for _ in 0..6 {
        if ioctl_none(h, FSCTL_LOCK_VOLUME) {
            locked = true;
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }

    // Always dismount (forcibly severs open Explorer / Defender / Indexer handles)
    let _ = ioctl_none(h, FSCTL_DISMOUNT_VOLUME);

    if !locked {
        // Re-open fresh handle after dismount and acquire exclusive lock
        drop(vol_file);
        if let Some(fresh_file) = open_volume_rw(vol_path) {
            let fh = fresh_file.as_raw_handle();
            let _ = ioctl_none(fh, FSCTL_ALLOW_EXTENDED_DASD_IO);
            for _ in 0..6 {
                if ioctl_none(fh, FSCTL_LOCK_VOLUME) {
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            let _ = ioctl_none(fh, FSCTL_DISMOUNT_VOLUME);
            out_handles.push(fresh_file);
        }
    } else {
        out_handles.push(vol_file);
    }
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let mut drives = Vec::new();

    for disk_num in 0..32u32 {
        if disk_num == 0 || volume_path_belongs_to_disk(r"\\.\C:", disk_num) {
            continue;
        }

        let id = format!("PhysicalDrive{}", disk_num);
        let raw_path = PathBuf::from(format!(r"\\.\{}", id));

        let Ok(file) = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&raw_path)
        else {
            continue;
        };

        let handle = file.as_raw_handle();

        let query = StoragePropertyQuery {
            property_id: 0,
            query_type: 0,
            additional_parameters: [0; 4],
        };
        let mut desc_buf = [0u8; 1024];
        let mut returned: u32 = 0;
        let ok_desc = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const _ as *const c_void,
                std::mem::size_of::<StoragePropertyQuery>() as u32,
                desc_buf.as_mut_ptr() as *mut c_void,
                desc_buf.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            ) != 0
        };

        if !ok_desc || (returned as usize) < std::mem::size_of::<StorageDeviceDescriptorHeader>() {
            continue;
        }

        let header: StorageDeviceDescriptorHeader =
            unsafe { std::ptr::read_unaligned(desc_buf.as_ptr() as *const _) };

        let is_usb = header.bus_type == 7;
        let is_sd_mmc = header.bus_type == 12 || header.bus_type == 13;
        let is_removable = header.removable_media != 0;
        let is_virtual =
            header.bus_type == 14 || header.bus_type == 15 || header.bus_type == 16;

        if !is_usb && !is_sd_mmc && !is_removable && !(allow_virtual && is_virtual) {
            continue;
        }

        let mut geom: DiskGeometryEx = unsafe { std::mem::zeroed() };
        let ok_geom = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null(),
                0,
                &mut geom as *mut _ as *mut c_void,
                std::mem::size_of::<DiskGeometryEx>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            ) != 0
        };

        if !ok_geom || geom.disk_size <= 0 {
            continue;
        }

        let vendor = extract_cstr(&desc_buf, header.vendor_id_offset);
        let product = extract_cstr(&desc_buf, header.product_id_offset);
        let combined = format!("{} {}", vendor, product).trim().to_string();
        let model = if !combined.is_empty() {
            combined
        } else if is_sd_mmc {
            "SD / MMC Card".to_string()
        } else {
            "USB Flash Drive".to_string()
        };

        let bus = if is_usb {
            "USB".to_string()
        } else if is_sd_mmc {
            "SD/MMC".to_string()
        } else if is_virtual {
            "VIRTUAL".to_string()
        } else {
            "REMOVABLE".to_string()
        };

        drives.push(UsbDrive {
            id,
            model,
            size_bytes: geom.disk_size as u64,
            raw_device_path: raw_path,
            bus,
            is_system: false,
            is_virtual,
        });
    }

    drives.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes).then_with(|| a.id.cmp(&b.id)));
    Ok(drives)
}

fn extract_disk_number(drive: &UsbDrive) -> Option<u32> {
    drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
}

pub fn is_path_on_drive(path: &Path, drive: &UsbDrive) -> bool {
    let Some(disk_num) = extract_disk_number(drive) else {
        return false;
    };
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy();
    let clean = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut chars = clean.chars();
    if let (Some(letter), Some(':')) = (chars.next(), chars.next()) {
        if letter.is_ascii_alphabetic() {
            let vol = format!(r"\\.\{}:", letter.to_ascii_uppercase());
            return volume_path_belongs_to_disk(&vol, disk_num);
        }
    }
    false
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    let Some(disk_num) = extract_disk_number(drive) else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system || volume_path_belongs_to_disk(r"\\.\C:", disk_num) {
        anyhow::bail!("Safety lock: Refusing to unmount system boot drive");
    }

    let mut locked = LOCKED_VOLUMES.lock().unwrap();
    locked.clear();

    for vol_path in enumerate_disk_volume_paths(disk_num) {
        lock_and_dismount_volume(&vol_path, &mut locked);
    }

    Ok(())
}

pub fn open_raw_device(drive: &UsbDrive) -> Result<File> {
    let Some(disk_num) = extract_disk_number(drive) else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to open PhysicalDrive0 for writing");
    }

    let mut last_err = None;
    let mut opened_file = None;

    // Notice: We do NOT pass FILE_FLAG_WRITE_THROUGH here because synchronous SCSI FUA
    // writes per chunk cripple USB 2.0 write throughput down to ~3 MB/s.
    // Instead, we stream 4 MB bursts and call FlushFileBuffers in prepare_for_verify().
    for _ in 0..6 {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&drive.raw_device_path)
        {
            Ok(f) => {
                opened_file = Some(f);
                break;
            }
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let file = opened_file.ok_or_else(|| {
        anyhow::anyhow!(
            "Failed to open {} for raw write ({}): Click 'Unlock Admin Access' first",
            drive.raw_device_path.display(),
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Access denied".to_string())
        )
    })?;

    let handle = file.as_raw_handle();
    let _ = ioctl_none(handle, FSCTL_ALLOW_EXTENDED_DASD_IO);
    let layout_deleted = ioctl_none(handle, IOCTL_DISK_DELETE_DRIVE_LAYOUT);

    let raw_disk = CreateDiskRaw {
        partition_style: 2,
        data: [0u8; 20],
    };
    let mut returned: u32 = 0;
    unsafe {
        let _ = DeviceIoControl(
            handle,
            IOCTL_DISK_CREATE_DISK,
            &raw_disk as *const _ as *const c_void,
            std::mem::size_of::<CreateDiskRaw>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
    let _ = ioctl_none(handle, IOCTL_DISK_UPDATE_PROPERTIES);

    // If Windows kernel refused IOCTL_DISK_DELETE_DRIVE_LAYOUT due to a stubborn pre-existing
    // partition, run a silent 1-second diskpart clean so "Access is denied (os error 5)" never occurs.
    if !layout_deleted {
        drop(file);
        if let Ok(mut locked) = LOCKED_VOLUMES.lock() {
            locked.clear();
        }
        if let Ok(mut dp) = Command::new("diskpart")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = dp.stdin.take() {
                let script = format!(
                    "select disk {}\r\nattributes disk clear readonly\r\nclean\r\nexit\r\n",
                    disk_num
                );
                let _ = stdin.write_all(script.as_bytes());
            }
            let _ = dp.wait();
        }
        let _ = unmount_drive(drive);
        let mut reopened = None;
        for _ in 0..10 {
            if let Ok(f) = OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(&drive.raw_device_path)
            {
                reopened = Some(f);
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
        let reopened = reopened.context("Failed to reopen PhysicalDrive after partition clean")?;
        let _ = ioctl_none(reopened.as_raw_handle(), FSCTL_ALLOW_EXTENDED_DASD_IO);
        return Ok(reopened);
    }

    // Re-lock any remaining volume handles after layout deletion
    let _ = unmount_drive(drive);

    Ok(file)
}

pub fn prepare_for_verify(file: &mut File) -> Result<()> {
    file.sync_all()
        .context("Failed to flush Windows hardware buffers")
}

pub fn finish_device(file: Option<File>) {
    if let Some(f) = file {
        let _ = ioctl_none(f.as_raw_handle(), IOCTL_DISK_UPDATE_PROPERTIES);
        drop(f);
    }
    if let Ok(mut locked) = LOCKED_VOLUMES.lock() {
        locked.clear();
    }
}

pub fn eject_drive(drive: &UsbDrive) -> Result<()> {
    if drive.id == "PhysicalDrive0" || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to eject PhysicalDrive0");
    }
    let _ = unmount_drive(drive);
    if let Ok(mut locked) = LOCKED_VOLUMES.lock() {
        for vol in locked.iter() {
            let _ = ioctl_none(vol.as_raw_handle(), IOCTL_STORAGE_EJECT_MEDIA);
        }
        locked.clear();
    }
    if let Ok(raw) = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&drive.raw_device_path)
    {
        let _ = ioctl_none(raw.as_raw_handle(), IOCTL_STORAGE_EJECT_MEDIA);
    }
    Ok(())
}

pub fn format_drive_exfat(drive: &UsbDrive, label: &str, scheme: PartitionScheme) -> Result<()> {
    let Some(disk_num) = extract_disk_number(drive) else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system || volume_path_belongs_to_disk(r"\\.\C:", disk_num) {
        anyhow::bail!("Safety lock: Refusing to format system boot drive");
    }

    finish_device(None);

    let convert_cmd = match scheme {
        PartitionScheme::Gpt => "convert gpt",
        PartitionScheme::Auto | PartitionScheme::Mbr => "convert mbr",
    };

    let mut child = Command::new("diskpart")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn diskpart for drive formatting")?;

    if let Some(mut stdin) = child.stdin.take() {
        let script = format!(
            "select disk {}\r\nattributes disk clear readonly\r\nclean\r\n{}\r\ncreate partition primary\r\nformat fs=exfat label=\"{}\" quick\r\nassign\r\nexit\r\n",
            disk_num, convert_cmd, label
        );
        let _ = stdin.write_all(script.as_bytes());
    }
    let status = child.wait().context("Failed waiting for diskpart")?;
    if !status.success() {
        anyhow::bail!("diskpart failed while formatting PhysicalDrive{}", disk_num);
    }
    Ok(())
}
