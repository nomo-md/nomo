# Nomo 原生页面与 NomoMark 接入 Roadmap

编辑控件现统一命名为 **NomoMark**，核心类型为 `NomoMark`，模块为 `nomo_mark`。当前入口和验证命令见[开发记录](../docs/nomo-mark.md)；下方更名前的核查与验证记录保留当时的类型名、feature 和测试过滤名。

## 当前 Goal 与证据边界

- 目标：在 `gpui-editor` 当前未提交迁移成果上，将已有窗口化 `NomoMark` 接入默认 Workspace，并复刻主分支正式页面及对应功能。普通启动、新建、打开均使用正式入口；第一批闭环完成后继续页面及交互对齐。
- 固定对照：`origin/master` 的 `339fb932db064508af4e471a0fda8f44468d0157`。2026-10-26 已通过 `git ls-remote --symref origin HEAD refs/heads/master` 核实远端默认分支为 `master`，本地引用与远端相同。本轮始终以此 commit 为基准。
- 起始工作分支：`gpui-editor`，HEAD `98e9376e9a186177784d3e8fca973cd6321b0524`。已有原生迁移、大量旧前端删除和未跟踪文件均为本轮前的用户成果，禁止覆盖或还原。
- 2026-10-26 起始源码核查：`main.rs` 默认启动 `Workspace`；`workspace.rs` 每个 Tab 持有 `Buffer + semantic EditorView + source EditorView`；`NomoMark` 的真实文件宿主为 feature 下的 `demo.rs`。这是本轮核查事实，不能据旧文档把“尚未接入”排除在目标之外。
- 正文、历史、全局修订及坐标由唯一文档会话管理；控件仅接收窗口片段。必须复用连续窗口、视口调度、预取、回收、字节预算、取消及过期结果门禁；不引入完整 Buffer 镜像或全文解析渲染。
- 主代理统一负责 Workspace、文档状态和编辑器接入；子任务仅修改明确授权的独立文件。无自动提交、推送、合并、PR、发布或安装。
- 状态分为：**待实现、已实现未验证、构建通过、行为验证通过、视觉对照通过、阻塞**。状态只针对所述子项；构建与局部验证不等于整行或整个 Goal 完成。历史 `docs/archive` 不作为当前验收证据。

## 页面与交互对照清单

下表基准路径均属于固定 master commit，通过 `git show <commit>:<path>` 读取；当前入口相对仓库根目录。基准源码证据不等于已运行参考版或完成视觉验收。

