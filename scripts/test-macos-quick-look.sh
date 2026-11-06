#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'Run on macOS.' >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
work="$root/.artifacts/quick-look-tests"
mkdir -p "$work"
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
target_dir="$(jq -r '.target_directory' <<< "$metadata")"
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo test --locked --release --lib --features quick-look
cargo rustc --locked --release -p nomo-gpui --target aarch64-apple-darwin --lib --features quick-look --crate-type staticlib
source_dir="$root/src-gpui/macos-quick-look"
xcrun swiftc -swift-version 5 -O -parse-as-library -target arm64-apple-macosx12.0 \
  -sdk "$(xcrun --show-sdk-path)" -import-objc-header "$source_dir/PreviewBridge.h" \
  "$source_dir/PreviewViewController.swift" "$source_dir/NativeMarkdownRenderer.swift" "$source_dir/PreviewImages.swift" "$source_dir/PreviewTests.swift" \
  "$target_dir/aarch64-apple-darwin/release/libnomo_quick_look.a" \
  -framework AppKit -framework QuickLookUI -framework ImageIO -framework Security -framework SystemConfiguration \
  -Xlinker -dead_strip -o "$work/preview-tests"
"$work/preview-tests" "$work/fixtures" "$root/sample.md"
