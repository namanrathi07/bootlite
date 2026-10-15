pub mod platform;
pub mod windows_iso;

use anyhow::{Context, Result};
use bzip2_rs::DecoderReader as Bz2Decoder;
use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB (256 x 4096-byte sectors)
const SECTOR_ALIGN: usize = 4096;
const MAX_IO_RETRIES: usize = 4;

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
pub struct ImageMetadata {
    pub file_size: u64,
    pub compression: &'static str,
    pub is_compressed: bool,
    pub is_windows_udf_iso: bool,
    pub scheme: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FlashProgress<'a> {
    pub phase: &'a str,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompressionKind {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl CompressionKind {
    fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "gz" | "gzip" | "tgz" => Self::Gzip,
            "bz2" | "bzip2" | "tbz2" => Self::Bzip2,
            "xz" | "lzma" | "txz" => Self::Xz,
            "zst" | "zstd" => Self::Zstd,
            _ => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "Direct 1 MB Unbuffered Stream",
            Self::Gzip => "On-the-Fly Multi-GZIP Stream",
            Self::Bzip2 => "On-the-Fly BZip2 (.bz2) Stream",
            Self::Xz => "On-the-Fly Multi-Stream XZ / LZMA2",
            Self::Zstd => "On-the-Fly Zstandard (.zst) Stream",
        }
    }
}

struct CountingReader<R> {
    inner: R,
    bytes_read: Arc<AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

struct PipeWriter {
    tx: SyncSender<std::io::Result<Vec<u8>>>,
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.tx.send(Ok(buf.to_vec())).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Decompression pipe closed")
        })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: Receiver<std::io::Result<Vec<u8>>>,
    current: Cursor<Vec<u8>>,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.current.position() < self.current.get_ref().len() as u64 {
            return self.current.read(buf);
        }
        match self.rx.recv() {
            Ok(Ok(chunk)) => {
                self.current = Cursor::new(chunk);
                self.current.read(buf)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(0),
        }
    }
}

struct DeviceCleanupGuard {
    device: Option<File>,
}

impl Drop for DeviceCleanupGuard {
    fn drop(&mut self) {
        platform::finish_device(self.device.take());
    }
}

pub fn list_safe_usb_drives(allow_virtual: bool) -> Result<Vec<UsbDrive>> {
    platform::list_safe_usb_drives(allow_virtual)
}

pub fn is_image_on_drive(image_path: &Path, drive: &UsbDrive) -> bool {
    platform::is_path_on_drive(image_path, drive)
}

pub fn eject_drive(drive: &UsbDrive) -> Result<()> {
    platform::eject_drive(drive)
}

pub fn normalize_sha256_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed
        .strip_prefix("SHA256:")
        .or_else(|| trimmed.strip_prefix("sha256:"))
        .or_else(|| trimmed.strip_prefix("SHA-256:"))
        .or_else(|| trimmed.strip_prefix("sha-256:"))
        .unwrap_or(trimmed)
        .trim();
    let token = cleaned.split_whitespace().next().unwrap_or("");
    Some(token.to_ascii_lowercase())
}

pub fn reset_and_format_drive(drive: &UsbDrive, label: &str) -> Result<()> {
    platform::unmount_drive(drive)?;
    {
        let raw_file = platform::open_raw_device(drive)?;
        let mut guard = DeviceCleanupGuard {
            device: Some(raw_file),
        };
        let device = guard.device.as_mut().unwrap();

        wipe_stale_backup_gpt(device, drive.size_bytes);
        let zeros = vec![0u8; CHUNK_SIZE * 2];
        let _ = device.seek(SeekFrom::Start(0));
        let _ = device.write_all(&zeros);
        let _ = platform::prepare_for_verify(device);
    }
    platform::format_drive_exfat(drive, label)
}