| 区域 | 固定主分支入口与关键行为 | 当前 GPUI 入口 | 起始差距、依赖与验收方式 | 状态 |
|---|---|---|---|---|
| 默认入口与文档会话 | `src/app/App.svelte`、`components/AppShell.svelte`：新建、打开、恢复后进入同一正式外壳 | `src-gpui/src/main.rs`、`ui/workspace.rs`；控件宿主 `nomo_mark/document_host.rs`、`native_port.rs`、`session_window.rs` | 正式页仍使用旧 Buffer；将宿主业务能力与开发界面拆开后复用。验收普通启动、命令行/系统打开、新建与多文件打开均进入窗口化控件，无开发参数 | 基础行为验证通过；恢复/系统打开待验 |
| 标题栏与菜单 | `components/AppTitleBar.svelte`、`WindowsCaptionControls.svelte`；`styles/app-responsive.css:75`：独立 42px 标题栏，Windows 应用内菜单与窗口按钮，侧栏、主题、mini、更新入口；macOS 留 88px 红绿灯区 | `ui/chrome.rs`、`ui/workspace.rs::render_tabs`、`main.rs::set_menus/open_workspace` | 当前 38px 标签条充当标题栏且无条件左留 84px，未复刻 Windows 应用内菜单/窗口按钮。需分离标题栏与标签栏；逐项核对菜单实际动作、快捷键、禁用项、拖动/双击/右键及最小窗口菜单边界 | 构建通过；部分菜单待接，行为/视觉待验 |
| 标签栏 | `components/DocumentTabs.svelte`、`styles/app-chrome.css:231`：位于编辑区上方；30px 标签、普通宽 120–180px、12px 上圆角、14px 字；脏点、悬停关闭、预览斜体、溢出菜单、批量关闭、复制路径/定位、拖动 | `ui/workspace.rs::render_tabs/activate/close_tab` | 基本切换/关闭已有但未接新会话；条带层级、尺寸、脏点悬停、溢出/预览/右键/拖动缺失。验收长文件名、20 标签、未保存关闭、取消关闭、切换后历史/选区/阅读位置 | 基础切换/关闭行为通过；其余待实现/验证 |
| 空白及启动加载 | 文档主页采用已确认的新布局；目录工作区保留空文档引导 | `ui/home.rs`、`home/library.rs`、`home/thumbnail.rs`、`ui/workspace.rs` | 已实现无文件启动进入主页、关闭最后标签保留目录工作区、文件夹入口、最近记录移除、页首预览和返回编辑；完整工作区恢复仍待实现 | 主页已实现，实机待验 |
| 文件侧栏 | `components/ExplorerSidebar.svelte`、`styles/app-layout.css`：可拖宽、虚拟列表、预览/固定、快速搜索、文件管理和刷新 | `ui/file_tree.rs`、`ui/workspace/explorer*.rs` | 已接根目录同区滚动、滚动条、层级与标识动效、悬停和菜单、键盘搜索、后台按需扫描与 5 秒刷新；缓存有字节预算，超限和读取失败明确提示。打开的文档及所在目录重命名时同步会话路径；删除仍需先关闭。Windows、macOS、Linux 接入系统文件剪贴板，超长名称显示省略号。需在实际桌面验证滚动、输入法、文件操作、动效和大目录性能 | 已实现；实机待验 |
| 工具栏 | `components/EditorToolbar.svelte`、`styles/app-chrome.css:582`：44px 高、5px 16px 内边距、4px 间距；格式激活态、表格尺寸选择、链接编辑、公式/图表菜单、脚注/目录/emoji、搜索、宽度、三模式/对齐线、大纲、收起 | `ui/workspace.rs::render_toolbar`、`nomo_mark/commands.rs`、`inline_commands.rs`、`menu.rs` | 当前 42px、动作被 overflow_hidden 裁剪、格式态固定 false，多项直接拼文本；需复用控件命令与正确活动视图。补上下标、批注/details、图表/脚注/目录/emoji 等入口及选择器；窄窗动作可横滚，宽度控制按 820px 容器阈值折叠 | 基础命令/宽度构建通过；其余待实现/验证 |
| 编辑区与模式 | `components/EditorWorkspace.svelte`、`MarkdownSourceEditor.svelte`、`FrontMatterCard.svelte`、`styles/editor-document.css`：语义/源码/双栏、正文宽度、滚动/选区同步、front matter、活动对象交互 | `ui/workspace.rs::set_mode`、`nomo_mark/window_panes.rs`；`nomo_mark/view.rs`、`source_view.rs`、`viewport_ui.rs`、对象/表格模块 | 默认编辑区未用控件。必须同会话同历史，按需创建视图，模式转换保持全局坐标和滚动锚；补配置应用和 front matter 等未接宿主能力。验收窗口边界编辑、跨窗口选择、IME、长文档远跳/连续滚动、双栏双向编辑/撤销/保存 | 构建通过；单栏基础行为通过，双栏/视觉待验 |
| 文档大纲 | `components/EditorWorkspace.svelte`、`styles/editor-outline.css:1`、`services/outlineInteractionController.ts`：220px 浮卡、默认 top 168px、14px padding、层级展开/批量折叠、活动标题、卡片拖动及重置、章节拖动重排 | `ui/workspace.rs::render_outline/jump_to_heading`；会话 `text_document/structure_index` | 当前 236px 固定浮卡、28px 行、只按默认层级过滤；无卡片拖动、独立折叠与章节重排。改用完整文档索引/全局跳转，避免当前片段大纲。验收远距离标题、无标题、空标题、滚动高亮、拖动后正文及历史一致 | 600节索引/跳转行为通过；其余待实现/验证 |
| 搜索替换 | `components/SearchReplacePanel.svelte`、`services/searchReplace.ts`：可拖动且持久定位的 520px 面板，区分大小写/全词/反向/循环、统计、替换当前/全部、忙碌/只读/无结果状态 | `ui/workspace.rs::open_search/run_search/step_match/replace_one/replace_all/render_search`；会话 `text_document/markdown_search` | 当前 400px 固定浮卡且只有大小写选项；需接会话全局搜索及结果导航，不局限窗口片段。验收跨窗口命中、替换后选区/历史、取消过期请求、切标签不误写、最小窗口面板可达 | 查询/远跳行为通过；替换/视觉待验 |
| 状态统计与缩放 | `components/StatusBar.svelte`、`styles/editor-outline.css:250`：编辑区右下浮动控件，right 16px/bottom 14px；写作指标菜单、阅读时间、80–160% 缩放弹层、Markdown 检查状态/问题列表/跳转 | `ui/workspace.rs::render_status`、`text_document/writing_stats`、`ui/settings_schema.rs` | 当前全宽 28px 底栏，展示路径/行列/模式/固定 UTF-8/缩放按钮；缺对应浮动外观、指标菜单、检查器。必须统计完整文档或完整选区，区分 pending/error；编码不可硬编码假报。校验中英文/选区/多窗口语料及错误重试 | 全文统计构建通过；其余待实现/验证 |
| 设置窗口 | `components/SettingsWindow.svelte`：8 分区、212px 导航、即时生效；字体校验/建议、主题预览与导入导出、自定义快捷键、文件与窗口、图片策略、检查/插入等 | `ui/settings_window.rs`、`ui/settings_schema.rs`、`settings.rs`、`theme.rs` | 当前 200px 导航并新增独立快捷键页；多个 Support::None 项及整类被隐藏、主题仅默认、语言仅中文。逐项接控件/宿主后恢复基准设置与真实支持范围；不能将旧“隐藏未支持项”视为复刻完成。验收即时应用、持久化、旧 JSON 未知键保留、无效值及重开 | 待实现 |
| 页面弹窗/面板 | `components/UnsavedConfirmDialog.svelte`（392px）、`ExternalChangeDialog.svelte`（560px）、`FolderOpenDialog.svelte`（420px）、`CloseWindowBehaviorDialog.svelte`、`ConfirmDialog.svelte`、链接／Emoji 与更新系列组件 | `ui/dialogs.rs`、`dialogs/secondary.rs`、`toast.rs`、`workspace/application_dialogs.rs`、`tray.rs`、`updates.rs`；控件保留局部对象弹层 | 保存／删除／Toast 及第二阶段外部变更、恢复、打开位置、Windows 关闭策略、更新界面已接通；保留窗口化会话与系统文件选择器。具体实机范围、资源上限及未验证项见 `docs/native-build.md` 的两阶段验收记录；链接／Emoji 等局部编辑面板不计入本次第二阶段验收 | 两阶段已实现；Windows 已列场景通过，其他 DPI／跨平台待验 |
| 对比、窗口及文件相关操作 | `components/ComparisonWorkspace.svelte`、`ComparisonPane.svelte`、`App.svelte::createComparison/resizeComparison`；标签组合成两个文档对比、20–80% 拖动分隔、交换/解除；新窗口、窗口间标签、mini/置顶；`services/exportService.ts` 和 `wordExportService.ts` 真正执行 HTML/PDF/Word 导出 | 当前 Workspace 无对应对比/mini/多窗口管理/导出入口 | 主分支确有功能，不能沿旧 Roadmap 的“另行设计/暂下线”遗漏。与“同一文档源码/语义双栏”分开登记；依赖会话、窗口生命周期、原生导出实现。验收各窗活动文档隔离、关闭/保存、对比交换及导出完整正文与资源 | 待实现 |
| 主题、字体与响应式 | `styles/theme.css`、`app-responsive.css`、`services/themeRegistry.ts`：UI 14/13/12px、控件 28/34/38px、圆角 4/8/12px；Windows 正文 Segoe UI/Microsoft YaHei、等宽 Cascadia Code/JetBrains Mono/Consolas；多配色与深浅色 | `theme.rs`、`settings.rs`、`ui/components.rs`、全部 render 函数 | 当前单一自有配色、正文默认 Microsoft YaHei、等宽 Consolas，不能假定视觉一致。需固定同主题/字体/窗口/DPI 截图比较；大纲窄容器行为、菜单避边、弹层最大高、工具栏横滚都需逐项对齐。透明/模糊等平台差异先调查代价与实际效果再决策 | 待实现 |

