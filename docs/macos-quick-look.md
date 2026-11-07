# macOS Quick Look

原生分支通过 `Nomo.app/Contents/PlugIns/NomoPreview.appex` 提供 Finder 空格预览。正式包扩展标识为 `com.nomo.desktop.gpui-preview`，并排测试包使用 `com.nomo.desktop.gpui-test.gpui-preview`，使用 Apple 的 [QLPreviewingController](https://developer.apple.com/documentation/quicklookui/qlpreviewingcontroller) 与 AppKit `NSTextView`；不启动 Nomo 主程序、不创建编辑会话，也不使用 WebView 或 JavaScript。已在 macOS 27.2 的 Finder 中验证。并排测试请将专用测试包安装到 `/Applications`，并在系统的 Quick Look 扩展设置中启用 Nomo Markdown Preview。

## 内容与边界

- 注册 `.md` / `.markdown` 对应的 `net.daringfireball.markdown`。接受 UTF-8／UTF-8 BOM、UTF-16 LE／BE BOM、严格 GBK；二进制和无效编码报错。
- 支持标题、粗斜体、删除线、引用、列表和任务列表、GFM 表格、脚注文字、代码高亮、YAML 文首元数据。原始 HTML 作为文字呈现，不执行 HTML 或脚本；不承诺与编辑器的 HTML 容器、目录及自定义语法完全等价。
- 公式和 Mermaid 复用 `embeds_native.rs` 的原生管线；失败、过大或超预算时保留源码。macOS LastResort 占位字体不再被当作可用数学字形，缺字沿用失败回退。
- 预览打开时使用系统明暗外观，代码、公式与图表采用对应配色。暂不读取主应用主题设置；预览期间切换系统外观后，应重新打开以重新生成带颜色的附件。
- 只将 `http`、`https`、`mailto` 链接设为可点击，不激活本地文件、脚本或其它协议。Markdown 的 `data:image/…;base64` 位图直接显示。相对路径图片限定在文档目录内，解析符号链接后再次检查路径。沙箱拒绝读取时，点“选择文档目录以显示图片”，在系统选择器里确认文档所在目录后会重新渲染。扩展只为本次渲染使用所选目录，不保存授权书签。HTTP(S) 图片提供“打开原图”链接，.invalid 测试地址明确提示地址无效。Finder 预览不下载远程资源。
- 正文读取上限 1 MiB；仅普通文件可读，打开后复核类型，并限制读取期间增长。解析上限 30,000 个事件、64 层嵌套、32 列表格、20 MiB 事件文本；解析和原生富文本构造各设 8 秒检查预算。检查发生在阶段和事件边界，不能中断正在执行的系统图像解码或单次 TeX 调用。
- 每次最多 64 个增强对象，单个源不超过 8 KiB，Mermaid 单图计算限时 150 ms；首次字体加载在图表计时前完成，仍计入解析的 8 秒总预算，避免冷启动时字体初始化耗尽图表计算时间。增强位图边长最多 2048、总计最多 32 MiB 解码像素。本地图片最多 64 张、单文件 8 MiB、下采样至 1600 像素边长，总计最多 32 MiB 解码像素。计数是资源预算，不是进程内存上限；JSON、排版、字体和临时解码另占内存。
- 后台串行生成；替换请求或关闭视图会取消旧请求，过期结果不能更新新预览。文件描述符在读取后关闭；视图关闭清空正文与附件。语法库与共享字体库按需加载，不存放文档；没有正文／图片结果缓存、定时器或编辑器常驻任务。扩展二进制为独立载荷，主程序不链接启用后的预览实现。

## 构建与检查

沿用根 Cargo.lock 的依赖与版本，不新建 workspace。需要 Xcode 的 Swift 编译器，以及已有 Rust、Metal 工具与 `jq`。

```sh
# 生成包含扩展的完整 app、tar.gz 与 dmg；不安装、不注册。
bash scripts/package-macos.sh

# 与已有 Nomo 并排安装，使用独立的宿主和扩展标识。
bash scripts/package-macos.sh --side-by-side

# Rust 单元测试、真实 C ABI／AppKit 集成测试、浅深色离屏快照。
bash scripts/test-macos-quick-look.sh

# 仅为已有 staging app 构建扩展；路径必须为绝对路径。
bash scripts/build-macos-quick-look.sh /absolute/staging/Nomo.app
```

测试产物在 `.artifacts/quick-look-tests/`，包含样例与浅深色 PNG。测试覆盖结构、编码、超限、取消、图片路径、受限链接、公式／图表、连续 20 次生成、控制器请求替换、关闭释放与再次显示、窄容器附件缩放，以及仓库 `sample.md`。这些检查不注册扩展，不等同于 Finder 激活验收。

打包顺序为 Rust 静态库、Swift 扩展、带沙箱权限的扩展 ad-hoc 签名、主应用签名和深度验证。`verify-release.py` 同时检查归档内扩展可执行文件与注册元数据。并排 DMG 在 `.artifacts/quick-look-tests/packages/`，正式包仍使用 `.artifacts/packages/`。构建使用 ad-hoc 签名，Developer ID 和公证流程待发行验收。

## 本机验证与待验

2026-11-05：在 macOS 27.2 arm64、Xcode 27.0／Swift 6.4、Homebrew Rust 1.99.1 上构建与测试。浅色／深色离屏排版已检查；Finder 选中 `.md`／`.markdown` 后的空格预览、与其它 Markdown 扩展共存、快速切换文件、窗口缩放、系统权限、签名信任及旧系统兼容性仍需授权实机验收。没有安装应用或注册扩展。

已通过默认 `cargo check --locked --bin nomo`、`cargo build --locked --release --bin nomo`；扩展 Rust 单元测试 4 项通过，共享 `embeds::tests` 6 项通过／1 项既有目视导出测试忽略。Swift／Rust／AppKit 集成测试、完整 macOS 打包、嵌套签名验证、归档注册信息和 DMG 校验通过。命令日志与资产 SHA-256 记录在本机 `.artifacts/quick-look-tests/logs/`；未执行六平台发行资产总检查或远端 CI。

本机 Homebrew Rust 的标准库对象带 macOS 27.0 标记，链接到声明的 12.0 目标时会告警；保留原始告警，不能据本机构建确认 macOS 12 可运行。发行前应使用支持目标最低系统版本的工具链重新构建，并在该系统验证。尚未执行三轮实机内存／输入性能对照，20 次离屏重复生成不构成无泄漏证明。

### 2026-11-06：安装与 Finder 实测

在 macOS 27.2 arm64 上，从已核对 SHA-256 的 0.6.2 DMG 将测试版并排安装到用户应用目录，没有覆盖系统应用目录中的 Nomo 0.8.1。安装副本通过 `codesign --verify --deep --strict`。主程序从安装路径使用独立的 `NOMO_DATA_DIR` 打开 654 字节样例；`NOMO_OPEN_PROFILE` 记录到语义首帧（1280 × 820、2×）。未测试编辑、保存或更新流程。

- 两版扩展均注册时，`pluginkit -m` 在这台机器上选择了 0.8.1。测试期间临时移除其扩展注册；选中 0.6.2 后，在 Finder 对 `.md` 按空格，实际扩展进程位于 `Nomo GPUI Test.app/Contents/PlugIns/NomoPreview.appex/Contents/MacOS/NomoPreview`。
- 预览显示中文、任务列表、表格、Rust 代码高亮、行内／块级公式和 Mermaid 图；按方向键切换到 `.markdown` 并返回，文档内容随选择更新，Esc 正常关闭。
- 同目录 PNG 图片在本次 Finder 沙箱环境只显示替代文字；远程图片也显示替代文字。离屏测试可读的相邻文件不代表 Finder 会授予同样的访问权限。
- 测试后恢复 0.8.1 的注册，并确认它重新成为默认匹配项；0.6.2 测试应用保留。运行记录在本机 `.artifacts/quick-look-install-test/`。本次不覆盖初次下载的 Gatekeeper 信任流程、旧版本 macOS、深色 Finder 或性能基准。

### 2026-11-06：公式与图片复测

截图中显示公式和 Mermaid 源码的进程来自 0.8.1。原测试文档唯一的图片指向 `example.invalid`。为消除两套注册记录的冲突，使用 `--side-by-side` 生成专用测试包，并安装到 `/Applications/Nomo GPUI Test.app`。本机选择测试版扩展，对旧版扩展设置了 ignore。系统中 0.8.1 主程序仍可使用。用上一版用户应用目录路径进行的验收仅适用于上面的历史记录。

Finder 中重新打开原测试文档，确认行内公式、求和分式、Mermaid 和 Base64 内嵌 PNG 均按内容显示。另一个 `.markdown` 文档通过只读系统选择器选择测试文档目录后，显示了相对路径引用的 128 × 128 PNG。有效 HTTPS 外链显示其来源和原图链接，保留的无效网址单列原因。本机试验中，Quick Look 沙箱拒绝扩展的 DNS 服务访问，最终构建沿用无网络权限的预览。

可在系统 Quick Look 扩展设置中切换两版。命令行选择测试版的状态是 `pluginkit -e use -i com.nomo.desktop.gpui-test.gpui-preview` 和 `pluginkit -e ignore -i com.nomo.desktop.quicklook`。如需切回旧版，将两个标识的 use 和 ignore 对调，关闭并重新打开 Finder 预览面板。切换操作只针对这两个扩展。
