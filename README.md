# ⚡ BootLite

[![Build & Release](https://github.com/namanrathi07/bootlite/actions/workflows/build.yml/badge.svg)](https://github.com/namanrathi07/bootlite/actions)
[![Latest Release](https://img.shields.io/github/v/release/namanrathi07/bootlite?color=blue)](https://github.com/namanrathi07/bootlite/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**BootLite** is a fast, zero-bloat, cross-platform USB image flasher (`macOS`, `Linux`, `Windows`) written in pure Rust. It includes both a native dark-mode desktop GUI (`bootlite-gui`) and a headless CLI (`bootlite-cli`).

---

## Why BootLite?

| Feature | ⚡ BootLite | BalenaEtcher | Rufus |
| :--- | :--- | :--- | :--- |
| **Download Size** | **~2.8 MB** | ~250+ MB (Electron) | ~1.5 MB |
| **Cross-Platform** | **macOS, Linux, Windows** | macOS, Linux, Windows | Windows Only |
| **CLI + Native GUI** | **Both Included** | GUI Only | GUI Only |
| **Telemetry / Ads** | **Zero** | Built-in Telemetry / Ads | Zero |
| **Read-Back SHA-256 Check** | **Built-in** | Checksum Only | Optional |
| **On-the-Fly `.gz` Flashing** | **Yes** | Yes | Yes |

---

## Download Latest Release

Grab the prebuilt binaries for your OS from the [**Releases Page**](https://github.com/namanrathi07/bootlite/releases/latest):

| Operating System | Architecture | Package | Size |
| :--- | :--- | :--- | :--- |
| **macOS** | Apple Silicon (M1–M4) | [`BootLite-macOS-AppleSilicon.tar.gz`](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-macOS-AppleSilicon.tar.gz) | ~2.96 MB |
| **Windows** | 64-bit (`x86_64`) | [`BootLite-Windows-x86_64.zip`](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-Windows-x86_64.zip) | ~2.77 MB |
| **Linux** | 64-bit (`x86_64`) | [`BootLite-Linux-x86_64.tar.gz`](https://github.com/namanrathi07/bootlite/releases/latest/download/BootLite-Linux-x86_64.tar.gz) | ~6.93 MB |

---

## Key Features

- **Hardware Safety Locks:** Automatically blocks internal system drives (`Apple Fabric`, `/`, `/boot`, `/home`, `C:\`) so you can never accidentally wipe your OS disk.
- **Unbuffered Raw I/O:** Writes directly to hardware (`/dev/rdiskN` with `F_NOCACHE` on macOS, `O_SYNC` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows) for real write speeds and accurate progress bars.
- **On-the-Fly `.gz` Decompression:** Flash `.iso.gz` and `.img.gz` images directly without extracting them to disk first.
- **Sector-by-Sector Read-Back Verification:** Computes the source `SHA-256` hash while streaming and reads back every written byte from the USB drive to guarantee a 100% bit-perfect flash.

---

## Quick Start (CLI)

    # List safe removable USB drives
    bootlite-cli list

    # Flash and verify an ISO or compressed .gz image
    sudo bootlite-cli flash ubuntu.iso disk4

## Building from Source

    git clone https://github.com/namanrathi07/bootlite.git
    cd bootlite
    cargo build --release
