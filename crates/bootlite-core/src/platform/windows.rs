use crate::{PartitionScheme, UsbDrive};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;
const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
const FILE_FLAG_WRITE_THROUGH: u32 = 0x80000000;
const CREATE_NO_WINDOW: u32 = 0x08000000;

const FSCTL_LOCK_VOLUME: u32 = 0x00090018;
const FSCTL_DISMOUNT_VOLUME: u32 = 0x00090020;
const FSCTL_ALLOW_EXTENDED_DASD_IO: u32 = 0x00090083;
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;
const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x000700A0;
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
const IOCTL_DISK_DELETE_DRIVE_LAYOUT: u32 = 0x0007C100;
const IOCTL_DISK_UPDATE_PROPERTIES: u32 = 0x00070140;

static LOCKED_VOLUMES: Mutex<Vec<File>> = Mutex::new(Vec::new());

pub fn open_user_uid_init(_uid: u32) {}

pub fn open_user_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(
        h_device: *mut std::ffi::c_void,
        dw_io_control_code: u32,
        lp_in_buffer: *const std::ffi::c_void,
        n_in_buffer_size: u32,
        lp_out_buffer: *mut std::ffi::c_void,
        n_out_buffer_size: u32,
        lp_bytes_returned: *mut u32,
        lp_overlapped: *mut std::ffi::c_void,
    ) -> i32;

    fn GetLogicalDrives() -> u32;
}

#[repr(C)]
struct DiskExtent {
    disk_number: u32,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
struct VolumeDiskExtents {
    number_of_disk_extents: u32,
    extents: [DiskExtent; 1],
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
    additional_parameters: [u8; 4],
}

fn get_volume_disk_number(handle: *mut std::ffi::c_void) -> Option<u32> {
    let mut extents = VolumeDiskExtents {
        number_of_disk_extents: 0,
        extents: [DiskExtent {
            disk_number: 0,
            starting_offset: 0,
            extent_length: 0,
        }],
    };
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            std::ptr::null(),
            0,
            &mut extents as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<VolumeDiskExtents>() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok != 0 && extents.number_of_disk_extents > 0 {
        Some(extents.extents[0].disk_number)
    } else {
        None
    }
}

fn get_system_disk_numbers() -> HashSet<u32> {
    let mut sys = HashSet::new();
    sys.insert(0); // Always protect PhysicalDrive0

    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let clean = sys_drive.trim_end_matches('\\');
    let vol_path = format!(r"\\.\{}", clean);
    if let Ok(f) = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&vol_path)
    {
        if let Some(dnum) = get_volume_disk_number(f.as_raw_handle() as *mut std::ffi::c_void) {
            sys.insert(dnum);
        }
    }
    sys
}

fn query_disk_size(handle: *mut std::ffi::c_void) -> Option<u64> {
    let mut buf = [0u8; 256];
    let mut returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            std::ptr::null(),
            0,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            buf.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok != 0 && returned >= 32 {
        let size = u64::from_le_bytes(buf[24..32].try_into().ok()?);
        if size > 0 {
            return Some(size);
        }
    }
    None
}

fn extract_cstr(buf: &[u8], offset: usize) -> String {
    if offset == 0 || offset >= buf.len() {
        return String::new();
    }
    let slice = &buf[offset..];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).trim().to_string()
}

