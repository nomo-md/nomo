# Design: 跨平台安装包镜像竞速与 macOS 自动更新 (Design)

## 1. Architecture

```text
updates.rs
  官方优先、gh-proxy 回退的 Release 检查、版本比较、Release Notes、安装形态路由
                         │
install_ui.rs（Windows + macOS 共用）
  下载状态、单一百分比、取消、下载完成和重启确认
                         │
installer.rs（共享）
  Candidate、官方资产 URL、gh-proxy URL、延迟竞速、大小与 SHA-256、临时文件胜负裁决
                         │
          ┌──────────────┴──────────────┐
          ▼                             ▼
installer/windows.rs             installer/macos.rs
注册表安装身份                    当前 .app 身份和稳定可写位置
Windows 文件共享/缓存锁            macOS 缓存锁、安全解压和 bundle 校验
NSIS 启动                         退出后辅助命令、同卷替换、回滚和重启
```

模块安排：

- `src-gpui/src/ui/updates.rs` 保留唯一 Release 检查入口；安装器和安装 UI 改为 Windows/macOS 编译。
- `src-gpui/src/ui/updates/installer.rs` 保存跨平台候选包与下载编排。
- `src-gpui/src/ui/updates/installer/windows.rs` 保留 NSIS、注册表和 Windows 文件语义。
- `src-gpui/src/ui/updates/installer/macos.rs` 实现 `.app.tar.gz`、bundle 校验及退出后替换。
- `src-gpui/src/ui/updates/install_ui.rs` 由两平台共用，不复制 UI 状态机。
- `src-gpui/src/main.rs` 将 `QuitReason::Install` 和更新辅助命令接入 macOS；辅助命令必须在 GPUI、配置会话和窗口初始化之前运行。

平台适配采用编译期同签名函数或轻量 trait，不引入全局动态分发、常驻服务或第二套更新状态。

## 2. Data Model & Interfaces

共享数据结构：

```rust
struct Candidate {
    version: String,
    asset_name: String,
    official_url: String,
    size: u64,
    digest: [u8; 32],
}

enum DownloadSource { Official, GhProxy }

struct AttemptProgress {
    source: DownloadSource,
    downloaded: u64,
    total: u64,
}
```

平台适配职责：

```text
installation() -> 当前安装身份；不支持时给出便携/手动更新原因
candidate(release) -> 平台固定版本、架构和资产约束
cache_lease() -> 一个进程内两路尝试共享的跨进程租约
prepare_attempt(source) -> 独立临时文件
verify_download(...) -> 大小和 SHA-256 校验后保留文件读句柄和缓存租约
finish(candidate, winner) -> 两路任务退出后仅对胜者做平台准备，生成 Verified
Verified::launch() -> 启动 NSIS 或 macOS 退出后辅助命令
```

`ReleaseAsset.size` 和 `digest` 不再仅限 Windows。macOS 与 Windows 都必须获得非空大小和 `sha256:` 摘要。Release 元数据优先请求官方 API，失败或超时时通过 gh-proxy 请求同一地址；只允许代理官方仓库的 `releases/latest` 和更新成功提示所需的 `releases/tags/v<当前版本>`。代理 URL 由固定 `https://gh-proxy.com/` 前缀与经过白名单校验的官方 URL 构造，不接受用户输入提供任意代理地址。

本次不引入独立更新签名：官方元数据正常时，SHA-256 以官方响应为准；元数据回退到 gh-proxy 时，明确信任代理返回的清单和哈希。无论来源如何，都必须通过相同的版本、官方页面/资产 URL、资产唯一性、大小和摘要格式校验。安装包的 gh-proxy 地址只能由官方 `github.com/nomo-md/nomo/releases/download/...` URL 构造。

### 竞速策略

- 官方请求正常连接并持续产生数据时，不创建镜像请求。
- 官方连接失败、可重试 HTTP 错误、读取失败或校验失败时立即启动镜像；若官方任务已结束，则退化为单路镜像。
- 按官方源最近收到数据的时间（独立于界面百分比）判断停滞。官方连接或读取持续停滞达到内部超时时，启动镜像但不主动取消仍在运行的官方任务。
- 每个任务仍受连接、读取、总时长和平台包大小上限约束。
- 两路写不同临时文件；首个字节数精确匹配且 SHA-256 正确的任务获胜。
- 获胜后取消另一任务并清理其文件；失败或校验错误不覆盖另一任务的结果。
- 两路都失败才向 UI 报告失败，错误保留两个来源的简洁原因，不显示堆栈或敏感 URL 参数。

### 进度与资源

