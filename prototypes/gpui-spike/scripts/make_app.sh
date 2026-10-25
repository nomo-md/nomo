#!/usr/bin/env bash
# 打包最小 .app，使原型拥有 bundle id，便于 UI 自动化授权与输入法实测。
set -eu
app="target/NomoGpuiSpike.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/nomo-gpui-spike "$app/Contents/MacOS/"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.nomo.gpui-spike</string>
  <key>CFBundleName</key><string>NomoGpuiSpike</string>
  <key>CFBundleExecutable</key><string>nomo-gpui-spike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force -s - "$app" >/dev/null 2>&1 || true
echo "$app"
