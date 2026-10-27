# 原生构建与三平台发布

根目录是单成员 Cargo workspace。crate 为 `nomo-gpui`，二进制为 `nomo`，产品版本只在根 `Cargo.toml` 定义；不通过标签临时改写源码版本。根锁文件保留既有依赖版本。新增直接依赖 `url` 和构建期 `embed-resource` 复用锁文件已有版本，分别用于文件 URL 和 Windows 资源。

## 构建和检查

```sh
cargo check --locked --bin nomo
cargo check --locked --features editor-control-demo --all-targets
cargo build --locked --release --bin nomo
cargo build --locked --release --features editor-control-demo --bin nomo
cargo test --locked --release --bin nomo
cargo test --locked --release --features editor-control-demo --bin nomo
```

指定 `--target` 时产物在 `target/<target>/release/`，否则在 `target/release/`。两种 feature 配置使用同一二进制名：打包脚本始终重新构建默认配置。`--skip-build`／`-SkipBuild` 仅用于已确认默认产物的本地重复打包。

### Windows x64

需要 stable Rust MSVC、Visual Studio C++ Build Tools、Windows SDK 和 NSIS 3，在 x64 Developer PowerShell 环境运行。`fxc.exe` 和 `rc.exe` 必须可查找；GPUI 也支持进程变量 `GPUI_FXC_PATH`。

```powershell
pwsh -NoProfile -File scripts/package-windows.ps1
```

安装包使用当前用户范围，复用 `Software\Microsoft\Windows\CurrentVersion\Uninstall\Nomo` 卸载键与旧 InstallLocation。保留快捷方式和“打开方式”，不改写默认程序。卸载仅删除已知文件，不递归删除安装目录或配置。旧版全机／商店安装不属于此 NSIS 升级路径。

