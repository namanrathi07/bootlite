pub mod platform;

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct UsbDrive {
    pub id: String,
    pub raw_device_path: PathBuf,
    pub model: String,
    pub bus: String,
    pub size_bytes: u64,
    pub is_system: bool,
    pub is_virtual: bool,
}

pub struct FlashProgress {
    pub phase: &'static str,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
}

pub fn list_safe_usb_drives(include_virtual: bool) -> Result<Vec<UsbDrive>> {
    platform::list_drives(include_virtual)
}

pub fn flash_and_verify<F>(
    image_path: &Path,
    target: &UsbDrive,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    if target.is_system {
        bail!("SAFETY ABORT: Target is marked as a system disk!");
    }

    let raw_file = File::open(image_path).context("Failed to open source image")?;
    let file_len = raw_file.metadata()?.len();

    let is_gz = image_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gz"))
        .unwrap_or(false);

    if !is_gz && file_len > target.size_bytes {
        bail!("Image size exceeds target drive capacity!");
    }

    let mut reader: Box<dyn Read> = if is_gz {
        Box::new(GzDecoder::new(raw_file))
    } else {
        Box::new(raw_file)
    };

    let mut dest = platform::prepare_and_open_raw_device(target)?;

    const CHUNK_SIZE: usize = 4 * 1024 * 1024;
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut source_hasher = Sha256::new();
    let mut written: u64 = 0;
    let start_write = Instant::now();

    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                bail!("Operation cancelled by user.");
            }
        }

        let n = read_full_chunk(&mut reader, &mut buffer)?;
        if n == 0 {
            break;
        }

        if written + (n as u64) > target.size_bytes {
            bail!("Decompressed image size exceeds target USB drive capacity!");
        }

        source_hasher.update(&buffer[..n]);

        let write_len = if n % 4096 != 0 {
            let aligned = ((n / 4096) + 1) * 4096;
            buffer[n..aligned].fill(0);
            aligned
        } else {
            n
        };

        dest.write_all(&buffer[..write_len])
            .context("Failed while writing chunk to raw device")?;
        written += n as u64;

        let elapsed = start_write.elapsed().as_secs_f64().max(0.001);
        on_progress(FlashProgress {
            phase: if is_gz { "Decompressing & Writing" } else { "Writing" },
            bytes_processed: written,
            total_bytes: written.max(file_len),
            speed_mbps: (written as f64 / (1024.0 * 1024.0)) / elapsed,
        });
    }

    if let Err(e) = dest.sync_all() {
        if !matches!(e.raw_os_error(), Some(25) | Some(22)) {
            return Err(e).context("Failed to flush device buffers");
        }
    }

    let expected_hash = format!("{:x}", source_hasher.finalize());
    let total_uncompressed_bytes = written;

    // Phase 2: Read-back verification from sector 0
    dest.seek(SeekFrom::Start(0))
        .context("Failed to seek back to start of device for verification")?;
    let mut verify_hasher = Sha256::new();
    let mut verified: u64 = 0;
    let start_verify = Instant::now();

    while verified < total_uncompressed_bytes {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                bail!("Verification cancelled by user.");
            }
        }

        let remaining = (total_uncompressed_bytes - verified) as usize;
        let to_read = remaining.min(CHUNK_SIZE);
        let aligned_read = if to_read % 4096 != 0 {
            ((to_read / 4096) + 1) * 4096
        } else {
            to_read
        };

        dest.read_exact(&mut buffer[..aligned_read])
            .context("Failed while reading back bytes from device")?;
        verify_hasher.update(&buffer[..to_read]);
        verified += to_read as u64;

        let elapsed = start_verify.elapsed().as_secs_f64().max(0.001);
        on_progress(FlashProgress {
            phase: "Verifying",
            bytes_processed: verified,
            total_bytes: total_uncompressed_bytes,
            speed_mbps: (verified as f64 / (1024.0 * 1024.0)) / elapsed,
        });
    }

    let actual_hash = format!("{:x}", verify_hasher.finalize());
    if expected_hash != actual_hash {
        bail!(
            "VERIFICATION FAILED! Expected {}, got {}",
            expected_hash,
            actual_hash
        );
    }

    Ok(expected_hash)
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