主分支存在明确占位的菜单项（例如 `AppTitleBar.svelte` 中 `comingSoon(...)` 的段落前后插入、部分图片/行内公式入口），应复刻其真实禁用/提示行为并与可执行入口区分，不将它们描述成主分支已有完成的功能。图表、导出、对比等上表项目已找到实际回调或服务实现，不能按占位处理。

## 第一批闭环验收

| 验收项 | 必须留下的证据 | 状态 |
|---|---|---|
| 无开发参数启动正式页面 | 默认构建命令、构建版本、启动 PID、隔离配置及窗口截图；中央实际为 NomoMark | 行为验证通过（run-01/02）；本轮新壳待复验 |
| 新建、打开及多标签 | 新建空会话；任务文件打开成功/错误；同一路径复用；切换不新建独立正文或历史 | 基础行为验证通过；新双栏待复验 |
| 编辑、保存、另存为与重开 | 任务文档实机输入，完整文件内容与编码/换行核对，重开一致，保存失败明确反馈且不清脏 | 已列短文档路径通过；失败/双栏待验 |
| 未保存状态与关闭确认 | 编辑/撤销/保存后的脏标识；保存/放弃/取消，保存对话框取消；非活动标签关闭仍作用原文档 | 取消/保存关闭行为通过；其余待验 |
| 撤销重做、焦点、选区、阅读位置 | 输入、格式化、跨窗口编辑可撤销/重做；标签切换后恢复；工具栏操作不丢选区 | 单栏已列场景通过；跨窗/双栏待验 |
| 全文语义与窗口边界 | 长文档全选/复制/删除/替换、远跳、跨片段事务；完整正文仅在会话，控件载入范围和资源仍有界 | 600节远跳/末尾保存通过；其余待验 |
| 源码/语义/双栏与中文 IME | 同会话一致修订；模式往返后内容/选区/历史/滚动一致；组合中切换/滚动不损坏事务 | 构建通过；双栏/真实IME待验 |
| 生命周期与资源 | 隐藏标签暂停加载/渲染；关闭释放窗口/订阅/任务；取消或过期结果不回写关闭页；无 Buffer 镜像 | 构建通过；资源基准及完整释放待验 |
| 当前工具链验证 | 默认 cargo check/release build，保留 feature 的对应检查/构建，按风险定向既有测试；记录原有失败 | 构建通过；定向既有测试通过（详见记录） |

