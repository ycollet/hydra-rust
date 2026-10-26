#!/usr/bin/env bash
set -euo pipefail

: "${VERSION:?Set VERSION to the package version}"
: "${NUMERIC_VERSION:?Set NUMERIC_VERSION to a numeric bundle version}"
: "${MACOSX_DEPLOYMENT_TARGET:?Set MACOSX_DEPLOYMENT_TARGET to the minimum macOS version}"
target="${1:?Usage: package-macos.sh <Rust target>}"
stage="$PWD/target/installer-$target"
app="$stage/Hydra.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist
install -m 755 "target/$target/release/hydra" "$app/Contents/MacOS/hydra"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Hydra</string>
  <key>CFBundleDisplayName</key><string>Hydra</string>
  <key>CFBundleIdentifier</key><string>org.sova.hydra-rust</string>
  <key>CFBundleExecutable</key><string>hydra</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${NUMERIC_VERSION%.*}</string>
  <key>CFBundleVersion</key><string>${NUMERIC_VERSION%.*}</string>
  <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
ln -sfn /Applications "$stage/Applications"
image="dist/hydra-rust-$VERSION-$target.dmg"
hdiutil create -volname Hydra -srcfolder "$stage" -ov -format UDZO "$image"
hdiutil verify "$image"