- UI 只有一个更新任务，显示两路进度的较高值，不相加、不平均。
- 显示进度保持单调；领先任务失败时冻结显示并等待另一任务追上，下载完成但尚未校验时保持 99%。
- 取消同时作用于两路；关闭所有者窗口但仍有其他窗口时转移所有者，所有窗口退出时取消。
- 同一时刻最多两路下载、两份平台安装包临时文件。Windows 保持单包 128 MiB 上限；macOS 单包上限 256 MiB，因此临时磁盘上限分别为 256 MiB 和 512 MiB，另加 macOS 解压 staging。
- 缓存继续使用 `native-updates` 和跨进程 `.active` 租约；启动时清理超过 24 小时且未占有的已知更新残留，不使用通配递归删除。

## 3. Data Flow & Interaction

1. 官方 Release 检查发现新版本；官方失败或超时时由 gh-proxy 回退获取同一 API，平台安装适配器随后判断是否支持应用内更新。
2. 用户点击下载，共享 Candidate 校验版本、资产唯一性、官方 URL、大小和 SHA-256。
3. 官方下载开始；仅在失败或停滞时启动 gh-proxy。
4. 共享下载器汇报单任务进度；首个完整校验成功包进入平台 `Verified`。
5. 用户点击“重启并安装”，复用 `can_quit_to_install` 和正常文档关闭流程；取消关闭时保留已验证包。
6. Windows 持有胜者的文件读锁，从校验时的同一绝对路径启动 NSIS，保留 `/NOMOUPDATE=<PID> /P /R` 参数；缓存文件名以 `.official.exe` 或 `.gh-proxy.exe` 结尾。
7. macOS 启动复制到更新缓存中的辅助可执行文件，旧进程正常退出。
8. macOS 辅助命令等待旧 PID 退出，自行定位 `native-updates` 并加锁，再读取固定的 `macos-staging/Nomo.app`。暂存路径不接受命令行输入；拒绝链接目录。复核新旧 bundle 后在目标父目录同卷复制、备份、替换和重启。
9. 替换或启动失败时尽量恢复旧 bundle 并重新打开；不删除用户文档或应用数据。
10. 新版启动沿用 `nativeReleaseNotesVersion` 显示更新成功和 Release Notes。

### macOS 包与安装约束

- 仅接受正式 `com.nomo.desktop`、arm64、严格更高版本的 `Nomo_aarch64.app.tar.gz`。
- 当前可执行文件必须位于规范化 `<Nomo.app>/Contents/MacOS/nomo`；拒绝 side-by-side 测试 bundle、App Translocation、DMG/只读位置、符号链接身份和父目录不可写位置。
- 使用 macOS 系统 libarchive 单遍流式读取，逐条目检查 4096 项／1 GiB 预算并支持取消，只向空 staging 写入；在写入前拒绝绝对路径、`..`、链接及其他非普通目录/文件条目，解压后再次核对单一 `Nomo.app` 根和路径包含关系。
- 解压后核对 `Info.plist` 的 bundle 标识和版本，读取主程序的 32 字节 Mach-O 头确认其为单架构 arm64 可执行文件（不依赖运行时 `lipo`），再运行 `/usr/bin/codesign --verify --deep --strict`。这里验证现有 ad-hoc 签名完整性，不要求 Developer ID 或公证。
- 替换 staging 必须位于原应用父目录，保证 rename 同卷；只有经过全部校验后才启动退出安装流程。

## 4. Error Handling

- **官方 API 不可用**: 通过 gh-proxy 请求同一官方 API；两个来源都失败时沿用现有启动静默/手动报错策略，不开始包下载。代理元数据没有独立签名保护，文档和错误处理不得将其描述为与官方来源具有相同信任边界。
- **官方包下载异常**: 启动或切换镜像；官方已经产生的临时文件不能污染镜像文件。
- **镜像内容被修改**: 大小或 SHA-256 不匹配时拒绝执行，并允许另一有效源继续。
- **用户取消或退出**: 两路任务都取消，已知临时文件删除，已验证包只在尚未开始退出安装时保留供重试。
- **macOS 位置或权限不支持**: 不创建替换辅助任务，不修改原 `.app`，更新详情回退到官方手动下载。
- **macOS 解压/签名/bundle 校验失败**: 删除 staging，不进入重启确认。
- **旧进程未在 60 秒内退出**: 辅助命令中止，不替换应用。
- **替换失败**: 在旧应用仍完整可用时恢复；保留诊断所需的最小已知残留，后续按缓存策略清理。
- **启动安装器失败**: Windows 沿用重新打开工作区并报错；macOS 同样在尚未退出时恢复 UI，退出后则由辅助命令恢复并打开旧版。
