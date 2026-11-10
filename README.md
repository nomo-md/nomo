<p align="center">
  <img src="./assets/128x128.png" width="64" height="64" alt="Nomo 图标">
</p>

<h1 align="center">Nomo</h1>

<p align="center">
  <strong>免费、开源、高性能的所见即所得 Markdown 编辑器。</strong><br>
  支持源码与双栏编辑，集成公式、图表、代码高亮和精细排版。
</p>

<p align="center">
  <a href="https://github.com/nomo-md/nomo/releases"><img src="https://img.shields.io/github/v/release/nomo-md/nomo?style=flat-square&amp;label=release&amp;color=2f7d6f" alt="已发布版本"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-68707a?style=flat-square" alt="Windows、macOS 与 Linux">
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-2f7d6f?style=flat-square" alt="AGPL-3.0-or-later 许可证"></a>
</p>

<p align="center">
  <a href="https://github.com/nomo-md/nomo/releases"><strong>下载 Nomo</strong></a> ·
  <a href="#核心特性">核心特性</a> ·
  <a href="#性能优化">性能</a> ·
  <a href="./README.en.md">English</a>
</p>

<p align="center">
  <img src="./assets/nomo-editor-zh.gif" width="800" alt="Nomo 中文 Markdown 编辑演示">
</p>

## 核心特性

- **免费开源**：采用 AGPL-3.0-or-later 许可证，免费使用，源码公开。欢迎查看源码、反馈问题或参与改进。
- **所见即所得**：编辑时直接呈现 Markdown 格式，支持源码与双栏模式，切换时共用文档和撤销历史。输入 `/` 可快速插入内容。
- **原生高性能**：基于 Rust + GPUI，按需加载与排版，优先处理可见内容，减少重复解析和不必要的资源驻留。
- **丰富的 Markdown 支持**：表格、代码高亮、LaTeX 公式、Mermaid 图表、任务清单、Callout、脚注、行内注释、折叠内容与 Front Matter。
- **精细排版与主题**：可选 Knuth–Plass 断行、英文断词与悬挂标点；四套内置主题、明暗配色、本机字体选择和主题导入导出。显示用连字符不改写原文。
- **本地文档工作区**：文件树、最近文件、多标签、大纲和全文搜索替换；支持自动保存、本地快照与工作区恢复。文档保存在自己的文件夹中。
- **Mini 与文档对照**：Markdown Mini 将当前文档收进小窗，Windows 支持置顶；Windows 组合标签可将两份文档并排阅读。
- **导出与系统集成**：导出 HTML、PDF 和 Word，Word 正文与表格可继续编辑；支持 macOS Quick Look、本地图片管理及可选的 PicGo 上传。

还提供 Markdown 格式检查、自定义快捷键、字数与阅读时间统计，以及简体中文、繁体中文、English、日本語四种界面语言。

## 性能优化

从普通笔记到包含图片、代码和图表的 Markdown，Nomo 通过原生渲染与按需处理减少打开、编辑和切换时的重复工作。

- **按需加载与排版**：优先加载可见区域，按需预取相邻内容并回收离屏片段。
- **多视图共享文档**：所见即所得、源码和双栏共用文档会话与历史，避免每种模式各持有一份完整正文。
- **有界资源管理**：布局、图片、代码高亮和增强渲染采用预算控制；过期任务取消或失效，相关资源随文档关闭回收。

### 首屏可见时间

| 测试场景 | 首屏可见 |
| --- | ---: |
| 5 MiB 纯文本 Markdown | **160 ms** |
| 10 MiB 全节点混合 Markdown | **203 ms** |
| 图片瀑布流 | **119 ms** |
| 七类 Mermaid 图表 | **137 ms** |
| 29 种代码语言 | **216 ms** |
| 多标签：打开第 4 份文档 | **146 ms** |

2026-11-07，Windows / Intel Core i5-12400，三轮测试中位数；从已运行应用的打开请求到首屏文字出现，不含应用启动。测试构建、环境与原始数据见[性能说明](./docs/performance.md)。

## 下载与安装

从 v0.7.0 起，Nomo 正式版本采用 Rust + GPUI 原生架构，不再依赖 WebView。v0.6.x 及更早的 Releases 为历史版本。

从 [GitHub Releases](https://github.com/nomo-md/nomo/releases) 下载适合系统的版本：

| 系统 | 发行文件 |
| --- | --- |
| Windows x64 | `.exe` 安装版 / `.zip` 免安装版 |
| macOS Apple Silicon | `.dmg` / `.app.tar.gz` |
| Linux x64 | `.deb` / `.rpm` |

安装后打开本地 Markdown 文件或文件夹，也可以从文档主页新建。版本变化见[更新记录](https://github.com/nomo-md/nomo/releases)，各平台构建与集成说明见[构建指南](./docs/native-build.md)。

## 从源码运行

使用 stable Rust；根 workspace 的唯一成员是 `src-gpui`，依赖锁定在根 `Cargo.lock`。当前使用 `gpui-pre` / `gpui-pre-platform` **0.3.10**。

```sh
cargo run --locked --bin nomo
cargo run --locked --bin nomo -- /path/to/document.md
cargo check --locked --bin nomo
cargo build --locked --release --bin nomo
```

Windows 需要 MSVC C++ Build Tools、Windows SDK 和 PowerShell 7，打包另需 NSIS 3；macOS 需要完整 Xcode 与 Metal 工具链；Linux 依赖见[构建指南](./docs/native-build.md)。

Windows 一键打包：

```powershell
pwsh -File scripts/package-windows.ps1
```

安装包与便携 ZIP 输出到 `.artifacts/packages/`。NomoMark 开发入口、测试方法与代码归属分别见[开发记录](./docs/nomo-mark.md)、[原生测试工具](./test-lab/README.md)和 [CLASSMAP](./CLASSMAP.md)。

## 参与与许可

通过 [Issue](https://github.com/nomo-md/nomo/issues) 反馈问题时，请附系统、应用版本、复现步骤与可分享的样例；也欢迎提交 [Pull Request](https://github.com/nomo-md/nomo/pulls)。

社区交流见 [linux.do](https://linux.do/)。

原生开发以 `master` 为主线，新功能从 `master` 创建分支并通过 PR 合回；提交推送与正式版本发布分开进行。

Nomo 采用 [AGPL-3.0-or-later](./LICENSE)。Lucide 图标采用 [ISC 许可](./src-gpui/assets/icons/LICENSE)，其他组件保留各自许可证。感谢 GPUI、pulldown-cmark、latex-rust、merman、resvg 等开源项目。
