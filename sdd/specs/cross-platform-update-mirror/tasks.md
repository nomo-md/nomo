# Task Breakdown & Execution Board: 跨平台安装包镜像竞速与 macOS 自动更新 (Tasks)

> 按依赖顺序实施，完成一项立即勾选；真实桌面安装须另行获得用户授权，不能用构建结果代替。

## Phase 1: 共享模型与平台边界
- [x] Task 1.1: 将安装器和安装 UI 扩展为 Windows/macOS 编译，提取共享 Candidate、来源、进度与平台适配接口，保持当前 Windows 行为不变。
- [x] Task 1.2: 实现官方 Release API 失败或超时后的 gh-proxy 元数据回退，将资产大小和 SHA-256 字段开放给 macOS，统一官方 URL、版本、资产唯一性和代理 URL 白名单校验。

## Phase 2: 官方优先的镜像竞速
- [x] Task 2.1: 实现官方优先、失败或停滞时启动 gh-proxy 的双路编排，使用独立临时文件和共享取消。
- [x] Task 2.2: 实现首个完整大小/SHA-256 校验成功者获胜、另一路取消清理及双失败错误汇总。
- [x] Task 2.3: 将双路尝试合并为单调单任务百分比，接入现有标题栏、设置页、弹窗和窗口所有者生命周期。

## Phase 3: macOS 适配与安装
- [x] Task 3.1: 实现正式 `.app` 身份、稳定可写位置和不支持原因检测，并接入 macOS 更新详情路由。
- [x] Task 3.2: 实现 `.app.tar.gz` 有界下载、安全解压、bundle/plist/arm64/codesign 复核及 staging 清理。
- [x] Task 3.3: 实现复制到缓存的退出后辅助命令、PID 等待、同卷替换、失败回滚、重启和残留清理。
- [x] Task 3.4: 将 `QuitReason::Install`、启动辅助命令和更新成功提示接入 macOS，复用所有 Workspace 的正常关闭与取消。

## Phase 4: 验证与文档
- [x] Task 4.1: 为元数据回退、代理 URL、重试/停滞状态、胜者裁决、进度合并、macOS 路径与包身份等关键纯逻辑补充既有 Rust 测试入口中的定向测试。
- [x] Task 4.2: 运行格式化、`cargo check --locked --bin nomo`、默认与开发入口定向测试及 macOS Release 打包/校验；可用时执行 Windows 交叉检查。
- [ ] Task 4.3: 经用户授权后进行 Windows 与 macOS 真实旧版到新版更新验收，覆盖官方正常、镜像接管、取消、未保存文档取消、权限不足和失败恢复。
- [x] Task 4.4: 更新 `CLASSMAP.md`、`docs/native-build.md`、必要本地化与规格任务状态，清理调试残留并核对最终 diff。

# Task Dependencies
- [Task 1.2] depends on [Task 1.1]
- [Task 2.1] depends on [Task 1.1] and [Task 1.2]
- [Task 2.2] depends on [Task 2.1]
- [Task 2.3] depends on [Task 2.1]
- [Task 2.3] and [Task 2.2] can run in parallel
- [Task 3.1] depends on [Task 1.1]
- [Task 3.2] depends on [Task 3.1] and [Task 2.2]
- [Task 3.3] depends on [Task 3.2]
- [Task 3.4] depends on [Task 3.3]
- [Task 4.1] depends on [Task 2.2], [Task 2.3] and [Task 3.4]
- [Task 4.2] depends on [Task 4.1]
- [Task 4.3] depends on [Task 4.2] and explicit desktop authorization
- [Task 4.4] depends on all completed implementation and verification tasks
