# ⚡ BootLite

[![Build & Release](https://github.com/namanrathi07/bootlite/actions/workflows/build.yml/badge.svg)](https://github.com/namanrathi07/bootlite/actions)
[![Latest Release](https://img.shields.io/github/v/release/namanrathi07/bootlite?color=blue)](https://github.com/namanrathi07/bootlite/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**BootLite** is a fast, zero-bloat, cross-platform USB image flasher (`macOS`, `Linux`, `Windows`) written in pure Rust.

---

## Direct Download & Install

Grab the ready-to-run installer or portable app for your OS from [**GitHub Releases**](https://github.com/namanrathi07/bootlite/releases/latest):

| Operating System | Direct Download | How to Run |
| :--- | :--- | :--- |
| **macOS** (Apple Silicon M1–M4) | [**`BootLite-macOS-AppleSilicon.dmg`**](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-macOS-AppleSilicon.dmg) | Open the `.dmg`, drag **`BootLite.app`** to `Applications`, and double-click to launch. *(If macOS Gatekeeper prompts on first launch, right-click `BootLite.app` -> **Open**).* |
| **Windows** (64-bit) | [**`BootLite-Windows-x86_64-Portable.exe`**](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-Windows-x86_64-Portable.exe) | Zero-install portable executable (like Rufus). Right-click -> **Run as Administrator**. |
| **Linux** (`x86_64`) | [**`BootLite-Linux-x86_64.tar.gz`**](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-Linux-x86_64.tar.gz) | Extract and double-click **`BootLite`** (or run `sudo ./BootLite`). |

---

## Why BootLite?

| Feature | ⚡ BootLite | BalenaEtcher | Rufus |
| :--- | :--- | :--- | :--- |
| **Download Size** | **~3 MB** | ~250+ MB (Electron) | ~1.5 MB |
| **Cross-Platform** | **macOS, Linux, Windows** | macOS, Linux, Windows | Windows Only |
| **CLI + Native GUI** | **Both Included** | GUI Only | GUI Only |
| **Telemetry / Ads** | **Zero** | Built-in Telemetry / Ads | Zero |
| **Read-Back SHA-256 Check** | **Built-in** | Checksum Only | Optional |
| **On-the-Fly `.gz` Flashing** | **Yes** | Yes | Yes |

---

## Key Features

- **Hardware Safety Locks:** Automatically blocks internal system drives (`Apple Fabric`, `/`, `/boot`, `/home`, `C:\`) so you can never accidentally wipe your OS disk.
- **Unbuffered Raw I/O:** Writes directly to hardware (`/dev/rdiskN` with `F_NOCACHE` on macOS, `O_SYNC` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows) for real write speeds and accurate progress bars.
- **On-the-Fly `.gz` Decompression:** Flash `.iso.gz` and `.img.gz` images directly without extracting them to disk first.
- **Sector-by-Sector Read-Back Verification:** Computes the source `SHA-256` hash while streaming and reads back every written byte from the USB drive to guarantee a 100% bit-perfect flash.
