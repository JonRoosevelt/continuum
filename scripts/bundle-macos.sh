#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
APP="dist/Continuum.app"

cargo build --release -p continuum-app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/continuum "$APP/Contents/MacOS/continuum"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Continuum</string>
  <key>CFBundleDisplayName</key><string>Continuum</string>
  <key>CFBundleIdentifier</key><string>dev.continuum.app</string>
  <key>CFBundleExecutable</key><string>continuum</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature for local use; replace with a Developer ID for distribution.
codesign --force --deep --sign - "$APP"

echo "built $APP"
echo "for distribution: codesign --sign 'Developer ID Application: ...' then notarytool submit"