## 后续验收与保留事项

- 按上表逐区执行“核对差距 → 实现 → 验证 → 修复 → 记录”；每区同时验证外观、事件目标和错误状态。代码与默认/开发构建入口必须同步更新直接相关文档，旧入口描述随实际代码变化更新。
- 实机使用任务专用文档及隔离配置，启动前记录已有进程/端口；只管理本任务启动的进程。记录参考版与 GPUI 版本、窗口尺寸、DPI、主题、截图和产物位置。
- 性能按仓库基准：同 Release/GPUI/主题/窗口/DPI/固定媒体，空白、纯文本、四标签、复杂文档各三轮；20 次切换和打开关闭，区分预热与持续增长。私有驻留、私有提交、原生分配、GPU 分别报告；未测项目不计通过。
- 外部文件变更、工作区恢复、多窗口、托盘、国际化、快捷键、图片及复杂块均按主分支页面入口追踪；当前缺口不因隐藏设置而消失。
- 保留三平台构建、打包、输入法、文件打开/保存、升级/卸载及系统预览后续工作；不得恢复旧前端/WebView。系统预览、安装/发布和机器级集成如需新权限另行明确，不自动执行。
- 全部目标有对应构建、行为及视觉证据后才能完成 Goal。局部完成、构建通过、单次性能数字或遇到阻塞均不代表整体完成。

## 本轮实现与验证记录

- 2026-10-26：完成固定主分支页面与交互的只读源码对照，建立以上待实现清单；未启动应用、未截图、未运行构建或测试，不据此报告行为/视觉通过。后续实现记录由主代理追加。

### 2026-10-26 默认入口接入检查点

