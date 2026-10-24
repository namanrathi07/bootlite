# Contributing to BootLite ⚡

Thank you for your interest in contributing to **BootLite**! Our goal is to keep BootLite the fastest, safest, and lightest cross-platform USB flasher and Windows deployment studio in the Rust ecosystem.

## 🏗️ Workspace Architecture

- **`crates/bootlite-core`**: Pure Rust flashing engine, decompression pipelines (`.xz`, `.zst`, `.bz2`, `.gz`), Windows 10/11 UDF UEFI builder (`windows_iso.rs`), OS-specific hardware safety locks (`platform/macos.rs`, `platform/linux.rs`, `platform/windows.rs`), and AutoPilot script generators (`unattend.rs`).
- **`crates/bootlite-gui`**: Native `egui` / `eframe` GPU-accelerated desktop studio application.
- **`crates/bootlite-cli`**: Command-line interface for headless servers and automation.

## 🛡️ Hardware Safety Rule (#1 Priority)

Any Pull Request touching `crates/bootlite-core/src/platform/` **must** preserve kernel-level system drive lockouts (`disk0` / `Apple Fabric` on macOS, `PhysicalDrive0` / `C:` on Windows, and `/`, `/boot`, `/home` on Linux).

## 🧪 Pre-Submit Cross-Target Check

Before opening a Pull Request, verify that your changes compile cleanly across all three OS targets and pass all unit tests:

```bash
cargo check --workspace --target aarch64-apple-darwin
cargo check --workspace --target x86_64-unknown-linux-gnu
cargo check --workspace --target x86_64-pc-windows-gnu
cargo test --workspace
```

## 📜 Pull Request & Code Rules

1. **Zero Unsafe System Disk Operations:** Never bypass `is_system` or raw bus filters.
2. **Zero Third-Party Branding in UI:** Keep the application interface (`crates/`) clean and self-contained without competitor tool names.
3. **Cross-Platform Parity:** New core features in `bootlite-core` must compile cleanly on macOS, Windows, and Linux.

## 💡 Submitting Issues & Feature Requests

- **Bug Reports:** [Open a Bug Report](https://github.com/namanrathi07/bootlite/issues/new?template=bug_report.md) with your host OS details and Live Hardware Log output.
- **Feature Requests:** [Suggest an Enhancement](https://github.com/namanrathi07/bootlite/issues/new?template=feature_request.md).
