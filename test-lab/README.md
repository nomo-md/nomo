# Nomo 原生编辑与性能测试工具

本工具将 `8f91ee9944f58c10c216048334e08679b9a4c6bd` 中的 test-lab 迁移到 Rust/GPUI。保留样本导入、文件校验、Windows 进程采样、真实键盘、统计、基线约束和 HTML/JSON/CSV 报告；构建与控制改为原生实现。Node.js 仅作为测试运行器，不参与产品构建，没有恢复 pnpm、Tauri 或 WebView。

需要 Windows、Node.js 22+、PowerShell 7、Rust/MSVC。测试会打开独立可见窗口，真实键盘用例需要保持该窗口前台；后台采样器隐藏启动。输入或截图前检查前台 PID，失焦明确失败，不向其他窗口发送按键。

## 使用

```powershell
node scripts/automation/run.mjs --help
node scripts/automation/run.mjs import --source "D:\固定测试集" --name main
node scripts/automation/run.mjs build
node scripts/automation/run.mjs --config test-lab/suites/quick.json --no-build
node scripts/automation/run.mjs --config test-lab/suites/release.json --no-build
node scripts/automation/run.mjs opening --config test-lab/suites/release.json --no-build
node scripts/automation/run.mjs --suite corpus --dataset main --no-build
node scripts/automation/run.mjs --suite soak --soak-minutes 30 --no-build
node --test test-lab/runner/contracts.test.mjs
```

`release.json` 指向本次从原始 ZIP 校验恢复的 `original` 数据集：空白、5 MiB 纯文本、1 MiB 混合、图片瀑布流、Mermaid、代码和四标签，每场景三轮，每轮 20 次生命周期循环。`pilot.json` 是少量轮次的运行器验证，不是正式性能结果。可用 `--dataset`、`--case`、`--suite`、`--cycles`、`--repetitions` 覆盖配置；`--list` 与 `--dry-run` 不启动应用。

构建使用根 Cargo.lock 和 target，命令相当于 `cargo build --locked --release --bin nomo --features automation`。运行器从根 target 复制独立 EXE 并保存源码/二进制指纹；`--no-build` 拒绝源码不匹配的旧构建。普通发布仍使用不带 `automation` 的原有构建命令，切勿发布 `test-lab/apps` 中的测试程序。

数据与产物：

- `fixtures/<名称>/`：不可变样本及资源；导入拒绝覆盖、符号链接和哈希不符。每个用例复制到自己的 fixtures，不修改原始测试集。
- `apps/<构建ID>/`：独立测试 EXE、源码与二进制指纹。
- `results/<运行ID>/`：完整报告、coverage.json、逐步 RPC 日志、进程 JSONL/CSV、独立 profile 和文档副本；失败截图是辅助证据。
- `baselines/`：显式固定的合格基线，禁止自动覆盖。
- `.cache/`：构建日志。

这些目录均忽略提交。来源包含测试清单时，正文、编码和图片都按原始 SHA-256 校验，不因原文件被编辑而重新生成哈希掩盖差异。

## 原生控制协议

协议版本 3，本地文件 RPC；没有监听端口。只有显式 `automation` feature 和运行器注入的参数同时存在时才启用。控制器验证随机运行身份、PID、独立 `NOMO_DATA_DIR`、fixtures 真实路径和协议版本；打开与保存均拒绝目录外路径，包括父目录与链接绕过。

请求最多 512 KiB，文本/局部读取最多约 64 KiB（读取可向外对齐 UTF-8 字符边界），响应最多 2 MiB。一个请求在途；传输超时后停止发送后续请求，拒绝迟到响应串线。全文摘要固定修订后分块计算，成功或失败均释放快照；不保存另一份全文。IO 与索引查询在后台，UI 命令走正式 Workspace/DocumentHost 入口。控制循环只持弱窗口引用，关闭窗口即停止。普通发布版不包含控制循环、诊断投影及 token 接口。

已接入：

| 接口 | 原生行为 |
| --- | --- |
| open / new / close / switch / mode | 正式文档加载、关闭、标签和单栏/源码/双栏切换 |
| select / reveal / outlineNavigate | 全局字节选区、原生导航和过期修订检查 |
| insert / command / undo / redo | 正式编辑事务、行内/块/表格命令和主会话历史 |
| save / autosave | 正式保存流程，支持新文件指定 fixtures 内路径 |
| search / searchCancel / replaceAll | 原生搜索与替换入口，报告实际命中数和错误 |
| digest / readRange / read | 固定修订全文 SHA-256、有界原文读取 |
| progress / outline | 同一原生结构索引及分页游标 |
| diagnostics / telemetry | 实际标签、会话、快照、挂载视图、阅读锚点、可见区状态和有界性能样本 |
| focus / scroll / exit | 测试窗口聚焦、实际滚动容器、保存/关闭后的正常退出 |

`wait` 在运行器实现，分别判断 readable、editable、stable、indexed、resources。stable 检查可见区布局、待提交编辑和引用加载，再观察短时间内锚点/几何不变；resources 只表示已请求资源完成，绝不声称离屏全文已渲染。它不是“等待两帧就通过”。动作返回 accepted 仅表示已经派发，运行器还等待对应原生完成条件，正文与持久化另做独立断言。

