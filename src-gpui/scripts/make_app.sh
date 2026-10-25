#!/usr/bin/env bash
# 把 release 二进制打包成 macOS 应用 nomo-gpui.app（复用 src-tauri 的应用图标）。
# 用法：scripts/make_app.sh [二进制路径]，默认 target/release/nomo-gpui；产物在 target/nomo-gpui.app。
set -eu
cd "$(dirname "$0")/.."
binary="${1:-target/release/nomo-gpui}"
app="target/nomo-gpui.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/nomo-gpui"
cp ../src-tauri/icons/icon.icns "$app/Contents/Resources/nomo-gpui.icns"
version="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.nomo.gpui</string>
  <key>CFBundleName</key><string>nomo-gpui</string>
  <key>CFBundleDisplayName</key><string>nomo-gpui</string>
  <key>CFBundleExecutable</key><string>nomo-gpui</string>
  <key>CFBundleIconFile</key><string>nomo-gpui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array><dict>
    <key>CFBundleTypeName</key><string>Markdown</string>
    <key>CFBundleTypeRole</key><string>Editor</string>
    <key>LSItemContentTypes</key><array><string>net.daringfireball.markdown</string><string>public.plain-text</string></array>
  </dict></array>
</dict></plist>
PLIST
codesign --force -s - "$app" >/dev/null 2>&1 || true
echo "$app"
