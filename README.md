<p align="center">
  <img src="./assets/128x128.png" alt="Nomo 图标" width="60">
</p>

<h1 align="center"><strong>Nomo</strong></h1>

<p align="center">
  <a href="https://github.com/LIXianSenQwQ/nomo/releases">
    <img src="https://img.shields.io/github/v/release/LIXianSenQwQ/nomo?label=release" alt="GitHub Release">
  </a>
  <a href="./LICENSE">
    <img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue" alt="License">
  </a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="Platform">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Svelte-5-FF3E00" alt="Svelte 5">
</p>

<p align="center">
  <a href="./README.md"><strong>简体中文</strong></a>
  ·
  <a href="./README.en.md">English</a>
</p>

---

Nomo 是一款本地优先、Markdown-first 的桌面编辑器，支持 macOS 与 Windows。它以 Markdown 文本作为文档主数据，在语义编辑与源码模式之间保持一致，同时提供 TXT、JSON 大文件分段编辑、文件管理、文档导航和桌面集成能力。

本文描述当前 `master` 的功能；已发布安装包的具体能力与变更，请以对应的 [GitHub Release](https://github.com/LIXianSenQwQ/nomo/releases) 说明为准。

<p align="center">
  <img src="./assets/demo_image.gif" alt="Nomo 演示" width="1920">
</p>

## 下载与安装

从 [GitHub Releases](https://github.com/LIXianSenQwQ/nomo/releases) 下载对应系统的安装包：

| 系统 | 最低版本 | 推荐下载 |
| :--- | :--- | :--- |
| macOS（Apple Silicon / arm64） | 12.0+ | `.dmg`，或通过 Homebrew 安装 |
| Windows | 10/11 | `Nomo_<version>_x64-setup.exe` 安装版 / `Nomo_<version>_x64.zip` 免安装版 |

macOS 也可用 Homebrew：

```bash
brew tap nomo-md/nomo https://github.com/nomo-md/nomo
brew install --cask nomo
```

已安装时执行 `brew upgrade --cask nomo`。

正式版本同时提供 `checksums.md5`，可用于核对下载文件完整性。

当前 GitHub Release 构建未配置 Windows 发行者代码签名或 Apple 公证，首次打开时可能出现 SmartScreen / Gatekeeper 提示。请只从本项目 Release 下载，并在需要时使用校验清单核对文件。

Windows NSIS 安装版与 macOS App 会向系统声明 `.md`、`.markdown`、`.txt` 和 `.json` 的打开能力；Windows 免安装版不会自动注册这些类型。系统声明只会把 Nomo 加入“打开方式”候选，不会强制替换默认应用。Windows 用户还可以在 Nomo 设置中管理 Markdown 默认打开方式与经典右键菜单。

## 主要功能

### Markdown 编辑与渲染

- **语义编辑与源码模式**：基于 ProseMirror 即时解析 Markdown，语义编辑和原文编辑共享同一份文档，可设置默认打开模式。
- **双栏内容跟随**：两栏保留自然高度，以视口 30% 处的内容锚点连续换算滚动位置，不插入空白强制等高。手动滚动的一栏优先；编辑时，仅当对应位置离开另一栏的 15%～85% 可见区域才辅助定位。支持左右交换、分栏比例与可选参考线，复杂图形按块近似对应。实现边界见[双栏同步说明](docs/split-content-sync.md)。
- **原编码安全保存**：打开 Markdown 时识别并保留 UTF-8、UTF-8 BOM、UTF-16 LE / BE BOM 和 GBK；采用临时文件替换方式写盘，避免保存中断留下半份文件。
- **完整文档节点**：支持 H1～H6、段落、硬换行、粗体、斜体、下划线、删除线、高亮、链接、行内代码、行内公式、列表、任务列表、引用、五类 Callout、Front matter、脚注、注释、水平分割线和安全 HTML。
- **技术内容编辑**：代码块支持 Shiki 高亮、标题、语言选择、复制、行号和缩进偏好；KaTeX 支持行内与块级公式；Mermaid 提供流程图、时序图、类图、状态图、饼图、甘特图和 ER 图模板、实时预览及全屏查看。
- **结构化表格与目录**：表格支持尺寸选择、增删行列、列对齐、表头切换和整表删除；正文 TOC 随标题同步，标题层级角标、大纲和脚注导航帮助维护长文档。

### TXT / JSON 大文件

- **分段编辑引擎**：基于 CodeMirror 6 分块读取、全文虚拟滚动和异步行索引，不需要一次性把大文件全部载入前端。
- **完整编辑流程**：支持全文行号、选择、编辑、撤销 / 重做、自动保存、另存为、工作区恢复、外部变更协调和分段恢复日志。
- **后台全文任务**：查找、替换全部和全文复制使用后台流式任务并提供进度与取消；查找和替换另有命中 / 替换计数，JSON 额外支持后台格式化。

### 文件、工作区与数据安全

- **资源管理器**：浏览 `.md`、`.markdown`、`.txt`、`.json`，可新建文件或文件夹、重命名、删除、刷新、折叠全部、复制路径并在 Explorer / Finder 中定位。
- **标签页与多窗口**：支持预览标签、固定标签、标签溢出列表、关闭其他 / 右侧 / 全部标签、最近文件与文件夹、文件拖放打开，以及在当前窗口或新窗口打开文件夹。
- **外部打开文件**：优先切换到已打开的文件；文件的直接父目录已有窗口时，在该窗口增加标签。没有对应目录窗口时，才按打开方式设置处理，并保留初始空窗口复用；新建窗口不会额外唤起无关旧窗口。
- **保存保护**：可启用自动保存并调整延迟；Markdown 可在保存前创建本地快照。源文件只读或检测到外部修改、删除、移动时会暂停自动保存，并提供重新载入、另存为、覆盖外部版本或忽略本次变更等相应处理方式。
- **状态恢复**：恢复工作区、标签页、窗口位置与尺寸、侧栏 / 工具栏状态，以及每个 Markdown、TXT、JSON 标签的阅读或编辑位置。

### 导航与写作辅助

- **当前文档查找替换**：Markdown、TXT、JSON 使用一致的查找面板，支持向前 / 向后、全词、区分大小写、循环、单次替换、全部替换和结果计数。
- **大纲重排**：标题可逐项折叠、全部展开或按默认层级收起；拖拽会连同子章节整节移动，支持放到目标之前、内部或之后，自动调整层级并可撤销。双栏模式下，大纲悬浮覆盖正文，不挤占左右编辑栏的内容宽度。
- **链接导航**：在语义编辑中使用 <kbd>Ctrl</kbd> / <kbd>Cmd</kbd> + 点击，可跳转文内锚点、打开本地 Markdown / TXT / JSON、交给系统打开受支持的 PDF、Office、CSV 与图片附件，或打开 `http`、`https`、`mailto` 外部链接。
- **统计与格式检查**：状态栏可显示行数、词数、字符数、预计阅读时间和缩放；Markdown 格式检查 Beta 提供宽松 / 默认规则、问题列表、源码定位和手动重试。

### 图片、剪贴板与对象操作

- **图片导入**：支持选择、粘贴和拖放图片，可复制到当前目录、统一 `assets`、文档专属 `.assets` 目录，或通过 PicGo Server / PicGo-Core 上传；可设置默认宽度、对齐和可选的未引用本地图片清理。
- **图片交互**：支持全屏查看、滚轮缩放、拖动平移，以及右键修改对齐、尺寸、替代文本、复制图片或路径、定位文件和删除节点。
- **桌面剪贴板**：复制粘贴可保留富文本、纯文本或图片，并在平台能力不足时降级；链接、标题、代码块、表格、公式、Mermaid、标签页、文件树和大纲均有对象级右键菜单。

### 外观、窗口与平台集成

- **主题与排版**：支持跟随系统 / 浅色 / 深色，内置 Nomo 默认、琥珀纸页、经典灰、GitHub 四套配色，以及引用块与 Callout 的经典 / 现代样式；可调字号、行高、正文宽度、界面缩放和 <kbd>Ctrl</kbd> + 滚轮缩放。
- **界面语言**：可跟随系统，或选择简体中文、繁体中文、English、日本語。
- **专注与窗口**：资源管理器、工具栏、大纲和统计可独立显示；Markdown 可进入共享当前编辑状态的可置顶小窗；主窗口可选择关闭、每次询问或隐藏到系统托盘。
- **导出与预览**：Markdown 可导出单文件 HTML，并尽量内嵌可访问的图片；Windows / macOS PDF 会尝试根据标题写入书签。macOS Quick Look 支持 UTF-8 Markdown 的主题、代码、公式和 Mermaid 预览。
- **桌面集成与更新**：Windows 可管理 Markdown 默认打开候选，以及 `.md` / `.markdown` 文件、文件夹和文件夹空白处的经典右键菜单。启动更新检查默认开启且可关闭；NSIS 安装版可在应用内下载并校验更新，免安装版只打开 ZIP 下载链接；macOS 可通过 Homebrew 升级。

## 设置与个性化

| 设置区域 | 当前可配置内容 |
| :--- | :--- |
| 基础行为 | 默认语义 / 源码模式、自动保存与延迟、保存前快照、界面语言 |
| 编辑器 | 字号、行高、内容宽度、引用块与 Callout 的经典 / 现代样式、大文件阈值、代码缩进与行号、行内代码渲染、Markdown Lint 规则 |
| 外观 | 跟随系统 / 浅色 / 深色、四套内置配色、80%～160% 缩放、Ctrl 滚轮缩放 |
| 文件与窗口 | 文件夹打开位置、预览标签、启动时隐藏资源管理器、关闭窗口行为、外部文件变更默认动作、Windows 文件关联与经典右键菜单 |
| 图片 | 本地资源目录或上传策略、PicGo Server / PicGo-Core、连接测试、默认宽度与对齐、未引用本地图片自动清理 |
| 统计与导航 | 大纲、文档统计、默认统计指标、阅读时间、大纲默认展开层级 |
| 高级与关于 | Windows WebView2 硬件 / 软件渲染、默认代码语言、默认 Mermaid 类型、部分命令快捷键、开发日志、启动更新检查 |

## 功能边界

- 查找替换作用于当前文档，不是跨文件全文搜索。
- Markdown 格式检查仍为 Beta，默认关闭，只报告问题、不自动改写文档；大文档会跳过检查，详情面板最多展示前 200 条结果。
- 超过设置阈值的 Markdown 会进入只读源码模式，以降低长文档渲染压力；TXT / JSON 则使用分段编辑器。
- Markdown 会尽量保持原文件编码；GBK 文档若新增了无法表示的字符，保存会失败且原文件不会被覆盖。TXT / JSON 仅 UTF-8 与 UTF-8 BOM 可编辑，其他编码以只读方式打开。
- 本地链接不支持 UNC 路径、`file://`、查询参数和未列入白名单的附件类型；相对链接要求当前 Markdown 已保存。
- 本地图片复制要求文档先保存；PicGo 上传依赖用户自行运行和配置的 PicGo 服务或命令。未引用本地图片自动清理默认关闭，启用后会删除文档目录内对应文件。
- PDF 导出仅支持 Windows 与 macOS，当前固定为 A4 纵向、四边 20 mm；Quick Look 仅支持 macOS，当前只读取 UTF-8 Markdown。
- Windows NSIS 安装版支持应用内检查、下载和安装更新；Windows 免安装版可检查更新并在系统浏览器打开 ZIP 下载链接，退出应用后需手动替换；macOS 可用 Homebrew 升级，或从 Release 页面下载 DMG。

## 默认快捷键

以下为 Windows 默认配置。macOS 原生菜单使用 `CmdOrCtrl` 语义，部分编辑器快捷键仍在持续适配；可在“设置 → 高级”中修改支持自定义的组合键。

### 文件与窗口

| 快捷键 | 作用 |
| :--- | :--- |
| <kbd>Ctrl</kbd> + <kbd>N</kbd> | 新建 Markdown 文件 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>N</kbd> | 新建窗口 |
| <kbd>Ctrl</kbd> + <kbd>O</kbd> | 打开文件 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>O</kbd> | 打开文件夹 |
| <kbd>Ctrl</kbd> + <kbd>S</kbd> | 保存 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>S</kbd> | 另存为 |
| <kbd>Ctrl</kbd> + <kbd>W</kbd> | 关闭当前文件 |

### 编辑与格式

| 快捷键 | 作用 |
| :--- | :--- |
| <kbd>Ctrl</kbd> + <kbd>Z</kbd> | 撤销 |
| <kbd>Ctrl</kbd> + <kbd>Y</kbd> | 重做 |
| <kbd>Ctrl</kbd> + <kbd>B</kbd> | 加粗 |
| <kbd>Ctrl</kbd> + <kbd>I</kbd> | 斜体 |
| <kbd>Ctrl</kbd> + <kbd>U</kbd> | 下划线 |
| <kbd>Alt</kbd> + <kbd>Shift</kbd> + <kbd>5</kbd> | 删除线 |
| <kbd>Ctrl</kbd> + <kbd>`</kbd> | 行内代码 |
| <kbd>Ctrl</kbd> + <kbd>K</kbd> | 插入或编辑链接 |
| <kbd>Ctrl</kbd> + <kbd></kbd> | 清除行内格式 |

### 段落与元素

| 快捷键 | 作用 |
| :--- | :--- |
| <kbd>Ctrl</kbd> + <kbd>1</kbd> ～ <kbd>6</kbd> | 一级至六级标题 |
| <kbd>Ctrl</kbd> + <kbd>0</kbd> | 转换为段落 |
| <kbd>Ctrl</kbd> + <kbd>=</kbd> / <kbd>-</kbd> | 提升 / 降低标题级别 |
| <kbd>Shift</kbd> + <kbd>Enter</kbd> | 段内换行 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>T</kbd> | 插入表格 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>K</kbd> | 插入代码块 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>M</kbd> | 插入公式块 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Q</kbd> | 切换引用块 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>A</kbd> | 插入 Callout |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>[</kbd> / <kbd>]</kbd> | 有序 / 无序列表 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>X</kbd> | 任务列表 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>H</kbd> | 水平分割线 |

### 查找、视图与导出

| 快捷键 | 作用 |
| :--- | :--- |
| <kbd>Ctrl</kbd> + <kbd>F</kbd> | 打开或关闭查找 |
| <kbd>Ctrl</kbd> + <kbd>H</kbd> | 打开或关闭替换 |
| <kbd>Ctrl</kbd> + <kbd>Tab</kbd> / <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Tab</kbd> | 切换标签页 |
| <kbd>Ctrl</kbd> + <kbd>E</kbd> | 切换源码 / 语义编辑模式 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>L</kbd> | 切换浅色 / 深色主题 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>F</kbd> | 显示 / 隐藏资源管理器 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>B</kbd> | 显示 / 隐藏工具栏 |
| <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>M</kbd> | 打开 / 返回 Markdown 小窗 |
| <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>F</kbd> | 格式化当前 JSON 文件 |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>E</kbd> | 导出 HTML |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>P</kbd> | 导出 PDF |

## 技术栈与开发

Nomo 使用 **Tauri 2 + Svelte 5** 构建：

| 层级 | 主要技术 |
| :--- | :--- |
| 前端 | Svelte 5、Vite、TypeScript |
| 桌面运行时 | Tauri 2、Rust |
| Markdown 编辑 | ProseMirror、markdown-it |
| TXT / JSON 编辑 | CodeMirror 6 |
| 渲染 | Shiki、KaTeX、Mermaid |
| 格式检查 | markdownlint、Web Worker |
| 动效 | GSAP |
| 国际化 | Inlang Paraglide JS |

### 环境要求

- [Node.js 22](https://nodejs.org/) 或兼容的 LTS 版本
- [pnpm](https://pnpm.io/) 11.5.1+
- [Rust](https://www.rust-lang.org/tools/install) 与 Cargo
- Windows：Visual Studio 2022 Build Tools 或完整 Visual Studio
- macOS：Xcode Command Line Tools；构建 Quick Look 扩展需要完整 Xcode 工具链

### 常用命令

```bash
pnpm install
pnpm tauri dev
pnpm check
pnpm test

# Windows x64 NSIS 安装包
pnpm run build:win64:nsis

# macOS 应用、DMG 与 Quick Look 扩展
pnpm run build:macos
```

构建产物位于 `src-tauri/target/` 下；使用显式 Rust target 时会进入对应的 `<target>/release/bundle/`，原生目标则位于 `release/bundle/`。

## 项目结构

```text
.
├── assets/                     # 图标、演示图片等静态资源
├── docs/                       # 技术方案、隐私政策与专项说明
├── scripts/                    # 构建辅助脚本与 Quick Look 工具
├── src/
│   ├── app/                    # 应用壳、组件、状态与业务协调
│   ├── lib/editor-core/        # Markdown / ProseMirror 编辑器核心
│   ├── lib/text-editor/        # TXT / JSON 分段编辑器
│   └── quicklook/              # macOS Quick Look 前端渲染
├── src-tauri/                  # Tauri / Rust 后端与原生平台能力
├── sample.md                   # 首次运行实例文档
└── package.json
```

## 路线图

- [ ] 完善 macOS 原生快捷键、触控板手势和菜单语义
- [ ] 设计可控的主题、代码块语言与导出后处理扩展机制
- [ ] 探索 Word、ePub 等更多导出格式
- [ ] 持续完善多语言翻译和界面文案
- [ ] 优化超大 Markdown、长代码块和图片密集文档性能

## 贡献指南

欢迎提交 Issue 与 Pull Request：

1. 提交前先搜索已有 Issue，避免重复。
2. 问题报告尽量包含复现步骤、系统版本、样例文档和截图或 GIF。
3. 功能改动请保持 Markdown-first，并避免引入不必要的私有文档格式。

## 支持项目

<p>
  <a href="https://github.com/LIXianSenQwQ">
    <img src="https://img.shields.io/github/followers/LIXianSenQwQ?style=social" alt="Follow LIXianSenQwQ">
  </a>
  <a href="https://github.com/LIXianSenQwQ/nomo">
    <img src="https://img.shields.io/github/stars/LIXianSenQwQ/nomo?style=social" alt="Star Nomo">
  </a>
</p>

如果 Nomo 对你有帮助，欢迎关注 [LIXianSenQwQ](https://github.com/LIXianSenQwQ)，并为 [Nomo](https://github.com/LIXianSenQwQ/nomo) 点一个 Star。

## License

Nomo 是自由开源软件，使用 [GNU Affero General Public License v3.0 or later](./LICENSE) 授权。你可以使用、修改和再分发 Nomo；分发 Nomo（包括修改版本）或通过网络向用户提供修改版本时，必须遵守 AGPL 并提供对应源代码。AGPL 允许商业使用和收费再分发，但下游不能取消接收者依据 AGPL 获得的权利。

如需闭源集成、专有发行或希望不受 AGPL 开源义务约束，请联系维护者商议单独的商业授权。第三方组件继续适用各自的许可证。

## 社区

- [linux.do](https://linux.do/)

## 致谢

感谢 Tauri、Svelte、ProseMirror、CodeMirror、markdown-it、Shiki、KaTeX、Mermaid、markdownlint、GSAP、Lucide 和 Inlang 等开源项目。

