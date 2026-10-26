# Nomo 编辑与性能测试工具

首版在 Windows 上运行真实 Release Nomo，依赖本机 Node.js 22+、PowerShell 7；构建另需仓库原有 pnpm、Rust/MSVC。无需 AI 控制电脑。真实键盘用例会短暂占用测试窗口前台，失焦时明确失败。测试程序通过运行器启动，由运行器创建隔离目录并注入接口参数。

## 开始使用

```powershell
pnpm test:desktop --help
pnpm test:desktop --list
pnpm test:desktop --dry-run --suite standard
pnpm test:desktop --config test-lab/suites/quick.json
pnpm test:desktop build
pnpm test:desktop --suite standard --no-build
pnpm test:desktop --suite editing,search --no-build
pnpm test:desktop --suite stress --no-build
pnpm test:desktop --suite soak --soak-minutes 30 --no-build
pnpm test:desktop --suite interaction --cycles 2 --repetitions 3 --no-build
pnpm test:desktop --suite diagnostics --case editing.commands --profile cpu --no-build
```

没有 `--no-build` 时，缺少构建或源码指纹改变会先构建；`--no-build` 必须与当前源码匹配，不会静默测试旧程序。Debug 使用 `--debug`，不参与 Release 基线比较。构建缓存、前端产物均在本目录 `.cache/`，不覆盖普通 `dist` 或 `src-tauri/target`。

`--suite` 取并集，`--case` 与 `--tag` 在该集合内筛选；多个标签要求全部匹配。未知名称、零匹配、无效配置在启动程序前失败。测试逐个执行，失败保存证据并重置后继续；`--fail-fast` 可改为首次失败停止。

## 目录与数据

- `runner/`：运行器、Windows 采样和输入脚本、协议验证、报告生成。
- `suites/default.json`：默认运行参数。`--config 文件` 可覆盖；命令行优先。
- `fixtures/main/`：从用户指定的 2026-10-22 原始测试集导入，77 个真实样本、256 文件目录负载、96 个资源；说明文件不算用例。
- `apps/<构建ID>/`：独立测试程序、必要资源、源码与二进制指纹。
- `results/<运行ID>/`：HTML、JSON、CSV、覆盖清单，以及每个用例的文档副本、独立 profile、进程采样、日志、截图。测试配置、会话、原生缓存和 WebView 用户数据均在该用例 profile 内。
- `baselines/`：显式选定的基线，禁止自动覆盖。

大型样本、程序和结果不提交 Git。每次只改沙箱副本；共享资源保持原相对目录。测试所需原始图鉴已按 ZIP 的原始字节恢复，恢复前备份保存在 `results/restore-*`；额外未登记图片未删除。

导入其他样本目录：

```powershell
pnpm test:desktop import --source "D:\我的测试文档" --name mydocs
pnpm test:desktop --dataset mydocs --suite corpus
```

导入保留目录及图片资源，不跟随符号链接。有 manifest 时逐项验证哈希；没有时自动生成。已有数据集不可覆盖，使用新名称保留来源版本。

## 测试集与范围

`smoke`、`editing`、`structure`、`search`、`render`、`lifecycle`、`persistence`、`compat`、`errors` 可分别选择。`corpus` 映射全部样本；`stress` 为高压及极限样本。`standard` 包含常规、中压、兼容、异常样本、专项功能和代表性性能场景；性能默认三轮、生命周期循环 20 次。

样本场景不只打开文件：验证索引终态、模式往返全文摘要、资源完成、结构身份、首尾定位、编辑、撤销重做、保存及重开；兼容样本额外做编码和换行字节校验。格式和块命令校验准确 Markdown 与未选中文本，正常样本还检查可见图片加载失败。资源按 manifest 完整复制，不依赖目录名或正文编码。清单中的结构期望仅在对应断言已实现时算验证；`coverage.json` 记录映射，`case.json` 列出实际成功断言。

