---
id: "cross-platform-update-mirror"
kind: feature
parent: ""
status: in_progress
impact_radius:
  - "src-gpui/src/ui/updates.rs"
  - "src-gpui/src/ui/updates/installer.rs"
  - "src-gpui/src/ui/updates/installer/{windows,macos}.rs"
  - "src-gpui/src/ui/updates/install_ui.rs"
  - "src-gpui/src/main.rs"
  - "src-gpui/src/ui/workspace.rs"
  - "docs/native-build.md"
dependencies:
  - "existing reqwest, tokio, sha2, serde and tempfile"
  - "macOS system /usr/bin/tar, /usr/bin/codesign and /usr/bin/open"
---

# Specification: 跨平台安装包镜像竞速与 macOS 自动更新 (Specification)

## 1. Scope
- **In Scope**: Windows NSIS 安装包官方优先、异常时 gh-proxy 延迟竞速；共享下载、校验、进度和取消；macOS `.app.tar.gz` 应用内更新；平台安装适配层；相关文档、既有测试和关键新逻辑测试。
- **Out of Scope**: Windows 便携版或 MSIX 自动替换；Linux 自动更新；独立更新公私钥签名；Apple Developer ID、公证、Sparkle、appcast、增量更新、断点续传、分块下载和自定义代理设置。

## 2. Functional Requirements

### ADDED
#### Requirement: 官方优先的元数据代理回退
The system SHALL 优先从官方 GitHub API 获取 Release、版本、资产大小和 SHA-256，只在官方请求失败或超时时通过 gh-proxy 请求同一官方 API 地址；本次不引入独立更新签名，代理元数据回退明确信任 gh-proxy 返回的清单和哈希。

##### Scenario: 官方 API 正常
- **WHEN** 官方 Release API 请求成功
- **THEN** 不产生代理元数据请求，沿用官方元数据

##### Scenario: 官方 API 不可用
- **WHEN** 官方 Release API 请求失败或超时
- **THEN** 通过 gh-proxy 请求同一 API，并对返回版本、官方页面/资产地址、资产唯一性、大小和 SHA-256 执行与官方响应相同的严格校验

##### Scenario: 两个元数据来源均不可用
- **WHEN** 官方和 gh-proxy 都无法返回合法 Release 元数据
- **THEN** 不构造更新候选，沿用启动静默或手动报错策略

#### Requirement: 官方优先的镜像延迟竞速
The system SHALL 先下载官方安装包，只在官方连接失败、可重试响应、读取失败、校验失败或持续停滞时启动 gh-proxy；官方持续正常下载时不得产生镜像请求。

##### Scenario: 官方正常
- **WHEN** 官方源持续下载并通过校验
- **THEN** 不启动 gh-proxy，沿用现有安装流程

##### Scenario: 官方停滞
- **WHEN** 官方任务达到内部连接或读取停滞阈值
- **THEN** 启动 gh-proxy，并允许仍存活的官方任务继续竞争

#### Requirement: 完整校验后的胜者裁决
The system SHALL 为两路使用独立临时文件，只接受首个大小精确匹配且 SHA-256 等于官方元数据的完整文件，随后取消另一任务并清理其临时文件。

##### Scenario: 较快文件校验失败
- **WHEN** 先完成的任务大小或 SHA-256 不匹配
- **THEN** 不运行该文件，删除对应临时文件，并允许另一任务继续

##### Scenario: 两路均失败
- **WHEN** 官方和镜像都无法产生有效文件
- **THEN** 更新进入可重试错误状态，不启动平台安装器

#### Requirement: 单任务进度与取消
The system SHALL 将双路下载呈现为一个更新任务，以较高进度显示、不相加或平均，并在下载完成但校验未完成时保持未就绪状态。

##### Scenario: 领先任务失败
- **WHEN** 当前较高进度来源失败而另一来源仍在下载
- **THEN** 显示进度不归零、不倒退，另一来源追上后继续增长

##### Scenario: 用户取消
- **WHEN** 用户取消下载或最后一个应用窗口退出
- **THEN** 两路请求均取消，相关已知临时文件被清理，不保留可安装状态

#### Requirement: 编译期平台适配层
The system SHALL 共用 Candidate、竞速、哈希、进度、取消和安装 UI 状态，只将安装身份、平台文件语义、包复核和安装启动放入 Windows/macOS 适配器。

##### Scenario: 修改共享下载行为
- **WHEN** 调整竞速或进度规则
- **THEN** 两平台使用同一实现，不出现两份独立状态机

#### Requirement: macOS 应用内候选包
The system SHALL 为正式 macOS arm64 安装选择 `Nomo_aarch64.app.tar.gz`，要求严格更高稳定版本、唯一资产、官方 URL、非空大小不超过 256 MiB 和官方 SHA-256。

##### Scenario: macOS 正式可写安装
- **WHEN** 当前运行于稳定的正式 `Nomo.app` 且父目录可写
- **THEN** 更新详情提供与 Windows 一致的应用内下载入口

##### Scenario: 不支持的 macOS 运行位置
- **WHEN** 当前为 side-by-side 测试包、App Translocation、DMG/只读位置、异常 bundle 或父目录不可写
- **THEN** 不提供自动替换，保留官方手动下载入口且不修改原应用

#### Requirement: macOS 安全暂存与包复核
The system SHALL 在重启确认前完成安全解压和包复核，包括路径边界、链接拒绝、单一 Nomo.app 根、bundle 标识、版本、arm64 主程序以及 codesign 完整性。

##### Scenario: 包结构或签名无效
- **WHEN** 任一解压或复核检查失败
- **THEN** 清理 staging，不显示“重启并安装”，原应用保持不变

#### Requirement: macOS 退出后替换与重启
The system SHALL 复用正常文档关闭流程，并由更新缓存中的辅助可执行文件等待旧进程退出、在同卷复核并替换 bundle、重新启动新版本；替换或启动失败时尽量恢复旧版。

##### Scenario: 用户取消未保存文档关闭
- **WHEN** 任一 Workspace 取消退出
- **THEN** 不启动辅助替换，已验证更新包保留供稍后重试

##### Scenario: 正常更新
- **WHEN** 所有窗口正常关闭且旧进程退出
- **THEN** 新版替换原 `Nomo.app` 并启动，应用数据和文档不被清除，新版启动显示既有更新成功界面

##### Scenario: 旧进程未及时退出或替换失败
- **WHEN** 等待超过 60 秒或替换任一步失败
- **THEN** 不留下半替换的正式路径；可恢复时重新打开旧版并报告失败

### MODIFIED
#### Requirement: Windows NSIS 应用内更新
Windows 既有注册表身份判断、128 MiB 单包限制、SHA-256、缓存租约、取消、正常退出、NSIS PID 等待和重启行为 SHALL 保持不变；仅下载传输增加官方优先的镜像竞速。

##### Scenario: Windows 官方线路正常
- **WHEN** 官方安装包正常下载
- **THEN** 用户流程和安装结果与现有版本一致，且无额外镜像流量

#### Requirement: 更新缓存生命周期
更新缓存 SHALL 继续使用有界 `native-updates` 和跨进程租约；新增的每源临时文件、macOS staging 和辅助文件必须具有取消、失败、成功或超过 24 小时启动清理路径，不得持有旧窗口回调或无限增长。

##### Scenario: 另一进程正在更新
- **WHEN** 缓存租约由另一进程持有
- **THEN** 当前进程不删除其临时文件或 staging，并提示稍后重试
