# ⚡ BootLite (`v0.3.2`)

**Cross-Platform Raw Hardware USB Flasher, Windows 10/11 UEFI Builder & SHA-256 Verifier**  
*Built in 100% Native Rust (`bootlite-core`, `bootlite-gui`, `bootlite-cli`) for macOS, Windows, and Linux.*

[![Release](https://img.shields.io/github/v/release/NR3009/bootlite?style=flat-square&color=blue)](https://github.com/NR3009/bootlite/releases/latest)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-10b981?style=flat-square)](#download-pre-built-binaries)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

---

## 🚀 Why BootLite?

Most USB flashers force you to choose: **Rufus** works great on Windows (including Windows 10/11 ISOs) but doesn't exist on macOS or Linux. **balenaEtcher** runs on all 3 OSes, bundles a 150 MB Electron/Chromium runtime, and **cannot create bootable Windows 10/11 USBs**.

**BootLite (`~4 MB` single binary)** combines the best of both into a native GPU-accelerated Rust application:

| Capability | ⚡ BootLite `v0.3.2` | Rufus `v4.x` | balenaEtcher `v2.x` |
| :--- | :---: | :---: | :---: |
| **Native macOS, Windows & Linux GUI + CLI** | ✅ All 3 OSes | ❌ Windows Only | ✅ All 3 OSes |
| **Windows 10 / 11 UDF ISO Bootable USB Builder** | ✅ Auto (FAT32 UEFI + NTFS/ExFAT) | ✅ Yes | ❌ Fails to boot |
| **Linux ISOHybrid / Raspberry Pi / BSD Raw Flashing** | ✅ Unbuffered 1 MB I/O | ✅ DD Mode | ✅ Yes |
| **On-the-Fly `.xz` (Multi-Stream), `.zst`, `.bz2`, `.gz`** | ✅ Built-in Pure Rust | ✅ Yes | ✅ Yes |
| **Partition Scheme Selector (`Auto` / `GPT` / `MBR`)** | ✅ Yes | ✅ Yes | ❌ No |
| **One-Click USB ExFAT Reset / Capacity Recovery** | ✅ Built-in | ✅ Yes | ❌ No |
| **Full Physical Sector SHA-256 Read-Back Verify** | ✅ Cache-Bypassed | ✅ Optional | ✅ Yes |
| **Expected SHA-256 Checksum Verifier** | ✅ Pre-Flash & In-Flight | ✅ Yes | ❌ No |
| **Binary Footprint** | **~4 MB Native Rust** | ~1.5 MB C | ~150 MB Electron |

---

## 📦 Download Pre-Built Binaries

Download the latest release directly from [**GitHub Releases (`v0.3.2`)**](https://github.com/NR3009/bootlite/releases/latest):

* 🍎 **macOS (Apple Silicon):** [`BootLite-macOS-AppleSilicon.dmg`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-macOS-AppleSilicon.dmg)
* 💻 **Windows 10 / 11 (x64 Portable — Zero DLL Dependencies):** [`BootLite-Windows-x86_64-Portable.exe`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Windows-x86_64-Portable.exe)
* 🐧 **Linux (x86_64 Tarball — GUI + CLI):** [`BootLite-Linux-x86_64.tar.gz`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Linux-x86_64.tar.gz)

### 🍎 macOS First-Launch Note
Because `BootLite.app` is open-source and not signed with a paid Apple Developer ID certificate, macOS Gatekeeper may show a prompt on first launch. Run this once after dragging `BootLite.app` to `/Applications`:

```bash
xattr -cr /Applications/BootLite.app
```

---

## 🛡️ Architecture & Hardware Safety Guarantees

1. **System Disk Lockout (3-OS Kernel Protection):**
   * **macOS:** Blocks internal `Apple Fabric` NVMe drives, synthesized APFS containers, and `/dev/disk0`.
   * **Windows:** Queries Win32 `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` and blocks `PhysicalDrive0` and any physical disk hosting `C:`.
   * **Linux:** Inspects `/sys/block` and `/proc/mounts` directly in pure Rust (`< 0.5 ms`) and locks out any device backing `/`, `/boot`, `/boot/efi`, `/home`, `/usr`, or `/var`.
2. **Automatic Windows 10/11 UDF ISO Builder (`windows_iso.rs`):**
   * Detects non-hybrid Microsoft Windows 10/11 ISOs automatically.
   * If `sources/install.wim` exceeds the 4 GB FAT32 file limit, BootLite creates a **Dual-Partition UEFI + Data layout** (`WINBOOT` FAT32 for UEFI `boot.wim` + `WINSETUP` NTFS/ExFAT for `install.wim`) and verifies every written file back from the USB drive.
3. **Zeroed Sector-0 & Stale Backup GPT Wiper:**
   * Zeroes the last 64 KB of the USB drive before writing to destroy leftover Backup GPT headers from prior flashes.
   * Holds Sector 0 (the first 1 MB containing MBR/GPT partition tables) in RAM during both the **Write** phase and the **99% SHA-256 Read-Back Verification** phase so OS auto-mounters (`mountmgr`, `diskarbitrationd`, `udisks2`) never hijack the drive mid-operation.
4. **True Physical NAND Read-Back Verification:**
   * Bypasses OS RAM page caches (`F_NOCACHE` + `F_FULLFSYNC` on macOS, `FILE_FLAG_WRITE_THROUGH` on Windows, `flock(LOCK_EX)` + `BLKFLSBUF` + `POSIX_FADV_DONTNEED` on Linux) so SHA-256 verification reads real USB sectors.

---

## 💻 Command-Line Interface (`bootlite-cli`)

```bash
# 1. List connected external USB & SD drives
bootlite-cli list

# 2. Inspect any image or compressed archive (.iso, .img.xz, .iso.zst, .dmg)
bootlite-cli inspect --image ubuntu-24.04-desktop-amd64.iso

# 3. Flash & hardware-verify an image (with optional SHA-256 check and auto-eject)
sudo bootlite-cli flash \
  --image ubuntu-24.04-desktop-amd64.iso \
  --drive disk4 \
  --expected-sha256 <sha256-hash> \
  --eject

# 4. Wipe partition tables & restore a USB stick to full-capacity ExFAT
sudo bootlite-cli reset --drive disk4 --label BOOTLITE
```

---

## 🛠️ Building From Source

```bash
git clone https://github.com/NR3009/bootlite.git
cd bootlite

# Launch the Studio GUI
cargo run --release -p bootlite-gui

# Build the macOS .app bundle and .dmg installer
./scripts/bundle-macos.sh
```