pub fn compute_image_sha256<F>(
    image_path: &Path,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: F,
) -> Result<String>
where
    F: FnMut(FlashProgress),
{
    let file = File::open(image_path)
        .with_context(|| format!("Failed to open image file: {}", image_path.display()))?;
    let total_bytes = file.metadata()?.len().max(1);
    let mut reader = BufReader::with_capacity(CHUNK_SIZE, file);
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut hasher = Sha256::new();
    let mut processed: u64 = 0;
    let start = Instant::now();

    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Hash calculation cancelled by user");
            }
        }
        let n = read_full_chunk(&mut reader, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        processed += n as u64;
        let elapsed = start.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (processed as f64 / (1024.0 * 1024.0)) / elapsed;
        on_progress(FlashProgress {
            phase: "Computing file SHA-256...",
            bytes_processed: processed,
            total_bytes,
            speed_mbps,
        });
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn open_decompressed_stream<R: Read + Send + 'static>(
    reader: R,
    kind: CompressionKind,
) -> Result<Box<dyn Read>> {
    match kind {
        CompressionKind::None => Ok(Box::new(reader)),
        CompressionKind::Gzip => Ok(Box::new(MultiGzDecoder::new(reader))),
        CompressionKind::Bzip2 => Ok(Box::new(Bz2Decoder::new(reader))),
        CompressionKind::Zstd => {
            let dec = ruzstd::StreamingDecoder::new(reader).map_err(|e| {
                anyhow::anyhow!("Failed to initialize Zstandard (.zst) decoder: {}", e)
            })?;
            Ok(Box::new(dec))
        }
        CompressionKind::Xz => {
            let (tx, rx) = mpsc::sync_channel(8);
            thread::spawn(move || {
                let mut buf_reader = BufReader::with_capacity(256 * 1024, reader);
                let mut writer = PipeWriter { tx: tx.clone() };
                let is_lzma_alone = match buf_reader.fill_buf() {
                    Ok(b) if b.len() >= 6 => &b[0..6] != b"\xFD7zXZ\x00",
                    _ => false,
                };

                if is_lzma_alone {
                    if let Err(e) = lzma_rs::lzma_decompress(&mut buf_reader, &mut writer) {
                        let msg = e.to_string();
                        if !msg.contains("decompression pipe closed")
                            && !msg.contains("BrokenPipe")
                        {
                            let _ = tx.send(Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("LZMA stream decompression failed: {}", e),
                            )));
                        }
                    }
                } else {
                    loop {
                        let has_more = loop {
                            match buf_reader.fill_buf() {
                                Ok([]) => break Ok(false),
                                Ok(buf) => {
                                    let pad_zeros = buf.iter().take_while(|&&b| b == 0).count();
                                    if pad_zeros > 0 {
                                        buf_reader.consume(pad_zeros);
                                    } else {
                                        break Ok(true);
                                    }
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                    continue
                                }
                                Err(e) => break Err(e),
                            }
                        };

                        match has_more {
                            Ok(false) => break,
                            Err(e) => {
                                let _ = tx.send(Err(e));
                                break;
                            }
                            Ok(true) => {
                                if let Err(e) =
                                    lzma_rs::xz_decompress(&mut buf_reader, &mut writer)
                                {
                                    let msg = e.to_string();
                                    if !msg.contains("decompression pipe closed")
                                        && !msg.contains("BrokenPipe")
                                    {
                                        let _ = tx.send(Err(std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            format!("XZ stream decompression failed: {}", e),
                                        )));
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            });
            Ok(Box::new(PipeReader {
                rx,
                current: Cursor::new(Vec::new()),
            }))
        }
    }
}

pub fn inspect_image(image_path: &Path) -> Result<ImageMetadata> {
    let mut file = File::open(image_path)
        .with_context(|| format!("Failed to open image: {}", image_path.display()))?;
    let file_size = file.metadata()?.len();
    let kind = CompressionKind::from_path(image_path);

    let mut has_koly_trailer = false;
    if kind == CompressionKind::None && file_size >= 512 {
        let mut trailer = [0u8; 512];
        if file.seek(SeekFrom::End(-512)).is_ok() && file.read_exact(&mut trailer).is_ok() {
            has_koly_trailer = &trailer[0..4] == b"koly";
        }
        let _ = file.seek(SeekFrom::Start(0));
    }

    let mut stream = open_decompressed_stream(file, kind)?;
    let mut header = vec![0u8; 65536];
    let n = read_full_chunk(&mut stream, &mut header).unwrap_or(0);
    drop(stream);

    let has_mbr_sig = n >= 512 && header[510] == 0x55 && header[511] == 0xAA;
    let has_gpt_sig = n >= 520 && &header[512..520] == b"EFI PART";
    let has_apm_sig = n >= 514 && (&header[512..514] == b"PM" || &header[0..2] == b"ER");
    let has_iso9660 = n >= 0x8006
        && (&header[0x8001..0x8006] == b"CD001" || &header[0x8001..0x8006] == b"BEA01");

    let is_windows_udf_iso = has_iso9660 && !has_mbr_sig && kind == CompressionKind::None;

    let (scheme, warning) = if has_iso9660 && has_mbr_sig && has_gpt_sig {
        ("ISOHybrid (MBR + GPT + ISO9660)".to_string(), None)
    } else if has_iso9660 && has_mbr_sig {
        ("ISOHybrid Bootable Image (MBR + ISO9660)".to_string(), None)
    } else if has_gpt_sig {
        ("Bootable GPT Disk Image".to_string(), None)
    } else if has_mbr_sig {
        ("Bootable MBR Disk Image".to_string(), None)
    } else if has_apm_sig {
        ("Apple Partition Map Image".to_string(), None)
    } else if is_windows_udf_iso {
        (
            "Windows 10/11 UDF Installer (Auto-UEFI Builder)".to_string(),
            None,
        )
    } else if has_koly_trailer {
        (
            "Compressed Apple UDIF (.dmg) Container".to_string(),
            Some(
                "This .dmg uses Apple UDIF compression ('koly' trailer) rather than a raw sector layout."
                    .to_string(),
            ),
        )
    } else {
        (
            "Raw Sector Image (No 0x55AA Boot Marker)".to_string(),
            Some(
                "No standard MBR (0x55AA) or GPT partition signature found in Sector 0."
                    .to_string(),
            ),
        )
    };

    Ok(ImageMetadata {
        file_size,
        compression: kind.label(),
        is_compressed: kind != CompressionKind::None,
        is_windows_udf_iso,
        scheme,
        warning,
    })
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

fn write_chunk_with_retry(
    device: &mut File,
    offset: u64,
    data: &[u8],
    cancel_flag: Option<&AtomicBool>,
) -> Result<()> {
    let mut attempt = 0;
    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Operation cancelled by user");
            }
        }

        match device.write_all(data) {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt += 1;
                if attempt > MAX_IO_RETRIES {
                    return Err(e).with_context(|| {
                        format!(
                            "Failed writing to raw device at offset {} after {} retries",
                            offset, MAX_IO_RETRIES
                        )
                    });
                }
                let backoff_ms = 120 * (1u64 << (attempt - 1));
                thread::sleep(Duration::from_millis(backoff_ms));
                let _ = device.seek(SeekFrom::Start(offset));
            }
        }
    }
}

