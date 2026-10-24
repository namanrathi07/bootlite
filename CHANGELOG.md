# Changelog

All notable changes to **BootLite** are documented in this file.

## [v0.6.3] - Responsive Multi-OS UI & Windows AutoPilot Hardware Hash Extractor
### Added
- **Windows AutoPilot Hardware Hash Extractor (`\AutoPilot\`):** Automatically provisions `\AutoPilot\Extract-AutoPilotHash.ps1` and `\AutoPilot\Extract-AutoPilotHash.cmd` onto Windows Bootable USB drives. Extracts the laptop's 4K Hardware Hash (`MDM_DevDetail_Ext01`), Serial Number, and Product ID offline and writes Intune-ready CSV files (`AutoPilotHWID_<Serial>_<Host>.csv` and `AutoPilotHWID_Fleet.csv`) directly into the `AutoPilot` folder on the USB drive.
- **Responsive Multi-OS UI Layout:** Unlocked resizable viewport with minimum size constraints and refactored Step 1, Step 2, and Step 3 cards into non-overlapping wrapped multi-row layouts across macOS, Windows, and Linux.
- **Automated Unit Test Suite:** Added unit tests for `autounattend.xml` generation, AutoPilot script & CSV header generation, SHA-256 normalization, and bootable image header inspection.
