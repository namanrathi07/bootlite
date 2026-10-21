# ⚡ BootLite (`v0.6.1`)

**Cross-Platform Raw Hardware USB Flasher, Windows 10/11 Unattended & AutoPilot Studio, and SHA-256 Verifier**

*Built in 100% Native Rust (`bootlite-core`, `bootlite-gui`, `bootlite-cli`) for macOS, Windows, and Linux.*

[![Release](https://img.shields.io/badge/release-v0.6.1-3b82f6?style=flat-square)](https://github.com/NR3009/bootlite/releases/latest)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-10b981?style=flat-square)](#download-pre-built-binaries)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

---

## 🚀 How BootLite Compares to Other Popular Tools

Most USB utilities force a compromise: **Rufus** is fast and supports Windows ISOs, but is Windows-only. **balenaEtcher** runs across OSes, but bundles a ~150 MB Electron runtime, cannot build bootable Windows 10/11 USBs, and forces a slow 100% read-back pass. **Ventoy** uses a custom bootloader shim that can trigger Secure Boot blocks on locked enterprise laptops.

**BootLite (`~4 MB` single binary)** combines native cross-platform raw flashing, clean UEFI Windows 10/11 media creation, unattended `autounattend.xml` customization, and automated **Windows AutoPilot Hardware Hash extraction** in a single native Rust studio:

| Capability | ⚡ BootLite `v0.6.1` | Rufus `v4.x` | balenaEtcher `v2.x` | Ventoy `v1.x` |
| :--- | :---: | :---: | :---: | :---: |
| **Native macOS, Windows & Linux GUI + CLI** | ✅ All 3 OSes | ❌ Windows Only | ✅ All 3 OSes | ⚠️ Windows / Linux |
| **Windows 10 / 11 UDF ISO Bootable USB Builder** | ✅ Auto (FAT32 UEFI + NTFS/ExFAT) | ✅ Yes | ❌ Fails to boot | ✅ Multi-ISO Shim |
| **Windows AutoPilot Hardware Hash Extractor (`\AutoPilot\`)** | ✅ Built-in (Saves Intune CSV to USB) | ❌ No | ❌ No | ❌ No |
| **Windows 10 / 11 Unattended Setup Studio (`autounattend.xml`)** | ✅ Basic, De-Bloat & Live XML Preview | ✅ Basic Dialog | ❌ No | ⚠️ Manual Plugin |
| **Standard Native UEFI Boot (No Custom Shim / MOK Required)** | ✅ 100% Native Boot | ✅ Yes | ✅ Yes | ❌ Custom Bootloader |
| **4 MB / 8 MB High-Throughput Burst Engine** | ✅ Full USB Bus Speed | ✅ Yes | ⚠️ Slow Full Verify | N/A (File Copy) |
| **Smart Verification (`Fast ~3s` / `Full 100%` / `Off`)** | ✅ 3 Selectable Modes | ✅ Optional | ❌ Mandatory Slow Read | ❌ No |
| **On-the-Fly `.xz` (Multi-Stream), `.zst`, `.bz2`, `.gz`** | ✅ Built-in Pure Rust | ✅ Yes | ✅ Yes | ❌ Uncompressed ISO |
| **Partition Scheme Selector (`Auto` / `GPT` / `MBR`)** | ✅ Yes | ✅ Yes | ❌ No | ✅ Yes |
| **One-Click USB ExFAT Reset / Capacity Recovery** | ✅ Built-in | ✅ Yes | ❌ No | ❌ No |
| **Expected SHA-256 Checksum Verifier** | ✅ Pre-Flash & In-Flight | ✅ Yes | ❌ No | ✅ Checksum Plugin |
| **Auto-Eject Hardware on Completion** | ✅ Built-in Toggle | ❌ No | ✅ Yes | ❌ No |
| **Application Footprint** | **~4 MB Native Rust** | ~1.5 MB C | ~150 MB Electron | Bootloader + Tools |

---

## 📦 Download Pre-Built Binaries

Download the latest release directly from [**GitHub Releases (`v0.6.1`)**](https://github.com/NR3009/bootlite/releases/latest):

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

## 🏢 Windows AutoPilot Hardware Hash Extractor & Unattended Studio

When you select a Windows 10 or Windows 11 ISO, BootLite automatically unlocks the **Windows Setup & AutoPilot Studio**:

1. **Automatic Laptop Hardware Hash Extraction (`\AutoPilot\` Folder on USB):**
   * BootLite provisions an `AutoPilot` directory directly on the USB drive containing `Extract-AutoPilotHash.ps1` and `Extract-AutoPilotHash.cmd`.
   * Queries WMI `root/cimv2/mdm/dmmap:MDM_DevDetail_Ext01` and `Win32_BIOS` **100% offline** (no internet or PowerShell Gallery required) and saves:
     * `\AutoPilot\AutoPilotHWID_<SerialNumber>_<Hostname>.csv` (per-laptop hash file ready for Microsoft Intune import)
     * `\AutoPilot\AutoPilotHWID_Fleet.csv` (appends hashes across multiple laptops onto the same USB stick)
   * Runs automatically during unattended Windows setup, or can be triggered manually on any laptop by running `\AutoPilot\Extract-AutoPilotHash.cmd`.
2. **Unattended Setup Studio (`autounattend.xml`):**
   * **Basic Setup:** Bypass Windows 11 TPM 2.0, Secure Boot, and 4 GB RAM checks; bypass mandatory online Microsoft Account (`OOBE\BYPASSNRO`); pre-create a Local Administrator account with auto-logon; disable OOBE privacy questions and automatic BitLocker encryption.
   * **Advanced & De-Bloat:** Set custom computer hostname and locale, disable Windows 11 Widgets and consumer bloatware, restore classic right-click context menus, show file extensions, and execute custom first-logon commands.
   * **Live XML Preview:** Inspect, copy, or export the generated `autounattend.xml` directly from the UI.

---

## 🔍 Verification Modes (`Fast` vs `Full` vs `Off`) & Partition Schemes

### 1. Verification Selector (`Verify:`)
* **`Fast (~3s)` (Default — Recommended):**
  * Flushes the physical USB hardware controller cache (`F_FULLFSYNC` / `FlushFileBuffers` / `BLKFLSBUF`).
  * **For Linux / Raw Images:** Verifies **100% of the 4 MB Sector-0 Boot Record (MBR/GPT)** + **32 evenly spaced 64 KB cryptographic sample blocks** across the drive (including tail sectors to catch counterfeit/fake-capacity USB drives).
  * **For Windows 10/11 ISOs:** Verifies all critical UEFI bootloaders (`bootx64.efi`, `bootmgr`, `BCD`, `setup.exe`) and spot-checks **512 KB Head, Middle, and Tail slices** of `sources/boot.wim` and `sources/install.wim`.
* **`Full (100%)`:**
  * Reads back **every single written sector/file (100% of the image)** from physical USB NAND flash and verifies its SHA-256 digest against the source stream.
* **`Off`:**
  * Flushes hardware caches, commits the boot record, and completes immediately after writing.

### 2. Partition Scheme Selector (`Scheme:`)
* **`Auto` (Default):** Preserves built-in `ISOHybrid` (`MBR + GPT`) tables on Linux/BSD images and automatically picks the optimal UEFI partition table when building a Windows 10/11 USB.
* **`GPT`:** Forces a **GUID Partition Table** layout for modern UEFI systems.
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

## 🛠️ Building & Testing From Source

```bash
git clone https://github.com/NR3009/bootlite.git
cd bootlite

# Run the automated unit test suite
cargo test --workspace

# Launch the Studio GUI
cargo run --release -p bootlite-gui

# Build the macOS .app bundle and .dmg installer
./scripts/bundle-macos.sh
```
