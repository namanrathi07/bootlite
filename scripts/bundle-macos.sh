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

# Unmount any previously mounted BootLite DMG so Finder never shows a stale window
hdiutil detach "/Volumes/${APP_NAME}" -force 2>/dev/null || true

BUNDLE_DIR="dist/${APP_NAME}.app"
MACOS_DIR="${BUNDLE_DIR}/Contents/MacOS"
RESOURCES_DIR="${BUNDLE_DIR}/Contents/Resources"
ICONSET_DIR="dist/AppIcon.iconset"

rm -rf dist
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${ICONSET_DIR}"

cp "${BIN_DIR}/bootlite-gui" "${MACOS_DIR}/${APP_NAME}"
cp "${BIN_DIR}/bootlite-cli" "${MACOS_DIR}/bootlite-cli"
chmod +x "${MACOS_DIR}/${APP_NAME}" "${MACOS_DIR}/bootlite-cli"

# Generate a crisp 512x512 macOS squircle icon using pure Python stdlib
python3 - << 'PYEOF'
import struct, zlib

W, H = 512, 512
raw = bytearray()
for y in range(H):
    raw.append(0)
    for x in range(W):
        dx = abs(x - 255.5)
        dy = abs(y - 255.5)
        # Squircle mask
        dist = ((dx / 220.0) ** 4 + (dy / 220.0) ** 4) ** 0.25
        if dist > 1.0:
            raw.extend((0, 0, 0, 0))
        else:
            # Lightning bolt polygon check
            nx, ny = (x - 256.0) / 140.0, (y - 256.0) / 140.0
            in_bolt = (
                (ny >= -0.85 and ny <= 0.08 and nx >= -0.55 + (ny + 0.08) * 0.65 and nx <= 0.15 - (ny + 0.08) * 0.35)
                or (ny >= -0.08 and ny <= 0.85 and nx <= 0.55 - (ny - 0.08) * 0.65 and nx >= -0.15 + (ny - 0.08) * 0.35)
            )
            if in_bolt:
                raw.extend((255, 255, 255, 255))
            elif dist > 0.94:
                raw.extend((59, 130, 246, 255))
            else:
                r = int(15 + (1.0 - y / H) * 20)
                g = int(28 + (1.0 - y / H) * 55)
                b = int(75 + (1.0 - y / H) * 135)
                raw.extend((r, g, b, 255))

def chunk(tag, data):
    return struct.pack("!I", len(data)) + tag + data + struct.pack("!I", zlib.crc32(tag + data) & 0xffffffff)

png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack("!IIBBBBB", W, H, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")
open("dist/icon_512.png", "wb").write(png)
PYEOF

for size in 16 32 128 256 512; do
    sips -z $size $size dist/icon_512.png --out "${ICONSET_DIR}/icon_${size}x${size}.png" >/dev/null
    sips -z $((size*2)) $((size*2)) dist/icon_512.png --out "${ICONSET_DIR}/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "${ICONSET_DIR}" -o "${RESOURCES_DIR}/AppIcon.icns"
rm -rf "${ICONSET_DIR}" dist/icon_512.png

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
    <string>0.5.1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.5.1</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>BootLite</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSRemovableVolumesUsageDescription</key>
    <string>BootLite needs access to removable USB drives to flash and verify disk images.</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "${BUNDLE_DIR}"

DMG_STAGING="dist/dmg_staging"
mkdir -p "${DMG_STAGING}"
cp -R "${BUNDLE_DIR}" "${DMG_STAGING}/"
ln -s /Applications "${DMG_STAGING}/Applications"

hdiutil create -volname "${APP_NAME}" \
    -srcfolder "${DMG_STAGING}" \
    -ov -format UDZO \
    "dist/BootLite-macOS-AppleSilicon.dmg"

rm -rf "${DMG_STAGING}"
echo "✅ Created dist/BootLite.app (with AppIcon.icns) and dist/BootLite-macOS-AppleSilicon.dmg"
