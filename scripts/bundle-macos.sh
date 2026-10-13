#!/usr/bin/env bash
set -euo pipefail

APP_NAME="BootLite"
BUNDLE_DIR="dist/${APP_NAME}.app"
MACOS_DIR="${BUNDLE_DIR}/Contents/MacOS"
RESOURCES_DIR="${BUNDLE_DIR}/Contents/Resources"

echo "Building release binaries..."
cargo build --release

rm -rf "${BUNDLE_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

cp target/release/bootlite-gui "${MACOS_DIR}/${APP_NAME}"
cp target/release/bootlite-cli "${MACOS_DIR}/bootlite-cli"
chmod +x "${MACOS_DIR}/${APP_NAME}" "${MACOS_DIR}/bootlite-cli"

cat << 'PLIST' > "${BUNDLE_DIR}/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>BootLite</string>
    <key>CFBundleDisplayName</key>
    <string>BootLite</string>
    <key>CFBundleIdentifier</key>
    <string>dev.bootlite.app</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>BootLite</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSRemovableVolumesUsageDescription</key>
    <string>BootLite needs access to removable USB drives to flash and verify disk images.</string>
</dict>
</plist>
PLIST

echo "✅ Created ${BUNDLE_DIR} ("$(du -sh "${BUNDLE_DIR}" | cut -f1)")"
