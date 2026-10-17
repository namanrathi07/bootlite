# ⚡ BootLite (`v0.6.0`)

**Cross-Platform Raw Hardware USB Flasher, Windows 10/11 Unattended & AutoPilot Studio, and SHA-256 Verifier**

*Built in 100% Native Rust (`bootlite-core`, `bootlite-gui`, `bootlite-cli`) for macOS, Windows, and Linux.*

[![Release](https://img.shields.io/badge/release-v0.6.0-3b82f6?style=flat-square)](https://github.com/NR3009/bootlite/releases/latest)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-10b981?style=flat-square)](#download-pre-built-binaries)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

---

## 🚀 Key Capabilities

**BootLite (`~4 MB` single binary)** is a native GPU-accelerated Rust application and CLI engineered for system administrators, DevOps engineers, and power users across macOS, Windows, and Linux:

| Capability | ⚡ BootLite `v0.6.0` |
| :--- | :---: |
| **Native macOS, Windows & Linux GUI + CLI** | ✅ All 3 Operating Systems |
| **Windows 10 / 11 UDF ISO Bootable USB Builder** | ✅ Auto (FAT32 UEFI + NTFS/ExFAT Dual-Partition) |
| **Windows AutoPilot Hardware Hash Extractor (`\AutoPilot\`)** | ✅ Saves Intune CSV (`AutoPilotHWID_*.csv`) directly on USB |
| **Windows 10 / 11 Unattended Setup Studio (`autounattend.xml`)** | ✅ Built-in Hardware/Account Bypasses & De-Bloat |
| **4 MB / 8 MB High-Throughput Burst Engine** | ✅ Full USB Bus Speed |
| **Smart Verification (`Fast ~3s` / `Full 100%` / `Off`)** | ✅ 3 Selectable Modes |
| **On-the-Fly `.xz` (Multi-Stream), `.zst`, `.bz2`, `.gz`** | ✅ Built-in Pure Rust |
| **Partition Scheme Selector (`Auto` / `GPT` / `MBR`)** | ✅ Yes |
| **One-Click USB ExFAT Reset / Capacity Recovery** | ✅ Built-in |
| **Expected SHA-256 Checksum Verifier** | ✅ Pre-Flash & In-Flight |
| **Auto-Eject Hardware on Completion** | ✅ Built-in Toggle |
| **Binary Footprint** | **~4 MB Native Rust** |

---

## 📦 Download Pre-Built Binaries

Download the latest release directly from [**GitHub Releases (`v0.6.0`)**](https://github.com/NR3009/bootlite/releases/latest):

* 🍎 **macOS (Apple Silicon):** [`BootLite-macOS-AppleSilicon.dmg`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-macOS-AppleSilicon.dmg)
* 💻 **Windows 10 / 11 (x64 Portable — Zero DLL Dependencies):** [`BootLite-Windows-x86_64-Portable.exe`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Windows-x86_64-Portable.exe)
* 🐧 **Linux (x86_64 Tarball — GUI + CLI):** [`BootLite-Linux-x86_64.tar.gz`](https://github.com/NR3009/bootlite/releases/latest/download/BootLite-Linux-x86_64.tar.gz)
* 🔐 **Release Checksums:** [`SHA256SUMS.txt`](https://github.com/NR3009/bootlite/releases/latest/download/SHA256SUMS.txt)

### 🍎 macOS First-Launch Note
Run this once after dragging `BootLite.app` to `/Applications`:

```bash
xattr -cr /Applications/BootLite.app
```

---

## 🏢 Windows AutoPilot Hardware Hash Extractor & Unattended Studio

When you select a Windows 10 or Windows 11 ISO, BootLite unlocks the **Windows Setup & AutoPilot Studio**:

1. **Automatic Laptop Hardware Hash Extraction (`\AutoPilot\` Folder on USB):**
   * BootLite automatically provisions an `AutoPilot` folder on the USB drive containing `Extract-AutoPilotHash.ps1` and `Extract-AutoPilotHash.cmd`.
   * Queries WMI `root/cimv2/mdm/dmmap:MDM_DevDetail_Ext01` and `Win32_BIOS` **100% offline** (no internet or PowerShell Gallery required) and saves:
     * `\AutoPilot\AutoPilotHWID_<SerialNumber>_<Hostname>.csv` (per-laptop hash file ready for Microsoft Intune import)
     * `\AutoPilot\AutoPilotHWID_Fleet.csv` (appends hashes across multiple laptops onto the same USB stick)
   * Runs automatically during unattended Windows setup, or can be launched anytime on any laptop by double-clicking `\AutoPilot\Extract-AutoPilotHash.cmd`.
2. **Unattended Setup Studio (`autounattend.xml`):**
   * Bypass Windows 11 TPM 2.0, Secure Boot, and 4 GB RAM checks.
   * Bypass mandatory online Microsoft Account (`OOBE\BYPASSNRO`) and pre-create a Local Administrator account.
   * Disable BitLocker auto-encryption, OOBE privacy prompts, and consumer bloatware.

---

## 🔍 Verification Modes (`Fast` vs `Full` vs `Off`) & Partition Schemes

* **`Fast (~3s)` (Default — Recommended):** Flushes the physical USB hardware controller cache, verifies **100% of the 4 MB Sector-0 Boot Record (MBR/GPT)** + **32 evenly spaced 64 KB cryptographic sample blocks** across the drive (or all UEFI bootloaders + WIM head/mid/tail slices on Windows ISOs).
* **`Full (100%)`:** Reads back every single written sector/file from physical USB NAND flash and verifies its SHA-256 digest against the source stream.
* **`Off`:** Flushes hardware caches, commits the boot record, and completes immediately after writing.

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