fn query_storage_descriptor(handle: *mut std::ffi::c_void) -> Option<(u8, bool, String)> {
    let query = StoragePropertyQuery {
        property_id: 0, // StorageDeviceProperty
        query_type: 0,  // PropertyStandardQuery
        additional_parameters: [0; 4],
    };
    let mut buf = [0u8; 1024];
    let mut returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            buf.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || (returned as usize) < 36 {
        return None;
    }

    let removable = buf[11] != 0;
    let vendor_off = u32::from_le_bytes(buf[12..16].try_into().ok()?) as usize;
    let product_off = u32::from_le_bytes(buf[16..20].try_into().ok()?) as usize;
    let bus_type = buf[28];

    let vendor = extract_cstr(&buf[..returned as usize], vendor_off);
    let product = extract_cstr(&buf[..returned as usize], product_off);
    let model = format!("{} {}", vendor, product).trim().to_string();

    Some((
        bus_type,
        removable,
        if model.is_empty() {
            "USB Storage".to_string()
        } else {
            model
        },
    ))
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    let sys_disks = get_system_disk_numbers();
    let mut drives = Vec::new();

    for disk_num in 1..64u32 {
        if sys_disks.contains(&disk_num) {
            continue;
        }
        let dev_path = format!(r"\\.\PhysicalDrive{}", disk_num);
        let Ok(file) = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&dev_path)
        else {
            continue;
        };

        let handle = file.as_raw_handle() as *mut std::ffi::c_void;
        let Some(size_bytes) = query_disk_size(handle) else {
            continue;
        };
        let Some((bus_type, removable, model)) = query_storage_descriptor(handle) else {
            continue;
        };

        // Win32 STORAGE_BUS_TYPE:
        // 7 = BusTypeUsb, 12 = BusTypeSd, 13 = BusTypeMmc, 14 = BusTypeVirtual, 15 = BusTypeFileBackedVirtual
        let is_usb_or_sd = bus_type == 7 || bus_type == 12 || bus_type == 13 || removable;
        let is_virtual = bus_type == 14
            || bus_type == 15
            || model.to_ascii_lowercase().contains("virtual")
            || model.to_ascii_lowercase().contains("msft virtual");

        if is_virtual {
            if !allow_virtual {
                continue;
            }
        } else if !is_usb_or_sd {
            continue;
        }

        let bus_label = if is_virtual {
            "VIRTUAL".to_string()
        } else if bus_type == 12 || bus_type == 13 {
            "SDXC".to_string()
        } else {
            "USB".to_string()
        };

        drives.push(UsbDrive {
            id: format!("PhysicalDrive{}", disk_num),
            model,
            size_bytes,
            raw_device_path: PathBuf::from(dev_path),
            bus: bus_label,
            is_system: false,
            is_virtual,
        });
    }

    drives.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes).then_with(|| a.id.cmp(&b.id)));
    Ok(drives)
}

pub fn is_path_on_drive(path: &Path, drive: &UsbDrive) -> bool {
    let Some(target_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return false;
    };

    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = canonical.to_string_lossy();
    let trimmed = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
        let letter = trimmed.chars().next().unwrap().to_ascii_uppercase();
        let vol_path = format!(r"\\.\{}:", letter);
        if let Ok(f) = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&vol_path)
        {
            if let Some(dnum) = get_volume_disk_number(f.as_raw_handle() as *mut std::ffi::c_void) {
                return dnum == target_num;
            }
        }
    }
    false
}

