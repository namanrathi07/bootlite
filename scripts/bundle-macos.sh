#!/usr/bin/env bash
set -euo pipefail

APP_NAME="BootLite"
TARGET="${1:-}"

if [ -n "$TARGET" ]; then
    cargo build --release --target "$TARGET"
    BIN_DIR="target/${TARGET}/release"
else
    cargo build --release
    BIN_DIR="target/release"
fi

BUNDLE_DIR="dist/${APP_NAME}.app"
MACOS_DIR="${BUNDLE_DIR}/Contents/MacOS"
RESOURCES_DIR="${BUNDLE_DIR}/Contents/Resources"

rm -rf dist
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

cp "${BIN_DIR}/bootlite-gui" "${MACOS_DIR}/${APP_NAME}"
cp "${BIN_DIR}/bootlite-cli" "${MACOS_DIR}/bootlite-cli"
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
    <string>0.1.1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.1</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>BootLite</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSRemovableVolumesUsageDescription</key>
    <string>BootLite needs access to removable USB drives to flash and verify disk images.</string>
</dict>
</plist>
PLIST

# Ad-hoc sign the .app bundle so Apple Silicon macOS allows it to run
codesign --force --deep --sign - "${BUNDLE_DIR}"

# Build a drag-to-Applications .dmg installer
DMG_STAGING="dist/dmg_staging"
mkdir -p "${DMG_STAGING}"
cp -R "${BUNDLE_DIR}" "${DMG_STAGING}/"
ln -s /Applications "${DMG_STAGING}/Applications"

hdiutil create -volname "${APP_NAME}" \
    -srcfolder "${DMG_STAGING}" \
    -ov -format UDZO \
    "dist/BootLite-macOS-AppleSilicon.dmg"

rm -rf "${DMG_STAGING}"
echo "✅ Created dist/BootLite.app and dist/BootLite-macOS-AppleSilicon.dmg"
