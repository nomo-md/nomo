# Nomo

[English](README.en.md) · [下载](https://github.com/nomo-md/nomo/releases) · [构建与发布](docs/native-build.md)

本地优先的原生 Markdown 编辑器，使用 Rust + GPUI。编译和打包不需要 Node.js、npm、pnpm、Vite 或 Tauri。

当前默认 Workspace 已接入窗口化 EditorControl，标签正文、历史和保存由文档会话管理；普通启动、新建和打开无需开发参数。现有文件树、工具栏、搜索替换与统计正在逐项对齐主分支。语义／源码和同会话双栏均已接入；双栏、菜单外壳与阅读位置同步尚待实机验收。完整状态与未验证项见[开发记录](docs/gpui-editor-control.md)和[Roadmap](src-gpui/ROADMAP.md)。

**当前源码尚未达到历史桌面版的功能与视觉等价水平。** 导出 PDF／Word／HTML／图片、系统 Quick Look、MSIX 商店包和 Shell 扩展暂时下线；外部文件监听、托盘、完整工作区恢复、自定义快捷键和多语言等仍未完成。历史正式版继续从 Releases 获取，原生收敛本身不会发布新版。

## 开发

需要 stable Rust；依赖锁定在根 `Cargo.lock`，GPUI 保持 0.2.2。

- Windows：MSVC C++ Build Tools、Windows SDK（含 `fxc.exe`、`rc.exe`）；打包需要 NSIS 3 和 PowerShell 7。
- macOS：完整 Xcode、Command Line Tools 和 Metal 编译工具；打包使用系统 `codesign`、`hdiutil` 及 `jq`。
- Linux：C/C++ 编译器、Clang、字体／X11／Wayland／图形库开发包；依赖命令见[构建说明](docs/native-build.md)。

```sh
cargo run --locked --bin nomo
cargo run --locked --bin nomo -- /path/to/document.md
cargo check --locked --bin nomo
cargo build --locked --release --bin nomo
```

开发控件：

```sh
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control-demo
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control /path/to/document.md
```

验证命令、六类发行资产和实机待验项目见[构建与发布](docs/native-build.md)。应用结构见 [CLASSMAP](CLASSMAP.md)，后续工作见 [Roadmap](src-gpui/ROADMAP.md)。

## 兼容性与许可

应用标识保持 `com.nomo.desktop`，配置结构和既有平台数据目录继续保留。尚未使用的设置键不会被清除，卸载器不删除用户配置或文档。

采用 [AGPL-3.0-or-later](LICENSE)。Lucide 图标采用 [ISC 许可](src-gpui/assets/icons/LICENSE)，Rust 组件遵循各自许可证。感谢 GPUI、pulldown-cmark、latex-rust、merman、resvg 等项目。
