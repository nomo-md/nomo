#!/usr/bin/env bash
# 构建产物仅写入传入的 .app staging；不注册、不安装或启动扩展。
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'Quick Look requires macOS and Xcode.' >&2; exit 1; }
[[ $# == 1 ]] || { echo 'Usage: build-macos-quick-look.sh path/to/Nomo.app' >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
app="$1"
[[ "$app" == /* && -d "$app/Contents" ]] || { echo 'Expected an absolute staged .app path.' >&2; exit 1; }
cd "$root"
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
version="$(jq -r '.packages[] | select(.name == "nomo-gpui") | .version' <<< "$metadata")"
target_dir="$(jq -r '.target_directory' <<< "$metadata")"
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo rustc --locked --release -p nomo-gpui --target aarch64-apple-darwin --lib --features quick-look --crate-type staticlib
source_dir="$root/src-gpui/macos-quick-look"
extension="$app/Contents/PlugIns/NomoPreview.appex"
mkdir -p "$extension/Contents/MacOS"
xcrun swiftc -swift-version 5 -O -application-extension -parse-as-library \
  -target arm64-apple-macosx12.0 -sdk "$(xcrun --show-sdk-path)" -module-name NomoPreview \
  -import-objc-header "$source_dir/PreviewBridge.h" \
  "$source_dir/PreviewViewController.swift" "$source_dir/NativeMarkdownRenderer.swift" "$source_dir/PreviewImages.swift" \
  "$target_dir/aarch64-apple-darwin/release/libnomo_quick_look.a" \
  -framework AppKit -framework QuickLookUI -framework ImageIO -framework Security -framework SystemConfiguration \
  -Xlinker -dead_strip -Xlinker -e -Xlinker _NSExtensionMain \
  -o "$extension/Contents/MacOS/NomoPreview"
host_id="$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")"
[[ "$host_id" == com.nomo.desktop || "$host_id" == com.nomo.desktop.gpui-test ]] || { echo "Unexpected host identifier: $host_id" >&2; exit 1; }
sed -e "s/@VERSION@/${version%%[-+]*}/g" -e "s/@HOST_ID@/$host_id/g" "$source_dir/Info.plist" > "$extension/Contents/Info.plist"
plutil -lint "$extension/Contents/Info.plist" "$source_dir/Entitlements.plist"
codesign --force --sign - --entitlements "$source_dir/Entitlements.plist" "$extension"
codesign --verify --strict "$extension"
echo "Quick Look extension: $extension"
