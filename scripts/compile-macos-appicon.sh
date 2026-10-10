#!/usr/bin/env bash
# 把 light/dark PNG 编进 Assets.car，让应用未运行时 Dock / 启动台也能跟随系统外观。
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "compile-macos-appicon.sh 只能在 macOS 上运行。" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIGHT="$ROOT/src-tauri/icons/nomo/macos/nomo-app-light-catalog-1024.png"
DARK="$ROOT/src-tauri/icons/nomo/macos/nomo-app-dark-catalog-1024.png"
OUT_DIR="$ROOT/src-tauri/target/appicon"

if [[ ! -f "$LIGHT" || ! -f "$DARK" ]]; then
  echo "缺少 macOS 1024px 图标，请先执行 pnpm icons:generate: $LIGHT / $DARK" >&2
  exit 1
fi

resolve_actool() {
  if xcrun --find actool >/dev/null 2>&1; then
    return 0
  fi
  local app
  for app in /Applications/Xcode.app /Applications/Xcode-beta.app; do
    if [[ -x "$app/Contents/Developer/usr/bin/actool" ]]; then
      export DEVELOPER_DIR="$app/Contents/Developer"
      return 0
    fi
  done
  echo "找不到 actool。请安装 Xcode（Command Line Tools 不够），以便编译跟随系统的 App 图标。" >&2
  exit 1
}

resolve_actool

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ICON="$TMP/AppIcon.icon"
mkdir -p "$ICON/Assets" "$OUT_DIR"

python3 - "$LIGHT" "$DARK" "$ICON" <<'PY'
import json
import shutil
import struct
import sys
from pathlib import Path


light_src, dark_src, icon_dir = map(Path, sys.argv[1:4])
assets = icon_dir / "Assets"
for src, filename in [(light_src, "nomo-app-light.png"), (dark_src, "nomo-app-dark.png")]:
    data = src.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{src} 必须是 PNG")
    width, height, bit_depth, color_type = struct.unpack(">IIBB", data[16:26])
    if (width, height, bit_depth, color_type) != (1024, 1024, 8, 6):
        raise SystemExit(f"{src} 必须是从 SVG 导出的 1024x1024、8-bit RGBA PNG")
    shutil.copyfile(src, assets / filename)

layer = {
    "glass": False,
    "image-scale": "fill",
    "opacity": 1,
    "position": {"scale": 1, "translation-in-points": [0, 0]},
}
payload = {
    "fill": "none",
    "groups": [
        {
            "layers": [
                {
                    **layer,
                    "hidden-specializations": [{"appearance": "dark", "value": True}],
                    "image-name": "nomo-app-light.png",
                    "name": "Light",
                },
                {
                    **layer,
                    "hidden-specializations": [
                        {"value": True},
                        {"appearance": "dark", "value": False},
                    ],
                    "image-name": "nomo-app-dark.png",
                    "name": "Dark",
                },
            ],
            "lighting": "combined",
            "name": "Nomo",
            "shadow": {"kind": "none", "opacity": 0},
            "specular": False,
            "translucency": {"enabled": False, "value": 0},
        }
    ],
    "supported-platforms": {"squares": ["macOS"]},
}
(icon_dir / "icon.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

xcrun actool \
  --output-format human-readable-text \
  --notices \
  --warnings \
  --errors \
  --platform macosx \
  --minimum-deployment-target 15.0 \
  --target-device mac \
  --app-icon AppIcon \
  --compile "$OUT_DIR" \
  --output-partial-info-plist "$TMP/partial.plist" \
  "$ICON"

if [[ ! -f "$OUT_DIR/Assets.car" ]]; then
  echo "actool 没有生成 Assets.car" >&2
  exit 1
fi

echo "已生成跟随系统的 App 图标: $OUT_DIR/Assets.car"