- **构建通过**：默认 Workspace 已改为 `Tab → DocumentHost → EditorControl`，开发入口共用宿主；普通启动、新建和打开不再创建 Buffer/EditorView。单应用共享会话管理器，每文档独立 owner；全文/历史由会话保存，窗口、预取、资源预算和取消沿用原实现。
- **已实现，部分行为验证通过**：文件打开/新建、异步保存/另存为、保存前快照、关闭确认、标签切换、撤销重做、自动保存、全局分页搜索/替换、索引大纲与全文统计已连接。自动保存、替换、长文档及失败路径尚待专项验证，不能视为全部通过。
- **行为验证通过（短文档）**：无开发参数启动；首页 Ctrl+N；打开任务文件；输入/保存/重开字节一致；撤销重做及保存点脏状态；切标签保留历史/光标；源码快捷键与工具栏状态一致；搜索跳转后继续输入查询不误改正文；取消关闭保留内容；保存后关闭原标签。输入 Unicode 中文只证明文本输入，不等于真实 IME。
- 实机产物：`.artifacts/workspace-goal/run-01/`（PID 14184，已正常退出）、`run-02/`（PID 6844，启动记录和日志）；隔离配置 `.artifacts/workspace-goal/profile/`，只使用 `fixtures/`。截图 `run-02/source-mode.png`。保存文件 `fixtures/new-editor.md` 与隔离配置下保存前快照已逐字核对。运行截图尚未与固定 master 参考版比较。
- **构建通过**：默认 `cargo check --locked --bin nomo`、feature `cargo check --locked --features editor-control-demo --all-targets`、两种 Release build 均通过；最后构建为默认版本。现有定向测试 `editor_control` 65 通过/3 ignored，`text_document::markdown_session` 36 通过；未新增或修改测试。日志见 `.artifacts/workspace-goal/{latest-check,feature-check,control-tests,session-tests,feature-build,release-build}.log`。保留已有警告，ignored 不计通过。
- 修复实机发现：首页焦点无法 Ctrl+N、模式动作被子控件截获导致状态不一致、搜索导航抢走查询焦点。修复审查发现：后台查询过期回写、跨文档统计取消串扰、隐藏页查询缓存驻留、挂起事务重试入口、全选长度忽略尚未确认修改。
- **待实现/未验证**：同会话双栏目前明确提示未接通；真实 IME、窗口边界、长文档资源/连续滚动、全文复杂命令、超大单结构/native_region、4 MiB 大纲分页、内容宽度/设置应用及主分支视觉逐区对齐。当前无性能收益或泄漏结论，三轮/20 次基准未执行。阶段一与整个 Goal 均未完成。
- 下一步：长文档实机定位与边界修复；调查双栏共享事务和有界视口方案；准备固定主分支隔离参考构建。表格保留起始差距，当前进展以本节明确子项证据为准。

### 2026-10-26 长文档与参考构建进行中

- 任务语料 `fixtures/long-window.md`：600 节、原始 288247 字节，SHA256 `01D5EB93E1542E5156F9C24FA93AE6D483677504B42DCBD110835FBC6567DCEE`。搜索能定位完整文档末尾，全文统计和600项大纲正常返回；末尾修改保存后288260字节与原文加“ 末尾追加”逐字一致。
- **仍在修复，未通过整体验收**：远距离导航及输入/撤销后的滚动位置在布局/结构刷新之间存在回跳。已采集 `run-03/far-jump-before.png`、`run-07/far-jump-after.png`、`run-09/long-edit-stable.png`；中间版本某些操作通过，但后续路径复现，因此不能用这些截图宣布问题全部解决。当前正在核对 GPUI prepaint、clamp、邻窗加载与定位意图结束的时序。
- 固定 master 参考版已成功隔离构建并由主代理核对产物哈希；目录 `.artifacts/workspace-goal/reference/`。页面与业务源保持固定commit，构建隔离仅调整局部Tauri路径、identifier、禁用自动更新。pnpm frozen/前端及Tauri Release通过，详见 `reference/BUILD-RECORD.md`。
- 参考版 PID 11576 已用任务 alpha.md 启动；主页面截图 `reference/logs/alpha-reference.png`，离线更新说明截图 `update-notes.png`。参考尺寸1180×760，当前GPUI1280×820，尚不计同尺寸视觉通过。未安装/发布，未触碰真实文档。
- 双栏只读方案：一个DocumentHost拥有唯一会话，内部两个按需pane共享事务协调，各自有界Viewport/ScrollHandle/高度树；operation_id须包含pane身份，ack须作用于捕获的源Entity；滚动按共同正文锚点映射。仍待实现，不能用两个独立DocumentHost替代。

### 2026-10-26 长文档定位修复复验