fn lock_and_dismount_disk_volumes(target_disk: u32) -> Vec<File> {
    let mut locked = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if (mask & (1 << i)) == 0 {
            continue;
        }
        let letter = (b'A' + (i as u8)) as char;
        if letter == 'C' {
            continue;
        }
        let vol_path = format!(r"\\.\{}:", letter);
        let Ok(vol_file) = OpenOptions::new()
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&vol_path)
        else {
            continue;
        };

        let h = vol_file.as_raw_handle() as *mut std::ffi::c_void;
        if get_volume_disk_number(h) != Some(target_disk) {
            continue;
        }

        let mut returned: u32 = 0;
        unsafe {
            let _ = DeviceIoControl(
                h,
                FSCTL_ALLOW_EXTENDED_DASD_IO,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
            for _ in 0..10 {
                let locked_ok = DeviceIoControl(
                    h,
                    FSCTL_LOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                );
                if locked_ok != 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = DeviceIoControl(
                h,
                FSCTL_DISMOUNT_VOLUME,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
        locked.push(vol_file);
    }
    locked
}

pub fn unmount_drive(drive: &UsbDrive) -> Result<()> {
    let Some(target_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };

    if target_num == 0 || drive.is_system || get_system_disk_numbers().contains(&target_num) {
        anyhow::bail!("Safety lock: Refusing to unmount or modify system disk");
    }

    if let Ok(mut guard) = LOCKED_VOLUMES.lock() {
        guard.clear();
    }

    // 1. Lock & dismount all mounted filesystem volumes on this USB disk
    let locked = lock_and_dismount_disk_volumes(target_num);

    // 2. Strip existing partition table layout at kernel level via IOCTL_DISK_DELETE_DRIVE_LAYOUT
    if let Ok(disk_file) = OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&drive.raw_device_path)
    {
        let h = disk_file.as_raw_handle() as *mut std::ffi::c_void;
        let mut returned: u32 = 0;
        unsafe {
            let _ = DeviceIoControl(
                h,
                IOCTL_DISK_DELETE_DRIVE_LAYOUT,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
            let _ = DeviceIoControl(
                h,
                IOCTL_DISK_UPDATE_PROPERTIES,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
    }

    if let Ok(mut guard) = LOCKED_VOLUMES.lock() {
        *guard = locked;
    }
    Ok(())
}

pub fn open_raw_device(drive: &UsbDrive) -> Result<File> {
    let Some(target_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        anyhow::bail!("Invalid Windows PhysicalDrive identifier: {}", drive.id);
    };
    if target_num == 0 || drive.is_system || get_system_disk_numbers().contains(&target_num) {
        anyhow::bail!("Safety lock: Refusing to open system disk for raw writing");
    }

    let file = OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(&drive.raw_device_path)
        .with_context(|| {
            format!(
                "Failed to open {} for raw writing (Click 'Unlock Admin Access' or close programs using the USB drive)",
                drive.raw_device_path.display()
            )
        })?;

    let h = file.as_raw_handle() as *mut std::ffi::c_void;
    let mut returned: u32 = 0;
    unsafe {
        let _ = DeviceIoControl(
            h,
            FSCTL_ALLOW_EXTENDED_DASD_IO,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
    Ok(file)
}

pub fn prepare_for_verify(file: &mut File) -> Result<()> {
    let h = file.as_raw_handle() as *mut std::ffi::c_void;
    let mut returned: u32 = 0;
    unsafe {
        let _ = DeviceIoControl(
            h,
            IOCTL_DISK_UPDATE_PROPERTIES,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
    file.sync_all()
        .context("Failed to flush Windows PhysicalDrive buffers")
}

pub fn finish_device(file: Option<File>) {
    if let Some(f) = file {
        let h = f.as_raw_handle() as *mut std::ffi::c_void;
        let mut returned: u32 = 0;
        unsafe {
            let _ = DeviceIoControl(
                h,
                IOCTL_DISK_UPDATE_PROPERTIES,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
        drop(f);
    }
    if let Ok(mut guard) = LOCKED_VOLUMES.lock() {
        guard.clear();
    }
}

pub fn eject_drive(drive: &UsbDrive) -> Result<()> {
    finish_device(None);
    let Some(target_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return Ok(());
    };
    if target_num == 0 || drive.is_system {
        anyhow::bail!("Safety lock: Refusing to eject system disk");
    }

    let ps = format!(
        "$vols = Get-Partition -DiskNumber {} -ErrorAction SilentlyContinue | Where-Object {{ [char]::IsLetter([char]$_.DriveLetter) }}; \
         $sh = New-Object -ComObject Shell.Application; \
         foreach ($v in $vols) {{ $sh.Namespace(17).ParseName(\"$($v.DriveLetter):\").InvokeVerb('Eject') }}",
        target_num
    );
    let _ = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status();
    Ok(())
}

pub fn format_drive_exfat(drive: &UsbDrive, label: &str, scheme: PartitionScheme) -> Result<()> {
    finish_device(None);
    let Some(disk_num) = drive
        .id
        .strip_prefix("PhysicalDrive")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        anyhow::bail!("Invalid Windows PhysicalDrive ID: {}", drive.id);
    };
    if disk_num == 0 || drive.is_system || get_system_disk_numbers().contains(&disk_num) {
        anyhow::bail!("Safety lock: Refusing to format system disk");
    }

    // Note: After 'clean', diskpart leaves the disk in MBR state.
    // Never run 'convert mbr' after 'clean' because diskpart rejects converting an already-MBR disk.
    let convert_line = match scheme {
        PartitionScheme::Gpt => "convert gpt\r\n",
        PartitionScheme::Auto | PartitionScheme::Mbr => "",
    };

    let script = format!(
        "automount enable\r\n\
         select disk {}\r\n\
         attributes disk clear readonly noerr\r\n\
         online disk noerr\r\n\
         clean\r\n\
         {}\
         create partition primary\r\n\
         format fs=exfat label=\"{}\" quick\r\n\
         assign\r\n\
         exit\r\n",
        disk_num, convert_line, label
    );

    let script_path = std::env::temp_dir().join(format!(
        "bootlite_fmt_{}_{}.txt",
        disk_num,
        std::process::id()
    ));
    std::fs::write(&script_path, script.as_bytes())?;
    let out = Command::new("diskpart")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/s", &script_path.to_string_lossy()])
        .output()
        .context("Failed to execute diskpart for ExFAT format")?;
    let _ = std::fs::remove_file(&script_path);

    if !out.status.success() {
        anyhow::bail!(
            "diskpart ExFAT format failed: {}",
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(())
}
