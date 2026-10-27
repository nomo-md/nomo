#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'Run on macOS with Xcode and its Metal toolchain.' >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target=aarch64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=12.0
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
version="$(jq -r '.packages[] | select(.name == "nomo-gpui") | .version' <<< "$metadata")"
target_dir="$(jq -r '.target_directory' <<< "$metadata")"
if [[ "${1:-}" != --skip-build ]]; then
  cargo build --locked --release --target "$target" --bin nomo
fi
binary="$target_dir/$target/release/nomo"
test -f "$binary"
[[ "$(lipo -archs "$binary")" == arm64 ]] || { echo 'Expected an arm64 executable' >&2; exit 1; }
output="$root/.artifacts/packages"
mkdir -p "$output" "$root/.artifacts/package-work"
stage="$(mktemp -d "$root/.artifacts/package-work/macos.XXXXXX")"
app="$stage/Nomo.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/samples/assets"
cp "$binary" "$app/Contents/MacOS/nomo"
cp src-gpui/assets/app/icon.icns "$app/Contents/Resources/Nomo.icns"
cp LICENSE "$app/Contents/Resources/LICENSE"
cp src-gpui/assets/icons/LICENSE "$app/Contents/Resources/LICENSE-Lucide"
cp src-gpui/assets/licenses/LICENSE-STIX "$app/Contents/Resources/LICENSE-STIX"
cp sample.md "$app/Contents/Resources/samples/sample.md"
cp assets/128x128.png "$app/Contents/Resources/samples/assets/128x128.png"
bundle_version="${version%%[-+]*}"
sed "s/@VERSION@/$bundle_version/g" scripts/packaging/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
# 发行格式与现有下载站一致。
tar -czf "$output/Nomo_aarch64.app.tar.gz" -C "$stage" Nomo.app
volume="$stage/dmg"
mkdir -p "$volume"
ditto "$app" "$volume/Nomo.app"
ln -s /Applications "$volume/Applications"
hdiutil create -ov -volname Nomo -srcfolder "$volume" -format UDZO "$output/Nomo_${version}_aarch64.dmg"
hdiutil verify "$output/Nomo_${version}_aarch64.dmg"
echo "macOS packages: $output (app: $app)"