- **行为验证通过（所列场景）**：`run-12` 在600节语料上完成首屏搜索末尾、末尾追加、异步结构刷新、撤销、重做、保存；命中/光标始终回到第600节末尾，没有再停到第566节。随后手动向上滚到第593—597节并等待，未被旧光标拉回。截图 `run-12/long-undo-stable.png`、`long-redo-stable.png`、`manual-scroll-stable.png`。
- 保存结果为288270字节，与原始全文加“ 末尾追加 第二次”逐字相等；不是仅保存当前窗口。此前288260字节文件已在新进程重开并看到末尾追加。此次保存后的再次重开、全选跨窗复杂编辑、真实IME及更大语料仍待验。
- 根因与修改：宿主此前没有光标可见性连接；相邻窗口挂载时先使用初始高度，后续真实布局及GPUI clamp会移动内容。现在在滚动容器 `on_children_prepainted` 后按实际几何校正，并等可见窗口采用测量高度、事务确认及窗口加载完成才结束定位；用户滚轮取消旧定位。结构刷新复用边界未变的已测高度。正式/开发外壳共用此逻辑。
- 增量成本：一个定位意图布尔值、只读局部几何查询和现有有界挂载集合检查；无新正文、历史、常驻任务、依赖或独立高度缓存。临时诊断输出已移除。仍未进行三轮/20次资源基准，不声称性能收益或泄漏结论。
- **构建通过**：最终代码的默认check、feature all-targets check、默认/feature Release build均通过；最后构建为默认。定向 `editor_control` 测试65通过、3 ignored。最新日志 `long-check.log`、`long-feature-check.log`、`long-control-tests.log`、`long-feature-build.log`、`long-build.log`。本轮没有新增或修改测试。
- 本次启动的旧GPUI窗口已逐个正常关闭；run-03曾有一个隐藏启动PID20512，核对精确exe路径后终止，之后改用已授权的可见验收窗口。当前保留 `run-12` PID7712（任务长文档已保存）和参考版PID11576（任务alpha.md），日志及启动记录在各自目录。
- **下一轮继续**：同一文档语义/源码切换的阅读位置、双栏协调和窗口边界；参考版同尺寸视觉对照及标题栏/标签栏/工具栏布局。第一批基础接入已有实际运行证据，但阶段一全部边界与整体Goal仍未完成，未提交/推送。

### 2026-10-27 同会话双栏与标题栏接入检查点

