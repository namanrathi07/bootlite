# Changelog

All notable changes to **BootLite** are documented in this file.

## [v0.3.3] - Production Hardening & Cross-Platform Windows 10/11 UEFI Builder
### Added
- **Automatic Windows 10/11 UDF ISO Bootable USB Builder (`windows_iso.rs`):** Natively builds UEFI-bootable Windows 10/11 installation USB drives on macOS, Windows, and Linux. Automatically switches between single-partition FAT32 (`install.esd` < 4 GB) and Dual-Partition (`WINBOOT` FAT32 UEFI + `WINSETUP` NTFS/ExFAT for `install.wim` >= 4 GB) with full file read-back verification.
- **Partition Scheme Selector (`Auto` / `GPT` / `MBR`):** Added interactive scheme selector in both the GUI and `bootlite-cli` (`--scheme auto|gpt|mbr`) for Windows ISO builds and USB ExFAT formatting.
- **One-Click USB Reset / Format Utility (`reset_and_format_drive_with_scheme`):** Wipes Primary MBR/GPT + Backup GPT headers and formats the USB drive back to a clean, full-capacity ExFAT volume (`BOOTLITE`).
- **Expected SHA-256 Checksum Verifier:** Pre-flash and in-flight SHA-256 checksum comparison bar in the GUI and `--expected-sha256` flag in `bootlite-cli`.
- **Auto-Eject Hardware on Completion (`eject_drive`):** Optional toggle to safely power down and eject the target USB drive (`diskutil eject`, `IOCTL_STORAGE_EJECT_MEDIA`, `udisksctl power-off`) upon verification success.
- **Multi-Stream `.xz`, `.zst`, `.bz2`, and `.gz` Decompression:** Added concatenated multi-stream XZ (`pixz`/`pxz`) and BZip2 streaming alongside Zstandard and GZIP.
- **Release `SHA256SUMS.txt`:** Automated SHA-256 release asset checksum generation in GitHub Actions CI.

### Fixed & Hardened
- **macOS Dynamic Slice Mount Discovery:** Queries `diskutil info` for exact slice mount points and uses Finder-safe `hdiutil attach -readonly -nobrowse -noverify`.
- **Windows Dynamic Drive Letter Discovery & Pre-Dismount:** Scans `GetLogicalDrives()` for collision-free mount letters and forcibly dismounts open Explorer handles before running `diskpart`.
- **Linux `systemd-udevd` Inhibitor:** Acquires exclusive `flock(LOCK_EX)` on raw block devices during Write and Verify.
- **Hardware I/O Retry Engine & Backup GPT Wiper:** Retries transient USB controller stalls up to 4 times with exponential backoff and zeroes the last 64 KB of the drive before flashing.
