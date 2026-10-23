#!/usr/bin/env bash
# Kadr.app and its .dmg (SPEC 12.2), on macOS. Takes kadr, kadr-tray and
# kadr-overlay built for both Apple silicon and Intel
#
#   cargo build --profile dist --target aarch64-apple-darwin -p kadr -p kadr-tray -p kadr-overlay
#   cargo build --profile dist --target x86_64-apple-darwin  -p kadr -p kadr-tray -p kadr-overlay
#
# joins each pair into one universal binary (lipo) and puts the three in the
# app. The app starts the tray (a menu bar icon, no Dock icon), which hands
# commands to the kadr beside it. Signed ad hoc only: Apple silicon runs no
# unsigned code at all; Developer ID and notarization come before 1.0.
#
#   scripts/bundle-macos.sh VERSION OUT_DIR   # OUT_DIR/kadr-VERSION-macos-universal.dmg
set -euo pipefail

version=$1
out=$2
root=$(cd "$(dirname "$0")/.." && pwd)
# The bundle's own version is numbers only: 0.3.0-alpha.1 is 0.3.0 there.
short=${version%%-*}

mkdir -p "$out"
app="$out/Kadr.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for bin in kadr kadr-tray kadr-overlay; do
    lipo -create -output "$app/Contents/MacOS/$bin" \
        "$root/target/aarch64-apple-darwin/dist/$bin" \
        "$root/target/x86_64-apple-darwin/dist/$bin"
done
cp "$root/data/macos/Kadr.icns" "$app/Contents/Resources/Kadr.icns"

cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Kadr</string>
    <key>CFBundleDisplayName</key><string>Kadr</string>
    <key>CFBundleIdentifier</key><string>sh.kadr.kadr</string>
    <key>CFBundleExecutable</key><string>kadr-tray</string>
    <key>CFBundleIconFile</key><string>Kadr</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$short</string>
    <key>CFBundleVersion</key><string>$short</string>
    <key>KadrVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSCameraUsageDescription</key><string>Kadr shows your camera in a bubble while it records the screen.</string>
    <key>NSMicrophoneUsageDescription</key><string>Kadr records your voice with the screen.</string>
</dict>
</plist>
EOF

codesign --force --deep --sign - "$app"
codesign --verify --deep "$app"

# The disk image: the app, and Applications to drag it onto.
staging=$(mktemp -d)
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
dmg="$out/kadr-$version-macos-universal.dmg"
rm -f "$dmg"
hdiutil create -volname "Kadr $version" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg"
rm -rf "$staging"
echo "$dmg"
