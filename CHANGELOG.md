# Changelog

All notable changes to **BootLite** are documented in this file.

## [v0.4.1] - 4x Faster Turbo Stream Engine, Fast Verify (~3s) & Windows OS Error 5 Auto-Cleaner
### Added
- **4 MB / 8 MB High-Throughput Burst Engine:** Quadrupled raw streaming block size to 4 MB and Windows ISO file streaming buffer to 8 MB, eliminating per-chunk `FILE_FLAG_WRITE_THROUGH` USB 2.0 bus stalls on Windows.
- **Configurable Verification Modes (`Fast` / `Full` / `Off`):** Added `VerifyMode::Fast` as the default mode (verifies the full 4 MB Sector-0 boot record + 32 evenly spaced 64 KB hardware sector samples + Windows EFI/WIM head/mid/tail integrity in ~3 seconds instead of 20+ minutes on USB 2.0), while retaining `Full` (100% read-back) and `Off`.
- **Automatic Pre-Flash Partition Cleaner on Windows:** Automatically dismounts Explorer/Defender volume handles and runs a silent partition-table strip fallback when `IOCTL_DISK_DELETE_DRIVE_LAYOUT` encounters locked pre-existing partitions, eliminating `Access is denied (os error 5)` without requiring manual USB formatting beforehand.
- **Enhanced Windows 10/11 Insider & UUPDump ISO Detection:** Detects UDF bridge descriptors and Microsoft volume identifiers so custom/Insider Windows 11 ISOs automatically route to the UEFI partition builder.