- **构建通过，实机未验证**：DocumentHost 新增按需 Pane。双栏各自使用原有连续窗口、预取、回收、滚动及高度树；共享唯一 NativeWindowPort、完整正文、修订和历史。普通工具栏/快捷键/默认模式设置可以进入双栏，不再以提示拒绝。
- 输入始终由一个控件负责，跨栏/跨窗口移交检查未确认事务、IME、对象草稿与修订。Submit 回执捕获提交 Entity；新栏窗口 ID 唯一；后续窗口请求核对 Pane ID、generation、目标 Entity/WindowIdentity 与文档修订。新选择、点击或滚轮取消旧阅读跟随，防止异步回包覆盖新输入位置。
- **已实现，实机未验证**：完整文档字节锚点的模式阅读位置恢复/双栏滚动同步；非活动栏延迟刷新；8px 分隔条、25–75% 比例、方向键/Shift/Home/End；遵循左右顺序配置；单栏内容宽度配置真正作用编辑区，双栏宽度控制不可改变配置。离开双栏 detach 另一栏并释放请求/预算；隐藏标签暂停两栏。
- 审查修复：空文档无结构组也执行输入控件移交；首尾阅读锚点按实际可滚范围完成，不反复 notify；活动栏光标可见性不受跨栏 IME 暂停门禁影响；宿主恢复选区不再伪造用户 SelectionChanged；加载失败显示重试，避免无界自动重载。
- **构建通过，行为/视觉未验证**：独立42px标题栏、Windows菜单/窗口按钮、拖动/双击/右键菜单、菜单键盘导航与焦点恢复；标签条另起一行。既有文件/编辑/查看/基础格式动作、新窗口、1–6级标题/清除格式已接线；关闭继续走 Workspace 保存确认。最近记录、导出、选择器、额外块命令、Mini/更新等未接能力保持显式禁用并继续列为待实现，不能把菜单可见算功能完成。
- 已对齐已支持菜单的默认快捷键：Ctrl+1–6 设置标题、Ctrl+0 正文、Ctrl+Shift+N 新窗口、Alt+Shift+5 删除线、Ctrl+Shift+X 任务列表、Ctrl+Shift+H 分隔线及清除样式等。去掉 Ctrl+1–3 的旧模式绑定冲突，模式改用工具栏或 Ctrl+E；自定义快捷键、未支持命令及跨平台完整映射仍待补齐。此项仅构建验证。
- 增量成本：仅在双栏时增加一套有界控件/布局/高度树与现有组元数据；两个 ResidentLease 登记到原共享源文本/布局预算，非必要预取仍受压力回收约束。每栏沿用32 MiB派生组元数据估算上限，双栏最多两份及对应标量高度树；没有新增全文 Buffer、独立历史、数据库、依赖、永久定时器，也未裁剪撤销历史。尚未运行三轮/20次性能基准，不声明性能或泄漏结论。
- 桌面验收阻塞证据：本轮两次重新绑定并激活旧任务窗口均返回 `failed to activate captured window`；只读检查存在 LockApp 进程，已请用户方便时解锁。未绕过桌面锁定或修改系统。未启动新的验收进程，旧 run-12 PID7712 与参考 PID11576 保留；本轮新代码不借用上一版截图计通过。
- 验证日志：`split-check.log`、`split-feature-check.log`、`split-control-tests.log`、`split-feature-build.log`、`split-build.log`；最终代码默认check、feature all-targets check、默认/feature Release均通过；控件测试65通过、3 ignored。最终产物为默认版本，冻结于 `split-candidate/nomo.exe`，SHA256及构建入口见同目录 `build-record.json`。未新增/修改测试，保留既有警告。
- 下一步：桌面恢复后先验证空文档双栏从两边输入、双向编辑/统一撤销重做/保存重开、600节远跳与连续滚动、双栏/单栏/标签往返和真实IME；同时按固定参考版复验新标题栏及菜单，继续标签/工具栏/侧栏等区域。未提交/推送，整体 Goal 保持进行中。

### 2026-10-27 菜单定位与入场动画修复

- **已实现，桌面未验证**：标题栏和二级菜单、设置下拉、工具栏选择器、链接/表格/Emoji、隐藏标签列表和文件树/大纲右键菜单共用 `ui/popup.rs`。使用本帧触发边界和内容实测尺寸，保留按钮间隔，空间不足时翻转、限高和滚动；二级菜单不再估算标题栏横坐标。长图表菜单的键盘选择会滚动到目标行。
- 工具栏链接浮层改为跟随链接按钮；快捷键打开仍跟随选区。移除翻转和横向避边后会指错位置的固定上箭头。弹层入场统一为 160ms 淡入及 3px 位移；减少动态效果时取消位移并缩短或跳过动画，结束后不再请求动画帧。
- 增量成本：每窗新增一个链接按钮几何记录；弹层沿用原有视图、弱文档引用和订阅释放路径，滚动和动画状态只随弹层保留。无新依赖、全文副本、后台任务、定时器或独立缓存；未运行三轮/20次性能基准，不声明性能收益。
- **验证通过**：`cargo check --locked --bin nomo`、`cargo check --locked --features editor-control-demo --bin nomo`、`cargo build --locked --release --bin nomo`；最终代码保留原有 93 条警告。日志位于本机 `%TEMP%/nomo-popup-fix-20261027/`。没有新增或修改测试；本机未安装 rustfmt，未执行格式检查。未启动应用，位置、点击/键盘行为、动画和不同 DPI 仍待桌面验收。
- 当前默认 Release 产物为根 `target/release/nomo.exe`，SHA256：`A4F3B28EC78FCE66B7C490AC20367A915FA1A8E77E89481BF1F74870A691F255`。本次修复未提交、未推送。

### 2026-10-28 快捷插入与本地图片

