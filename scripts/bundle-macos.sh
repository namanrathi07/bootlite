#!/usr/bin/env bash
set -euo pipefail

APP_NAME="BootLite"
VERSION="0.6.5"
TARGET="aarch64-apple-darwin"
BUNDLE_DIR="dist/${APP_NAME}.app"
DMG_PATH="dist/${APP_NAME}-macOS-AppleSilicon.dmg"

echo "==> Ensuring Rust target ${TARGET} is installed..."
rustup target add "${TARGET}" >/dev/null 2>&1 || true

echo "==> Building ${APP_NAME} v${VERSION} (${TARGET})..."
cargo build --release --target "${TARGET}" -p bootlite-gui -p bootlite-cli

rm -rf "${BUNDLE_DIR}" "${DMG_PATH}"
mkdir -p "${BUNDLE_DIR}/Contents/MacOS"
mkdir -p "${BUNDLE_DIR}/Contents/Resources"

cp "target/${TARGET}/release/bootlite-gui" "${BUNDLE_DIR}/Contents/MacOS/${APP_NAME}"
cp "target/${TARGET}/release/bootlite-cli" "${BUNDLE_DIR}/Contents/MacOS/bootlite-cli"
chmod +x "${BUNDLE_DIR}/Contents/MacOS/${APP_NAME}" "${BUNDLE_DIR}/Contents/MacOS/bootlite-cli"

cat << PLIST > "${BUNDLE_DIR}/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>com.namanrathi.bootlite</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "${BUNDLE_DIR}" >/dev/null 2>&1 || true

echo "==> Creating ${DMG_PATH}..."
STAGE_DIR=$(mktemp -d)
cp -R "${BUNDLE_DIR}" "${STAGE_DIR}/"
ln -s /Applications "${STAGE_DIR}/Applications"
hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGE_DIR}" -ov -format UDZO "${DMG_PATH}" >/dev/null
rm -rf "${STAGE_DIR}"

echo "==> Successfully built ${BUNDLE_DIR} and ${DMG_PATH}"