打包使用 `dumpbin` 检查实际导入表，随安装包和便携 ZIP 放入工具链 `VCToolsRedistDir` 下的 x64 `vcruntime140.dll`。系统 DLL 由 Windows 提供，运行库依赖发生变化时打包失败并要求更新清单；不从系统目录随意复制 DLL。分发方式参考 [Microsoft 的运行库部署说明](https://learn.microsoft.com/en-us/cpp/windows/determining-which-dlls-to-redistribute?view=msvc-170)。

### macOS arm64

需要完整 Xcode、Metal、Command Line Tools 和 `jq`。

```sh
rustup target add aarch64-apple-darwin
bash scripts/package-macos.sh
```

应用名 `Nomo.app`、标识 `com.nomo.desktop`，最低系统版本元数据保持 12.0。使用 ad-hoc 签名，签名或校验失败终止打包；不包含公证和 Quick Look。`.app` 同时保留于输出的任务 staging 目录。

### Linux x64

CI 使用 Ubuntu 24.04，构建和打包依赖如下：

```sh
sudo apt-get install clang libclang-dev cmake pkg-config libssl-dev \
  libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libxcb1-dev libxcb-xkb-dev libxcb-randr0-dev \
  libxcb-xinput-dev libxcb-cursor-dev libvulkan-dev libegl1-mesa-dev \
  libasound2-dev libudev-dev fonts-noto-cjk xdg-desktop-portal \
  dpkg-dev rpm desktop-file-utils jq
bash scripts/package-linux.sh
```

包包含桌面入口、图标、许可证和样本；保留 X11／Wayland。直接动态依赖由 `dpkg-shlibdeps`／RPM 扫描，另外声明运行时加载的图形、字体和桌面库。Ubuntu 24.04 二进制不承诺兼容更旧 glibc；RPM 仍需目标发行版安装验证。

## 发行契约

输出目录为 `.artifacts/packages/`，staging 为 `.artifacts/package-work/`。脚本不自动安装应用或清理用户目录。

| 平台 | 文件名（VERSION 来自 Cargo） |
|---|---|
| Windows | `Nomo_VERSION_x64-setup.exe`、`Nomo_VERSION_x64.zip` |
| macOS | `Nomo_VERSION_aarch64.dmg`、`Nomo_aarch64.app.tar.gz` |
| Linux | `Nomo_VERSION_amd64.deb`、`Nomo-VERSION-1.x86_64.rpm` |

分支推送、PR 和手动 workflow dispatch 仅构建、检查并上传工作流产物。只有推送匹配 Cargo 版本的 `v*.*.*` 标签才发布，且必须存在对应发布说明。

三平台全部成功后，验证六类资产及归档结构、生成 `checksums.md5`，上传草稿 Release，最后转为公开。已公开的同名 Release 不允许覆盖。稳定版继续更新 Homebrew；下载站使用既有部署钩子，不在本次工作中触发。

上述资产名已对照 v0.6.2 正式 Release，RPM 包身份保持 `nomo`。预发布标签仍可使用：资产名保留完整 Cargo 版本，Linux 包内部以 `~` 表示预发布，macOS bundle 数字版本取主／次／补丁段。

## 数据和功能边界

配置目录沿用旧版：Windows `%APPDATA%/com.nomo.desktop`、macOS `~/Library/Application Support/com.nomo.desktop`、Linux `~/.local/share/com.nomo.desktop`。JSON 未知键保留，设置通过原子替换写入，不清理旧文档、快照或缓存。

导出、Quick Look、MSIX／Shell 扩展尚未接入。默认 Workspace 与开发入口共用窗口化 EditorControl 和文档会话，feature 仅控制开发外壳，差距见 Roadmap。SQLite、编码、搜索及 Emoji 等沿用已锁定依赖，现由默认构建链接；没有新增依赖版本。

隔离验收可设置进程变量 `NOMO_DATA_DIR` 为任务专用目录；配置、会话和保存前快照均落入该目录。未设置时沿用既有平台数据目录。该变量不修改系统环境，不用于默认安装或迁移用户数据。

## 验收记录原则

源码迁移、静态检查、编译、包生成、安装运行和发布分别记录。本次本机验证记录：

- 96 个 Rust 源文件语法、编译期资源路径、Cargo TOML、workflow YAML 和任务依赖、PowerShell／Bash／Python 脚本语法通过。
- `cargo metadata --locked --all-features --offline` 解析 825 个包；全目标、全部 feature 的 `cargo tree` 未包含 Tauri、Wry、WebView2 或 WKWebView 应用壳。原锁文件依赖版本保持不变。
- 2026-10-26 在 Windows x64 安装 Rust 1.99.1、Visual Studio Build Tools 2022 17.14.42、MSVC 14.44、Windows SDK 10.0.26100 和 NSIS 3.13。此前缺少 `link.exe` 的环境阻塞已解除。
- `cargo check --locked --target x86_64-pc-windows-msvc --bin nomo` 和附加 `--features editor-control-demo --all-targets` 的检查均通过；既有警告保留。
- Windows 默认入口和 `editor-control-demo` 的 Release 可执行文件均构建通过。默认配置既有测试 69 通过、1 忽略；开发配置 148 通过、7 忽略，均无失败。忽略项维持原规则，不计为通过。
- 一条既有 Windows ChangeTime 测试最初因同一时钟粒度内的原地改写失败；确认迁移前后实现相同后，经用户授权仅在改写前等待 50 毫秒，全部断言保留。完整测试集和该测试额外连续三次复验通过，生产身份比较与磁盘格式未改变。
- `scripts/package-windows.ps1` 已实际完成默认 Release 构建及 NSIS／ZIP 打包：`Nomo_0.6.2_x64-setup.exe`、`Nomo_0.6.2_x64.zip` 位于 `.artifacts/packages/`。已核对版本、x64 PE、图标／manifest／版本资源、归档完整性、运行库与样本布局，以及安装包／ZIP／构建输出的可执行文件哈希一致；未包含 WebView 或前端 bundle。
- 尚未运行 macOS／Linux 构建与打包，也未触发远程 CI。未启动应用或安装器，Windows 升级／卸载及三平台桌面交互仍待独立实机验收；本地产物不是正式发布。

本次不新增正文副本、缓存、渲染层或后台任务。系统文件打开适配增加启动前路径队列，交给 Workspace 后清空；不据此宣称运行时状态完全零增量。

发版前需在授权环境核对：Windows 升级／卸载、macOS 签名和启动、Linux 安装／桌面打开，以及文件打开、保存、撤销、中文输入法、设置持久化。记录系统、构建版本、DPI 和未完成项。旧版功能、视觉与性能对照另行验收，不据删除量宣称性能收益。
