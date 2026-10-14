pub mod platform;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB (256 x 4096-byte sectors)
const SECTOR_ALIGN: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbDrive {
    pub id: String,
    pub model: String,
    pub size_bytes: u64,
    pub raw_device_path: PathBuf,
    pub bus: String,
    pub is_system: bool,
    pub is_virtual: bool,
}

#[derive(Debug, Clone)]
pub struct FlashProgress<'a> {
    pub phase: &'a str,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    platform::list_safe_usb_drives(allow_virtual)
}

fn read_full_chunk<R: Read + ?Sized>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

pub fn flash_and_verify<F>(
    image_path: &Path,
    drive: &UsbDrive,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    let img_file = File::open(image_path)
        .with_context(|| format!("Failed to open image file: {}", image_path.display()))?;
    let file_len = img_file.metadata()?.len();

    let is_gz = image_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("gz"))
        .unwrap_or(false);

    if !is_gz && file_len > drive.size_bytes {
        anyhow::bail!(
            "Image size ({} bytes) exceeds target USB drive capacity ({} bytes)",
            file_len,
            drive.size_bytes
        );
    }

    // 1. Unmount & clear existing partition locks on target USB drive
    platform::unmount_drive(drive)?;

    // 2. Open raw hardware device
    let mut writer = platform::open_raw_device(drive)?;

    let mut reader: Box<dyn Read> = if is_gz {
        Box::new(GzDecoder::new(img_file))
    } else {
        Box::new(img_file)
    };

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut source_hasher = Sha256::new();
    let mut total_written: u64 = 0;
    let write_start = Instant::now();

    // Rufus-style Deferred Sector-0 Write:
    // Hold the first 1 MB (containing MBR/GPT partition tables) in memory and write zeros first
    // so Windows Plug-and-Play never auto-mounts the new partition table mid-flash.
    let mut deferred_first_chunk: Option<Vec<u8>> = None;
    let mut is_first_chunk = true;

    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Operation cancelled by user");
            }
        }

        let n = read_full_chunk(&mut reader, &mut buf)
            .context("Failed while reading source image stream")?;
        if n == 0 {
            break;
        }

        if total_written + (n as u64) > drive.size_bytes {
            anyhow::bail!(
                "Decompressed image stream exceeds target USB drive capacity ({} bytes)",
                drive.size_bytes
            );
        }

        source_hasher.update(&buf[..n]);

        if is_first_chunk && n == CHUNK_SIZE {
            is_first_chunk = false;
            deferred_first_chunk = Some(buf[..n].to_vec());
            let zero_sector_block = vec![0u8; CHUNK_SIZE];
            writer
                .write_all(&zero_sector_block)
                .context("Failed while initializing sector 0 on raw device")?;
        } else {
            is_first_chunk = false;
            let write_len = if n % SECTOR_ALIGN != 0 {
                let aligned = (n + SECTOR_ALIGN - 1) & !(SECTOR_ALIGN - 1);
                buf[n..aligned].fill(0);
                aligned
            } else {
                n
            };

            writer
                .write_all(&buf[..write_len])
                .context("Failed while writing chunk to raw device")?;
        }

        total_written += n as u64;
        let elapsed = write_start.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (total_written as f64 / (1024.0 * 1024.0)) / elapsed;
        let est_total = if is_gz {
            total_written.max(file_len)
        } else {
            file_len
        };

        on_progress(FlashProgress {
            phase: "Writing raw image...",
            bytes_processed: total_written,
            total_bytes: est_total,
            speed_mbps,
        });
    }

    // Write the deferred first 1 MB (MBR/GPT boot table) to Sector 0 now that all data sectors are written
    if let Some(first_chunk) = deferred_first_chunk {
        writer
            .seek(SeekFrom::Start(0))
            .context("Failed to seek back to sector 0 for boot record write")?;
        writer
            .write_all(&first_chunk)
            .context("Failed while writing boot record (sector 0) to raw device")?;
    }

    on_progress(FlashProgress {
        phase: "Flushing hardware cache...",
        bytes_processed: total_written,
        total_bytes: total_written,
        speed_mbps: 0.0,
    });

    platform::flush_and_sync(&mut writer)?;
    drop(writer);

    let expected_hash = format!("{:x}", source_hasher.finalize());

    // 3. Sector-by-Sector Read-Back SHA-256 Verification
    let mut verify_reader = platform::open_raw_device_read(drive)?;
    let mut verify_hasher = Sha256::new();
    let mut verified_bytes: u64 = 0;
    let verify_start = Instant::now();

    while verified_bytes < total_written {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Verification cancelled by user");
            }
        }

        let to_read = ((total_written - verified_bytes) as usize).min(CHUNK_SIZE);
        let aligned_read = (to_read + SECTOR_ALIGN - 1) & !(SECTOR_ALIGN - 1);

        let n = read_full_chunk(&mut verify_reader, &mut buf[..aligned_read])
            .context("Failed while reading back from raw device during verification")?;
        if n < to_read {
            anyhow::bail!(
                "Unexpected EOF on raw device during verification (read {} of {} bytes)",
                verified_bytes + (n as u64),
                total_written
            );
        }

        verify_hasher.update(&buf[..to_read]);
        verified_bytes += to_read as u64;

        let elapsed = verify_start.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (verified_bytes as f64 / (1024.0 * 1024.0)) / elapsed;

        on_progress(FlashProgress {
            phase: "Verifying SHA-256...",
            bytes_processed: verified_bytes,
            total_bytes: total_written,
            speed_mbps,
        });
    }

    let actual_hash = format!("{:x}", verify_hasher.finalize());
    if actual_hash != expected_hash {
        anyhow::bail!(
            "SHA-256 verification mismatch! Source: {}, Device: {}",
            expected_hash,
            actual_hash
        );
    }

    Ok(actual_hash)
}
