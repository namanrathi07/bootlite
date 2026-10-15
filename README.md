# ⚡ BootLite (`v0.4.1`)

**Cross-Platform Raw Hardware USB Flasher, Windows 10/11 UEFI Builder & SHA-256 Verifier**

*Built in 100% Native Rust (`bootlite-core`, `bootlite-gui`, `bootlite-cli`) for macOS, Windows, and Linux.*

[![Release](https://img.shields.io/github/v/release/NR3009/bootlite?style=flat-square&color=blue)](https://github.com/NR3009/bootlite/releases/latest)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-10b981?style=flat-square)](#download-pre-built-binaries)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

---

## 🚀 Why BootLite?

Most USB flashers force you to choose: **Rufus** works great on Windows (including Windows 10/11 ISOs) but doesn't exist on macOS or Linux. **balenaEtcher** runs on all 3 OSes, bundles a 150 MB Electron/Chromium runtime, and **cannot create bootable Windows 10/11 USBs**.

**BootLite (`~4 MB` single binary)** combines the best of both into a native GPU-accelerated Rust application:

| Capability | ⚡ BootLite `v0.4.1` | Rufus `v4.x` | balenaEtcher `v2.x` |
| :--- | :---: | :---: | :---: |
| **Native macOS, Windows & Linux GUI + CLI** | ✅ All 3 OSes | ❌ Windows Only | ✅ All 3 OSes |
| **Windows 10 / 11 UDF ISO Bootable USB Builder** | ✅ Auto (FAT32 UEFI + NTFS/ExFAT) | ✅ Yes | ❌ Fails to boot |
| **4 MB / 8 MB High-Throughput Burst Engine** | ✅ Full USB Bus Speed | ✅ Yes | ⚠️ Slow Full Verify |
| **Smart Verification (`Fast ~3s` / `Full 100%` / `Off`)** | ✅ 3 Selectable Modes | ✅ Optional | ❌ Mandatory Slow Read |
| **On-the-Fly `.xz` (Multi-Stream), `.zst`, `.bz2`, `.gz`** | ✅ Built-in Pure Rust | ✅ Yes | ✅ Yes |
| **Partition Scheme Selector (`Auto` / `GPT` / `MBR`)** | ✅ Yes | ✅ Yes | ❌ No |
| **One-Click USB ExFAT Reset / Capacity Recovery** | ✅ Built-in | ✅ Yes | ❌ No |
| **Expected SHA-256 Checksum Verifier** | ✅ Pre-Flash & In-Flight | ✅ Yes | ❌ No |
| **Auto-Eject Hardware on Completion** | ✅ Built-in Toggle | ❌ No | ✅ Yes |
| **Binary Footprint** | **~4 MB Native Rust** | ~1.5 MB C | ~150 MB Electron |

---

## 📦 Download Pre-Built Binaries

Download the latest release directly from [**GitHub Releases (`v0.4.1`)**](https://github.com/NR3009/bootlite/releases/latest):

* 🍎 **macOS (Apple Silicon):** [`BootLite-macOS-AppleSilicon.dmg`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-macOS-AppleSilicon.dmg)
* 💻 **Windows 10 / 11 (x64 Portable — Zero DLL Dependencies):** [`BootLite-Windows-x86_64-Portable.exe`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Windows-x86_64-Portable.exe)
* 🐧 **Linux (x86_64 Tarball — GUI + CLI):** [`BootLite-Linux-x86_64.tar.gz`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Linux-x86_64.tar.gz)
* 🔐 **Release Checksums:** [`SHA256SUMS.txt`](https://github.com/NR3009/bootlite/releases/latest/download/SHA256SUMS.txt)

### 🍎 macOS First-Launch Note
Because `BootLite.app` is open-source and not signed with a paid Apple Developer ID certificate, macOS Gatekeeper may show a prompt on first launch. Run this once after dragging `BootLite.app` to `/Applications`:

```bash
xattr -cr /Applications/BootLite.app
```

---

## 🔍 Verification Modes (`Fast` vs `Full` vs `Off`) & Partition Schemes

### 1. Verification Selector (`Verify:`)
Reading back a 9 GB OS image byte-by-byte over a USB 2.0 port can add 15–20 minutes to a flash job. BootLite lets you choose how hardware verification is performed:

* **`Fast (~3s)` (Default — Recommended):**
  * Flushes the physical USB hardware controller cache (`F_FULLFSYNC` / `FlushFileBuffers` / `BLKFLSBUF`).
  * **For Linux / Raw Images:** Verifies **100% of the 4 MB Sector-0 Boot Record (MBR/GPT)** + **32 evenly spaced 64 KB cryptographic sample blocks** across the drive (including the tail sectors to catch counterfeit/fake-capacity USB sticks).
  * **For Windows 10/11 ISOs:** Verifies all critical UEFI bootloaders (`bootx64.efi`, `bootmgr`, `BCD`, `setup.exe`) and spot-checks **512 KB Head, Middle, and Tail slices** of `sources/boot.wim` and `sources/install.wim`.
* **`Full (100%)`:**
  * Reads back **every single written sector/file (100% of the image)** from physical USB NAND flash and verifies its SHA-256 digest against the source stream. Best for testing older or suspect USB drives.
* **`Off`:**
  * Flushes hardware caches, commits the boot record, and completes immediately after writing.

### 2. Partition Scheme Selector (`Scheme:`)
* **`Auto` (Default):** Preserves built-in `ISOHybrid` (`MBR + GPT`) tables on Linux/BSD images and automatically picks the optimal UEFI partition table when building a Windows 10/11 USB.
* **`GPT`:** Forces a **GUID Partition Table** layout for modern UEFI systems when building a Windows USB or formatting a drive via **Reset / Format USB**.
* **`MBR`:** Forces a **Master Boot Record (BIOS / UEFI-CSM)** layout for legacy motherboards or dual BIOS/UEFI boot compatibility.

---

## 🛡️ Architecture & Hardware Safety Guarantees

1. **System Disk Lockout (3-OS Kernel Protection):**
   * **macOS:** Blocks internal `Apple Fabric` NVMe drives, synthesized APFS containers, and `/dev/disk0`, while supporting external USB drives and built-in SDXC card slots.
   * **Windows:** Queries Win32 `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` and blocks `PhysicalDrive0` and any physical disk hosting `C:`.
   * **Linux:** Inspects `/sys/block` and `/proc/mounts` directly in pure Rust (`under 0.5 ms`) and locks out any device backing `/`, `/boot`, `/boot/efi`, `/home`, `/usr`, `/var`, or `/nix`.
2. **Automatic Windows 10/11 UDF ISO Builder (`windows_iso.rs`):**
   * Automatically detects retail and Insider/UUPDump Microsoft Windows 10/11 ISOs.
   * Streams files using an **8 MB burst buffer**, creating a **Dual-Partition UEFI + Data layout** (`900 MB` `WINBOOT` FAT32 partition for UEFI `boot.wim` + remaining capacity `WINSETUP` NTFS/ExFAT partition for `install.wim`) when `install.wim` exceeds the 4 GB FAT32 file limit.
3. **Auto-Clean Partition Unlock on Windows (Zero `os error 5`):**
   * Automatically dismounts open Explorer/Defender handles, re-locks volumes, and strips stubborn pre-existing partition layouts before opening `PhysicalDriveN` so `Access is denied (os error 5)` never occurs.

---

## 💻 Command-Line Interface (`bootlite-cli`)

```bash
# 1. List connected external USB & SD drives
bootlite-cli list

# 2. Inspect any image or compressed archive (.iso, .img.xz, .iso.zst, .bz2, .dmg)
bootlite-cli inspect --image Win11_24H2_English_x64.iso

# 3. Flash & verify (choose --verify fast, full, or off; --scheme auto, gpt, or mbr)
sudo bootlite-cli flash \
  --image ubuntu-24.04-desktop-amd64.iso \
  --drive disk4 \
  --scheme auto \
  --verify fast \
  --expected-sha256 <sha256-hash> \
  --eject

# 4. Wipe partition tables & restore a USB stick to full-capacity ExFAT
sudo bootlite-cli reset --drive disk4 --label BOOTLITE --scheme auto
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
