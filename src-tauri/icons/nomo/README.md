# Nomo 图标资源

本目录存放 Nomo 的 SVG 母版及从矢量生成的浅深色、托盘、Windows、macOS 图标。

## 修改与生成

应用造型只修改 `source/nomo-app-light.svg`。深色 SVG 共用相同几何结构，配色与托盘造型在 `scripts/generate-icons.mjs` 中维护。PNG、ICO、ICNS 都是生成产物，不要直接编辑位图。

```bash
pnpm icons:generate
```

该命令复用已有 Tauri CLI，更新各平台资源，无额外图像库依赖。生成时先把 SVG 渲染到 1024px，再输出所需尺寸，避免将 128/256px 位图放大。设置、关于、更新与更新完成弹窗直接引用浅深色 SVG。

## Windows

- `../icon.ico` 包含 16、24、32、48、64、256px，用于 EXE、安装器及文件关联；运行时保留固定打包图标。
- `windows/` 包含 15 档 `targetsize` 的默认、深色 `altform-unplated`、浅色 `altform-lightunplated` 资源，以及 125%、150%、200%、400% 的基础 Logo 缩放资源。
- MSIX 构建通过 `scripts/msix/Build-IconResources.ps1` 复制资源并生成 `resources.pri`，让系统按实际尺寸和外观选择图标。无限定符的基础 PNG 作为 100% 缩放回退，不额外生成重复的 `scale-100` 候选。

## macOS 两套图标（重要）

macOS 上 Dock / 启动台 / Cmd+Tab 看到的图标来自以下资源：

| 资源 | 用途 |
|------|------|
| `../icon.icns` | 无外观目录时的回退；旧系统或未编进 `Assets.car` 时使用 |
| `Assets.car`（由 `macos/*-catalog-1024.png` 编译） | 应用**未运行**时 Finder / Dock / 启动台按系统浅色/深色选图 |
| `macos/nomo-app-light-1024.png` / `dark` | 应用**运行后**由 Rust 调用 `setApplicationIconImage` 设置 Dock / Cmd+Tab |

因此只改 `icon.icns` 而应用已在运行，或只重打前端包、未重编译 Rust，Dock 图标**不会变化**。
退出后要跟随系统，必须把 `Assets.car` 打进 `.app`（`pnpm run build:macos` 会调用 `scripts/compile-macos-appicon.sh`）。

ICNS 与运行时 PNG 共用同一边距：底板宽度占画布的 `218/256`（约 85.2%）。`*-catalog-1024.png` 则从 SVG 的底板边界直接导出满幅图层，由 Icon Composer 应用平台遮罩；编译脚本只校验并复制 1024px RGBA，不再裁切、拉伸小 PNG。

只更新 macOS 资源并编译外观目录，可在 macOS 上执行：

```bash
bash scripts/regenerate-macos-icons.sh
pnpm run build:macos
```

`node scripts/generate-icons.mjs --macos-only` 只生成 macOS 相关资源，不调用 Xcode，也不更新 Windows / iOS / Android 资源。`Assets.car` 编译与实际 Dock 外观仍需 macOS 和 Xcode 验证。

## 托盘

浅深色各提供激活、非激活状态。SVG 使用不透明的圆角底板、对比边框和清晰的 N 与状态横线：浅色为白底深色 N，深色为深底浅色 N。应用主题与系统任务栏外观可能不同，底板和边框让图标在两种任务栏背景上都能辨认；未激活状态保留底板，仅将 N 和横线改为灰色。运行时嵌入 48px PNG，供高 DPI 缩放使用，同时保留 24px 预览。主题与激活态切换继续通过 `set_desktop_icon_theme` / `set_tray_active` 同步。

## 目录说明

- `source/`: 浅色 SVG 母版、生成的深色 SVG，以及 light/dark × 128/256/512/1024 PNG。
- `macos/`: 应用平台边距的 256/512/1024 PNG，以及外观目录使用的满幅 1024px 图层。
- `tray/`: light/dark × active/inactive SVG、24px 预览和 48px 运行时 PNG。
- `windows/`: MSIX 的目标尺寸、主题和缩放限定资源。
- 上级目录的标准 PNG、ICO、ICNS 及既有 Android / iOS 图标也由同一母版生成。

## 命名规则

- 文件名前缀统一使用 `nomo-`。
- `app-light` / `app-dark` 表示应用 logo 浅色或深色版本。
- `tray-light` / `tray-dark` 表示跟随应用浅色或深色主题的托盘图标；两者均带底板，兼容系统栏的浅深色背景。
- `active` / `inactive` 表示激活态或未激活态。
- 数字后缀表示导出像素尺寸。
- `catalog-1024` 表示 Icon Composer 外观目录图层。

## 当前接入状态

- 应用内图标直接使用 SVG；更新弹窗也随浅深色主题选图。
- macOS 运行时 Dock 图标由 `src-tauri/src/window/tray.rs` 嵌入 1024px PNG。
- 托盘嵌入四个带圆角底板的 48px PNG，独立保留 N 与状态横线造型。
- Windows 安装态任务栏图标保持固定 bundle 图标，避免动态切换导致任务栏按钮闪烁、消失、重排或受快捷方式图标缓存影响。
