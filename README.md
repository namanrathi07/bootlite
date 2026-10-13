# BootLite

A fast, zero-bloat, cross-platform USB image flasher (`macOS`, `Linux`, `Windows`) written in Rust.

## Features

- **Tiny Footprint:** ~5 MB native GUI (`eframe`/`egui`) and ~1.2 MB CLI—no Electron, no telemetry, no ads.
- **Hardware Safety Locks:** Automatically blocks internal system drives (`Apple Fabric`, `/`, `/boot`, `C:\`).
- **True Hardware Speeds:** Uses unbuffered raw device I/O (`/dev/rdiskN` with `F_NOCACHE` on macOS, `O_SYNC` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows).
- **On-the-Fly `.gz` Streaming:** Flash `.iso.gz` and `.img.gz` images directly without extracting first.
- **Read-Back SHA-256 Verification:** Reads back every written sector from the USB drive to verify integrity.

## Quick Start (CLI)

    # List safe removable USB drives
    bootlite-cli list

    # Flash and verify an ISO or compressed .gz image
    sudo bootlite-cli flash ubuntu.iso disk4

## Quick Start (Desktop GUI)

    cargo build --release -p bootlite-gui
    sudo ./target/release/bootlite-gui