fn read_chunk_with_retry(
    device: &mut File,
    offset: u64,
    buf: &mut [u8],
    cancel_flag: Option<&AtomicBool>,
) -> Result<usize> {
    let mut attempt = 0;
    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                anyhow::bail!("Verification cancelled by user");
            }
        }

        match read_full_chunk(device, buf) {
            Ok(n) => return Ok(n),
            Err(e) => {
                attempt += 1;
                if attempt > MAX_IO_RETRIES {
                    return Err(e).with_context(|| {
                        format!(
                            "Failed reading back from raw device at offset {} after {} retries",
                            offset, MAX_IO_RETRIES
                        )
                    });
                }
                let backoff_ms = 120 * (1u64 << (attempt - 1));
                thread::sleep(Duration::from_millis(backoff_ms));
                let _ = device.seek(SeekFrom::Start(offset));
            }
        }
    }
}

fn wipe_stale_backup_gpt(device: &mut File, drive_size: u64) {
    let wipe_len: u64 = 65536;
    if drive_size > CHUNK_SIZE as u64 * 4 {
        let aligned_end = drive_size & !((SECTOR_ALIGN as u64) - 1);
        if aligned_end > wipe_len {
            let tail_offset = aligned_end - wipe_len;
            let zeros = vec![0u8; wipe_len as usize];
            if device.seek(SeekFrom::Start(tail_offset)).is_ok() {
                let _ = device.write_all(&zeros);
            }
            let _ = device.seek(SeekFrom::Start(0));
        }
    }
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
    if is_image_on_drive(image_path, drive) {
        anyhow::bail!(
            "Safety lock: The selected image file resides on the target USB drive ({}). Copy the image to your internal disk first.",
            drive.id
        );
    }

    if let Ok(meta) = inspect_image(image_path) {
        if meta.is_windows_udf_iso {
            return windows_iso::flash_windows_iso_to_usb(
                image_path,
                drive,
                cancel_flag,
                on_progress,
            );
        }
    }

    let img_file = File::open(image_path)
        .with_context(|| format!("Failed to open image file: {}", image_path.display()))?;
    let file_len = img_file.metadata()?.len().max(1);
    let kind = CompressionKind::from_path(image_path);
    let is_compressed = kind != CompressionKind::None;

    if !is_compressed && file_len > drive.size_bytes {
        anyhow::bail!(
            "Image size ({} bytes) exceeds target USB drive capacity ({} bytes)",
            file_len,
            drive.size_bytes
        );
    }

    platform::unmount_drive(drive)?;

    let raw_file = platform::open_raw_device(drive)?;
    let mut guard = DeviceCleanupGuard {
        device: Some(raw_file),
    };
    let device = guard.device.as_mut().unwrap();

    wipe_stale_backup_gpt(device, drive.size_bytes);

    let compressed_pos = Arc::new(AtomicU64::new(0));
    let counting_file = CountingReader {
        inner: img_file,
        bytes_read: Arc::clone(&compressed_pos),
    };

    let mut reader = open_decompressed_stream(counting_file, kind)?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut full_source_hasher = Sha256::new();
    let mut tail_source_hasher = Sha256::new();
    let mut total_written: u64 = 0;
    let write_start = Instant::now();

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

        full_source_hasher.update(&buf[..n]);

        if is_first_chunk && n == CHUNK_SIZE {
            is_first_chunk = false;
            deferred_first_chunk = Some(buf[..n].to_vec());
            let zero_sector_block = vec![0u8; CHUNK_SIZE];
            write_chunk_with_retry(device, 0, &zero_sector_block, cancel_flag)?;
        } else {
            is_first_chunk = false;
            tail_source_hasher.update(&buf[..n]);

            let write_len = if n % SECTOR_ALIGN != 0 {
                let aligned = (n + SECTOR_ALIGN - 1) & !(SECTOR_ALIGN - 1);
                buf[n..aligned].fill(0);
                aligned
            } else {
                n
            };

            write_chunk_with_retry(device, total_written, &buf[..write_len], cancel_flag)?;
        }

        total_written += n as u64;
        let elapsed = write_start.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (total_written as f64 / (1024.0 * 1024.0)) / elapsed;

        let est_total = if is_compressed {
            let comp_read = compressed_pos.load(Ordering::Relaxed).max(1);
            let ratio = (comp_read as f64 / file_len as f64).clamp(0.01, 1.0);
            ((total_written as f64) / ratio).round() as u64
        } else {
            file_len
        };

        on_progress(FlashProgress {
            phase: "Phase 1/2: Writing raw sectors...",
            bytes_processed: total_written,
            total_bytes: est_total.max(total_written),
            speed_mbps,
        });
    }

    on_progress(FlashProgress {
        phase: "Flushing hardware controller cache...",
        bytes_processed: total_written,
        total_bytes: total_written,
        speed_mbps: 0.0,
    });

    platform::prepare_for_verify(device)?;

    let expected_full_hash = format!("{:x}", full_source_hasher.finalize());
    let expected_tail_hash = format!("{:x}", tail_source_hasher.finalize());

    let verify_start = Instant::now();
    let mut tail_verify_hasher = Sha256::new();
    let mut verified_bytes: u64 = if deferred_first_chunk.is_some() {
        device
            .seek(SeekFrom::Start(CHUNK_SIZE as u64))
            .context("Failed to seek to 1 MB offset for verification")?;
        CHUNK_SIZE as u64
    } else {
        device
            .seek(SeekFrom::Start(0))
            .context("Failed to seek to sector 0 for verification")?;
        0
    };

    while verified_bytes < total_written {
        let to_read = ((total_written - verified_bytes) as usize).min(CHUNK_SIZE);
        let aligned_read = (to_read + SECTOR_ALIGN - 1) & !(SECTOR_ALIGN - 1);

        let n = read_chunk_with_retry(
            device,
            verified_bytes,
            &mut buf[..aligned_read],
            cancel_flag,
        )?;
        if n < to_read {
            anyhow::bail!(
                "Unexpected EOF on raw device during verification (read {} of {} bytes)",
                verified_bytes + (n as u64),
                total_written
            );
        }

        tail_verify_hasher.update(&buf[..to_read]);
        verified_bytes += to_read as u64;

        let elapsed = verify_start.elapsed().as_secs_f64().max(0.001);
        let speed_mbps = (verified_bytes as f64 / (1024.0 * 1024.0)) / elapsed;

        on_progress(FlashProgress {
            phase: "Phase 2/2: Verifying SHA-256...",
            bytes_processed: verified_bytes,
            total_bytes: total_written,
            speed_mbps,
        });
    }

    let actual_tail_hash = format!("{:x}", tail_verify_hasher.finalize());
    if actual_tail_hash != expected_tail_hash {
        anyhow::bail!(
            "SHA-256 verification mismatch on data sectors! Expected: {}, Actual: {}",
            expected_tail_hash,
            actual_tail_hash
        );
    }

    if let Some(first_chunk) = deferred_first_chunk {
        device
            .seek(SeekFrom::Start(0))
            .context("Failed to seek to sector 0 for boot record commit")?;
        write_chunk_with_retry(device, 0, &first_chunk, cancel_flag)?;

        platform::prepare_for_verify(device)?;

        device
            .seek(SeekFrom::Start(0))
            .context("Failed to rewind to sector 0 for boot record verification")?;
        let n = read_chunk_with_retry(device, 0, &mut buf[..CHUNK_SIZE], cancel_flag)?;
        if n != CHUNK_SIZE || buf[..CHUNK_SIZE] != first_chunk[..] {
            anyhow::bail!("Boot record (sector 0) read-back verification mismatch!");
        }
    }

    Ok(expected_full_hash)
}
