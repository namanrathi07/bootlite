use crate::UsbDrive;
use anyhow::{Context, Result};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::PathBuf;
use std::sync::Mutex;

const FILE_FLAG_WRITE_THROUGH: u32 = 0x80000000;
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;

// Win32 Disk, Volume & Storage IOCTL Codes
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x000700A0;
const IOCTL_DISK_DELETE_DRIVE_LAYOUT: u32 = 0x0007C100;
const IOCTL_DISK_UPDATE_PROPERTIES: u32 = 0x00070140;
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;
const FSCTL_LOCK_VOLUME: u32 = 0x00090018;
const FSCTL_DISMOUNT_VOLUME: u32 = 0x00090020;
const FSCTL_ALLOW_EXTENDED_DASD_IO: u32 = 0x00090083;

// Hold locked volume handles open for the entire duration of the flash
// so Windows Explorer / AutoMount cannot re-mount the drive mid-write.
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
    data: [u8; 8],
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

fn volume_belongs_to_disk(drive_letter: char, target_disk_num: u32) -> bool {
    let path = format!(r"\\.\{}:", drive_letter);
    let Ok(file) = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
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

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let mut drives = Vec::new();

    for disk_num in 0..32u32 {
        // Never allow PhysicalDrive0 or the disk hosting C:\
        if disk_num == 0 || volume_belongs_to_disk('C', disk_num) {
            continue;
        }

        let id = format!("PhysicalDrive{}", disk_num);
        let raw_path = PathBuf::from(format!(r"\\.\{}", id));

        // access_mode(0) queries device descriptors directly without requiring Admin privileges
        let Ok(file) = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&raw_path)
        else {
            continue;
        };

        let handle = file.as_raw_handle();

        // 1. Query Storage Descriptor (BusType, RemovableMedia, Vendor/Product Model)
        let query = StoragePropertyQuery {
            property_id: 0, // StorageDeviceProperty
            query_type: 0,  // PropertyStandardQuery
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

        // Win32 STORAGE_BUS_TYPE: 7 = USB, 12 = SD, 13 = MMC, 14 = Virtual, 15 = FileBackedVirtual, 16 = Spaces
        let is_usb = header.bus_type == 7;
        let is_virtual =
            header.bus_type == 14 || header.bus_type == 15 || header.bus_type == 16;

        if !is_usb && !(allow_virtual && is_virtual) {
            continue;
        }

        // 2. Query Exact Disk Capacity via IOCTL_DISK_GET_DRIVE_GEOMETRY_EX
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
        let model = if combined.is_empty() {
            "USB Flash Drive".to_string()
        } else {
            combined
        };

        let bus = if is_usb {
            "USB".to_string()
        } else {
            "VIRTUAL".to_string()
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
    if disk_num == 0 || drive.is_system || volume_belongs_to_disk('C', disk_num) {
        anyhow::bail!("Safety lock: Refusing to unmount system boot drive");
    }

    let mut locked = LOCKED_VOLUMES.lock().unwrap();
    locked.clear();

    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if (mask & (1 << i)) != 0 {
            let letter = (b'A' + (i as u8)) as char;
            if letter == 'C' {
                continue;
            }
            if volume_belongs_to_disk(letter, disk_num) {
                let vol_path = format!(r"\\.\{}:", letter);
                if let Ok(vol_file) = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .open(&vol_path)
                {
                    let h = vol_file.as_raw_handle();
                    let _ = ioctl_none(h, FSCTL_LOCK_VOLUME);
                    let _ = ioctl_none(h, FSCTL_DISMOUNT_VOLUME);
                    // Keep handle open so Windows cannot remount the volume during raw write
                    locked.push(vol_file);
                }
            }
        }
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
                "Failed to open {} for raw write (Click 'Unlock Admin Access' first)",
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
    if let Ok(mut locked) = LOCKED_VOLUMES.lock() {
        locked.clear();
    }
    Ok(())
}