旧 DOM 选择器、浏览器堆、CDP 和 rAF 已移除。阅读位置改为原生正文 byte/fraction 和滚动几何。旧版组合标签、恢复确认、系统剪贴板、目录树专项及搜索末页的完整桌面断言仍需另行覆盖，不列为当前已通过项目；未知方法明确报错。样本增强节点计数不冒充完整渲染断言，coverage.json 保留未验证的清单项。导出、安装更新、TXT/JSON 和完整设置回归不在本测试集范围。

`--suite resources --case resources.lifecycle --dataset original --repetitions 1 --no-build` 验证两个文档共享媒体、后台标签保留、双栏切换、撤销重做和保存、最后一个媒体文档关闭后的缓存归零及再次加载。此用例独立于性能流程，不能替代完整的跨窗口、中文输入法或视觉验收。

## 指标与边界

`opening` 是独立的打开体验采样，复用现有七个性能场景、固定样本和正式打开入口。各轮新进程先显示轻量等待文档，再开始“发出打开请求 → 前台桌面首屏文字可见”和“发出打开请求 → 首屏完整且可交互”的同起点计时；四标签场景测前三份已打开时的第四份。真实输入只修改隔离副本的会话，随后撤销并校验原文与磁盘摘要。运行期间无须人工观察，完成后读取 `results/opening-*/summary.json` 或简短 `report.md`。

首屏可见按最终首部文字的桌面像素匹配判定；空白文档另要求目标身份及可读状态。完整且可交互取“首屏画面完整”与“已观察到可编辑、首屏资源成功就绪”同时成立的首个采样画面。随后确认画面/状态稳定，再以一次真实 Q 按键的正文和桌面回显验证；额外验证等待不计入主列，`inputEchoMs` 单独记录验证完成时间。两项仍包含 RPC 控制和离散采样误差，是观测上界；不包含进程启动或文件选择，不代表离屏全文加载完成，也不是显示器光学测量。

自然首屏的参考画面在任何按键验证准备（选区、聚焦）之前固定；自动等待实际正文区域的像素稳定至少 150 ms 并复核资源状态，避免固定延时不足或测试动作影响完成时点。采样器只在打开阶段运行，名义间隔 25 ms，客户区缩至四分之一宽高后保留灰度像素；记录实际采样间隔、捕获成本、跨进程时钟误差和观测区间。前台、位置、尺寸或遮挡检查失败即判该轮无效。像素暂存上限 128 MiB／45 s，成功后只保留五张关键帧和时间线，关闭时释放线程、位图、文件及进程。没有新增产品常驻任务、正文副本或生产依赖；自动化诊断仅增加实际视口坐标。三轮只汇总最小/中位/最大，不输出三样本 P95。

- 主采样默认 500 ms：CPU、私有提交、工作集和 IO。独立低频采样每 5 s 查询私有工作集与 GPU，超过 15 s 标记不可用；保留开销、间隔与覆盖率。
- 私有提交、私有工作集、原生布局/位图估算和 GPU 是不同口径，不相加。原生视图估算不是堆分配器的实际内存。周期后增长只提示需调查，不直接判定泄漏。
- 原生帧指标是 Workspace render 至 CPU 绘制结束，输入指标是应用收到按键/文本事件至随后 CPU 绘制结束，不是物理输入延迟、GPU 时间或硬件 FPS。每类保留最近 16,384 个样本；不满 20 个不输出可用 P95。
- RPC 与 settled 操作耗时包含文件传输、25 ms 控制轮询和等待判断，不等于精确的 UI 呈现延迟。进程采样与操作统计覆盖完整日志；三轮间汇总只展示最小/中位/最大，不制造三样本 P95。
- 记录实际窗口逻辑尺寸、缩放、明暗外观、GPUI、系统、硬件、配置、源码及 EXE 指纹。各用例为新进程和独立数据目录；没有清空系统文件缓存，不称作操作系统冷缓存测量。
- 指标来自带测试接口的 Release。控制轮询和启用的采样有增量成本，不能把绝对值直接视为无插桩发布版占用；回归比较必须使用相同测试构建方式和配置。
- 真实中文 IME 候选/组词、DPI 切换、连续视觉闪烁、跨平台与长期稳定性仍需单独验收。Unicode SendInput 不算中文输入法验收；API 编辑也不算真人逐键写作测试。

```powershell
node scripts/automation/run.mjs baseline --run <通过且采样有效的运行ID> --name native-release-reference
node scripts/automation/run.mjs --config test-lab/suites/release.json --baseline native-release-reference --no-build
```

没有显式基线时结果为 no-baseline。功能、采样质量、性能告警分开报告；缺失数据不是零。只有功能通过、轮次完整、采样有效且必需指标完整的非诊断 Release 才允许固定基线。数据集、环境、运行器和测量配置不一致拒绝比较；旧 WebView 结果与本协议不可直接比较。

故障隔离自检：`--suite selftest` 应返回 1 且后续独立用例继续；`--suite selftest-cleanup` 应返回 1 且后续哨兵 not-run。清理只处理本次运行器启动的进程，必须确认退出；失败停止后续用例。`clean --run <ID>` 仅删除明确指定、未被基线引用且经过真实路径核验的单次结果。