- **代码已实现，桌面未验收**：快捷插入从右键／Emoji 补全状态中拆出，仅实际输入在语义区触发；恢复原版 14 类／41 种样式、上下类别／左右变体导航、触发符设置和四语言面板。主面板按 `339fb932` 的尺寸、原始图标、字体层级和状态实现，使用滚动容器外的共享定位／动效。
- 链接、Emoji 和公式／注释草稿在确认时替换触发符，取消保留原文；同步插入走一笔局部事务。插入目标校验包括文档、修订、视图代数、编辑模式、选区与活动栏；语义边界不完整或不支持的容器明确提示，不猜测窗口外正文。
- 本地图片支持多选、未命名文档保存续接、三种目录策略、同内容复用、同名独占创建、取消和部分失败反馈。使用固定 64 KiB 读缓冲，复用既有输入／像素／解码预算和图片缓存；图片路径编码、像素／百分比宽度与对齐可以保存往返。保留旧上传／清理配置，上传、清理及粘贴／拖放不在本轮范围。
- **验证通过**：`cargo check --locked --bin nomo`、`cargo check --locked --features nomo-mark-demo --all-targets`、默认及 `nomo-mark-demo` 的 Release 构建。既有 `nomo_mark` 测试 68 通过／3 ignored，`images::tests` 2 通过；未新增或修改测试。14 类／41 个变体、四语言键集合及静态图标引用检查通过，UTF-8 无 BOM 与最终差异检查通过。最终构建输入在复核期间未再改变。
- 分隔线、图表、块级公式／注释及块级图片保留后续可输入段落；表格落点在首个单元格，代码块落点在正文。图片导入成功／部分失败通过独立状态通知反馈，真实失败继续走原错误通道；窗口失去激活时关闭快捷菜单，选择器过期后恢复活动编辑区焦点。
- 默认产物为 `target/release/nomo.exe`，SHA256：`9F8FB3D734D8CFF84916712C006230DEA9948C652790AE3EA2A27E692EFC06D0`。日志位于本机 `%TEMP%/nomo-quick-insert-104a0278cc704268be821ea3e1879b28/`。默认／开发 Release 分别保留 94／90 条警告；本机缺少 rustfmt 组件，未执行格式检查。
- **待授权实机验收**：未启动或重启应用，未执行截图、真实 IME、深浅主题／不同 DPI、41 种组合交互或三轮／20 次性能对照；上述构建与既有测试不代表这些实机项目已通过。未提交、未推送。

### 2026-10-28 Markdown Mini 小窗

- **Windows 已实现并完成部分实机验收**：标题栏按钮及 `Ctrl+Alt+M` 原地切换单文档小窗；默认 460×560、最小 320×240，默认置顶，控制条按原版悬停／焦点进入时 140ms 淡入。提供修改与外部变化圆点、置顶和返回；返回恢复普通窗口几何、最大化状态与编辑模式。macOS／Linux 和全屏内直接切换暂未接入。
- 保留同一 DocumentHost、窗口片段、共享会话与历史；进入时收回双栏的另一 Pane，返回时按需创建。控制条独立驱动短时动效，退出释放视图、订阅和 Win32 subclass；无全文副本、常驻计时器或新增缓存。直接使用依赖图中已有的 Windows 0.62.2 Shell 接口，切换任务栏按钮时不隐藏／重建窗口。
- **构建与既有测试通过**：默认及 `nomo-mark-demo` 检查、默认 Release 构建；`session_window::tests` 3 项、`viewport_model::tests` 3 项、`resident_budget::tests` 1 项、大纲页数／字节预算 1 项通过。未新增或修改测试。日志保存在 `.artifacts/mini-mode-20261028/`，产物为 `target/release/nomo.exe`；未提交或推送。
- **授权实机修复**：切换后主动同步 GPUI 视口，修复窄窗沿用原宽度；控制条改为捕获鼠标几何进出，避免原生拖动区遮挡悬停；文档顶部保持顶部，中段沿用阅读锚点；隐藏大纲暂停查询、保留有界分页和列表位置；最大化切换期间临时关闭 DWM 位图拉伸，避免重影。
- 100% DPI 下已操作空白／长文档、四标签和富内容文档，核对输入撤销重做保存、拖动／最小尺寸、返回按钮、关闭返回及双栏恢复。最终 Release 另复验明暗主题、最大化恢复和 20 轮普通窗口往返，哈希与分构建证据见 `verification.json`；保留验收窗口供用户试用。
- 20 轮期间用户同时执行公式插入／连续撤销，出现一次修订冲突提示，尚未修复；不据此宣称编辑器全部通过。未测真实 IME、跨屏 DPI、减少动态效果、外部变更小窗提示及完整三轮性能对照；采样不代表帧率、泄漏或永久内存上限。