原生索引明确标记 `metricsComplete=false` 且增强节点计数不足时，该计数期望记为不可用，并保留在未验证清单；不会把未完成的统计当作零节点，也不会声称该项已验证。全文完整性、保存和其他可用断言仍独立执行。

`blocked` 表示应用明确降级为源码显示，不能计作完整语义通过；此类样本仍继续源码编辑、撤销重做与保存校验，并保留降级截图。功能失败、超时或 blocked 都让整次运行返回非零。256 文件目录是负载夹具，目录用例验证代表性的跨目录打开，不声称逐个编辑了全部 256 个文件。

专项功能包括选区格式、块级标题／列表／引用、表格行列与对齐、大纲定位与过期修订拒绝、粘贴、真实键盘及快捷键、四标签的选区和阅读位置、组合拆分、搜索分页和替换、正常重启、崩溃恢复、外部变更、自动保存。TOC 样本保存时允许派生目录更新，但仍逐字节校验目录外正文。缺失图片既检查失败图片，也检查应用错误占位。

`interaction` 单独提供持续语义输入、5 MiB 文档真实输入和连续滚动，默认各三轮；输入场景要求足够样本才能给出 P95。`soak` 默认 30 分钟，可配置，交替覆盖源码编辑撤销、语义编辑、真实输入和滚动。二者会使用 Windows 键盘输入，运行期间需保持测试窗口前台，失焦直接失败。`diagnostics` 必须指定用例，`--profile cpu|heap|both` 保存诊断资料，不计作正常性能基线。`suites/` 提供 quick、stress、soak、diagnostics 配置示例，可复制后通过 `caseIds` 组成自己的测试集。quick 只调用应用接口，不发送系统键盘输入。

中文输入法候选与组词、肉眼视觉验收是人工待验项目；程序插入中文与截图不算通过。TXT/JSON、导出、安装更新及完整设置回归不在首版范围。

## 指标与判定

- 500 ms 主采样使用 Windows 原生进程 API，记录 CPU、私有提交、工作集和 I/O；私有工作集、进程角色和 GPU 由独立任务每 5 秒查询。低频结果超过 15 秒则标记不可用，单独记录查询耗时与数据年龄，避免阻塞主采样。首次角色未知时明确标为 `webview-unclassified`。
- CPU `cpuCorePercent` 按单逻辑处理器计，可能超过 100%；`cpuMachinePercent` 除以逻辑处理器数。缺失字段为 null 或 unavailable，不填零。
- `process-*.jsonl` 是原始时间序列；`process.csv` 可自行分析。采样记录自身耗时；采样峰值可能遗漏短瞬时峰值。
- 应用内记录可读、可编辑、稳定、索引完成、资源队列、视图数量、驻留窗口字节、缓存、长任务、rAF 帧间隔、输入到下一呈现帧估计；CDP 可用时记录 JS 堆。
- 记录实际 WebView2 可执行文件版本、窗口尺寸、DPI、主题、硬件和运行器版本。每个用例是新进程和独立 profile；不清理 Windows 文件缓存，不能当作操作系统冷缓存测量。
- 私有提交、工作集、JS 堆、GPU 是不同口径，不相加。循环后增长只是一条诊断线索，不能直接宣称泄漏。
- 默认三轮展示原始值、最小值、中位数和最大值。至少 20/100 个样本才显示 P95/P99。应用内直方图超过 1000 ms 单列溢出计数。进程与操作分位统计覆盖全部时间段，使用固定 4096 桶直方图，误差上界为值的 1% 加 0.01 单位；不截取前几千个样本。原始数据完整保留。
- 长时间运行的操作明细完整写入 `steps.jsonl`；内存中供查看的明细只保留最近 4096～8192 条，但统计重新流式读取全部日志，覆盖早期和后期。按方法及阶段分别统计，避免把所有 wait 阶段混成一种响应时间。
- `last-step.json` 保留当前或最后操作的阶段和有界参数；持续测试每分钟输出进度。周期快照完整写入 `measurements.jsonl`，汇总保留初始值及最近最多 255 次，预热后的稳定占用变化与启动增长分开记录。
- 编辑器节点替换属于性能诊断：虚拟窗口可能重分组，不能仅凭节点 ID 改变判定功能失败或视觉闪烁。标签阅读位置在相同视图模式下比较首个可见文本块及其相对视口位置（容差 2 CSS px）；虚拟高度变化时不要求绝对 scrollTop 不变。
- rAF 不代表硬件合成 FPS；应用事件到帧的时间不是物理键盘到屏幕延迟。CPU profile、堆快照独立运行，不额外调用 `collectGarbage`；堆快照诊断不能作为正常运行占用基线。

