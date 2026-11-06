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
skip_build=false
bundle_id=com.nomo.desktop
app_name=Nomo
asset_name=Nomo
output="$root/.artifacts/packages"
for option in "$@"; do
  case "$option" in
    --skip-build) skip_build=true ;;
    --side-by-side)
      bundle_id=com.nomo.desktop.gpui-test
      app_name="Nomo GPUI Test"
      asset_name=Nomo_GPUI_Test
      output="$root/.artifacts/quick-look-tests/packages" ;;
    *) echo "Unknown packaging option: $option" >&2; exit 1 ;;
  esac
done
if [[ "$skip_build" == false ]]; then
  cargo build --locked --release --target "$target" --bin nomo
fi
binary="$target_dir/$target/release/nomo"
test -f "$binary"
[[ "$(lipo -archs "$binary")" == arm64 ]] || { echo 'Expected an arm64 executable' >&2; exit 1; }
mkdir -p "$output" "$root/.artifacts/package-work"
stage="$(mktemp -d "$root/.artifacts/package-work/macos.XXXXXX")"
app="$stage/$app_name.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/samples/assets"
cp "$binary" "$app/Contents/MacOS/nomo"
cp src-gpui/assets/app/icon.icns "$app/Contents/Resources/Nomo.icns"
cp LICENSE "$app/Contents/Resources/LICENSE"
cp src-gpui/assets/icons/LICENSE "$app/Contents/Resources/LICENSE-Lucide"
cp src-gpui/assets/licenses/LICENSE-STIX "$app/Contents/Resources/LICENSE-STIX"
cp src-gpui/assets/licenses/LICENSE-Rendering "$app/Contents/Resources/LICENSE-Rendering"
cp src-gpui/assets/licenses/LICENSE-Export "$app/Contents/Resources/LICENSE-Export"
cp sample.md "$app/Contents/Resources/samples/sample.md"
cp assets/128x128.png "$app/Contents/Resources/samples/assets/128x128.png"
bundle_version="${version%%[-+]*}"
sed -e "s/@VERSION@/$bundle_version/g" -e "s/@BUNDLE_ID@/$bundle_id/g" -e "s/@APP_NAME@/$app_name/g" \
  scripts/packaging/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
bash scripts/build-macos-quick-look.sh "$app"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
# 发行格式与现有下载站一致。
tar -czf "$output/${asset_name}_aarch64.app.tar.gz" -C "$stage" "$app_name.app"
volume="$stage/dmg"
mkdir -p "$volume"
ditto "$app" "$volume/$app_name.app"
ln -s /Applications "$volume/Applications"
hdiutil create -ov -volname "$app_name" -srcfolder "$volume" -format UDZO "$output/${asset_name}_${version}_aarch64.dmg"
hdiutil verify "$output/${asset_name}_${version}_aarch64.dmg"
echo "macOS packages: $output (app: $app)"
