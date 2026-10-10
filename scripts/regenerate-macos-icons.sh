#!/usr/bin/env bash
# 从 SVG 更新 macOS 图标及外观目录，不修改 Windows / iOS / Android 资源。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
node "$ROOT/scripts/generate-icons.mjs" --macos-only
bash "$ROOT/scripts/compile-macos-appicon.sh"
echo "运行时 Dock 图标需重新编译 Rust：pnpm run build:macos 或 pnpm tauri dev"