固定基线：

```powershell
pnpm test:desktop baseline --run <通过的Release运行ID> --name release-reference
pnpm test:desktop --suite performance --baseline release-reference --no-build
```

只有功能通过、主采样质量有效、必需指标完整且非深度诊断的 Release 运行能成为基线。默认至少 3 个有效进程样本、2 个 CPU 样本，主采样平均耗时不超过间隔的 20%，覆盖率不低于 60%，最大间隔不超过目标的 6 倍；通过 `samplingQuality` 配置。采样器就绪后才开始用例。功能通过但数据质量无效返回退出码 2，报告分别显示功能、数据质量和性能状态；不能固定成基线。

样本、环境、运行器版本或采样参数不一致则不可比较。比较覆盖启动、业务工作负载耗时、私有提交峰值、CPU、输入 P95、帧间隔 P95、长任务频率/耗时、内存保留量及全时段增长斜率。准备夹具和清理时间保留为总耗时，不参与工作负载耗时比较。缺失指标列为 partial/unavailable，不当作零或全部通过。无基线也按 `budgets` 给出可配置的诊断告警；阈值不是永久内存上限或泄漏结论。相对退化需同时满足 `warnings` 的相对和绝对门槛。

每次结果完整保留。明确清理单次非基线运行：`pnpm test:desktop clean --run <ID>`。清理拒绝越界路径、符号链接、被基线引用的结果和不属于本工具的目录。

## 接口维护与自检

协议版本 2；本地文件 RPC 无监听端口。Rust `automation` feature 与前端显式构建开关共同隔离；CDP 只在测试进程开放动态 loopback 端口。主窗口、测试应用标识、fixture 路径均校验。大文档摘要在 Rust 流式计算；`readRange` 最大 64 KiB。

Windows 启动时给子进程注入独立 `WEBVIEW2_USER_DATA_FOLDER`，并读取本次 browser 子进程的实际命令行校验目录、读取可执行文件版本；不修改系统环境变量。无法验证或目录越界会失败。首次 GPU/WMI 查询可能慢于短用例，缺失进程采样会明确记录，版本识别不依赖采样完成。

常用方法：`open/new/close/switch/mode`、`select/insert/paste/command`、`undo/redo`、`search/searchCancel/replaceAll`、`outline/outlineNavigate/reveal`、`save/autosave/checkpoint/exit`、`externalCheck/reload`、`combine/ungroup/folder`、`wait/diagnostics/telemetry/digest/readRange`。`select` 是全局源码选区，表格结构命令先用 `reveal` 放置语义光标。`wait` 明确区分 readable、editable、stable、indexed、resources，不将两帧等待当成全部完成。当前 renderer 队列完成及可见图片完成不代表全文离屏资源都已渲染。

```powershell
node --test test-lab/runner/contracts.test.mjs
pnpm test:desktop --suite selftest --no-build
```

`selftest` 故意让第一个用例失败，第二个应继续通过；总退出码应为 1，用来验证故障隔离。正常退出通过应用退出流程验证；测试清理只处理本次启动的进程树。

`pnpm test:desktop --suite selftest-cleanup --no-build` 在实际清理后注入失败，退出码应为 1，后续哨兵必须保持 not-run。正常运行中，无法确认应用、采样器或已登记子进程退出时立即停止后续用例。子进程验证同时核对 PID 与创建时间，避免 PID 复用误判。
