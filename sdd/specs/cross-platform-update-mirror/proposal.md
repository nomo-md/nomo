# Proposal: 跨平台安装包镜像竞速与 macOS 自动更新 (Proposal)

## 1. Context & Problem Statement
- **Current State**: Windows NSIS 安装版已支持应用内下载、SHA-256 校验、取消、退出后覆盖安装和重启；Windows 便携版与 macOS 仍依赖浏览器手动下载。所有安装包均直连 GitHub，没有镜像回退。
- **Pain Points**: GitHub 线路异常时 Windows 更新容易失败；macOS 缺少与 Windows 一致的下载、校验和重启安装体验。若两个平台分别实现完整更新逻辑，会产生重复状态机、进度行为和校验规则。

## 2. Value Proposition
- Release 元数据和安装包均采用官方优先；官方元数据失败或超时时通过 gh-proxy 回退，安装包则在失败或停滞时启动镜像竞速。本次不引入独立更新签名，代理元数据模式明确信任 gh-proxy。
- 官方线路正常时不增加下载流量；异常时才启动镜像竞速。
- macOS 复用现有 `.app.tar.gz` 和用户确认流程，无需等待 Apple Developer ID 或公证。
- 共用候选包、下载竞速、校验、进度和取消逻辑，平台代码只处理安装形态与安装动作。

## 3. Alternatives Considered
- **Option A**: 每次同时完整下载官方源和镜像。(Cons: 正常网络也增加约一倍流量与临时磁盘占用，可能争用带宽，因此拒绝。)
- **Option B**: 只在官方下载彻底失败后串行重试镜像。(Cons: 实现简单，但遇到长时间停滞时需要等待完整超时，因此不采用为唯一策略。)
- **Option C**: macOS 直接接入 Sparkle。(Cons: 需要额外框架、appcast 和更新签名接入，且与现有 Windows 自定义界面及 Release 校验链路差异较大，本次不采用。)
- **Option D**: 官方优先，异常时保留官方请求并启动 gh-proxy，首个完整校验成功包生效。(Selected: 控制正常路径成本，同时缩短异常线路等待。)

## 4. Success Metrics
- [ ] Windows 官方下载正常时不会产生 gh-proxy 请求。
- [ ] 官方连接失败或停滞时，镜像能够接管；首个通过完整校验的包进入现有安装流程。
- [ ] Windows NSIS 更新既有行为无回归。
- [ ] macOS 正式安装版可在应用内完成下载、校验、正常退出、替换和重启。
- [ ] macOS 不支持的位置或权限不足时不会修改原应用，并回退到手动下载。
- [ ] 两平台取消、错误、缓存清理与窗口关闭行为一致且有界。
