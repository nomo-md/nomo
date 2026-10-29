# Nomo 原生代码职责索引

以当前代码为准；修改前先查职责入口。旧 Web 索引已归档，不参与构建。

| 职责 | 主入口 | 边界 |
|---|---|---|
| 根构建、版本、锁文件 | `Cargo.toml`、`Cargo.lock` | 单成员 workspace；唯一产品版本；根 target |
| 启动、菜单、平台打开事件 | `src-gpui/src/main.rs` | 默认 Workspace 与 feature 开发入口；本地文件 URL 路由 |
| 文档主页与最近记录 | `src-gpui/src/ui/home.rs`、`home/library.rs`、`home/thumbnail.rs` | 无文件启动进入主页；本地文件夹入口、最近列表、虚拟网格和有界页首缩略图；与编辑页共用外壳面板、侧栏宽度和主页导航行，不创建编辑会话 |
| 默认应用外壳 | `src-gpui/src/ui/workspace.rs`、`workspace/toolbar.rs`、`workspace_surface.rs` | 多标签、连体边框与阴影、文件树、模式、工具栏、搜索、自动保存 |
| 浮动缩放与统计 | `src-gpui/src/ui/workspace/status.rs` | 原版右下角按钮、向上弹层、四种统计、阅读时间与缩放；复用会话统计和阅读锚点，不持有正文 |
| Markdown Mini 小窗 | `src-gpui/src/ui/workspace/mini.rs`、`ui/mini.rs`、`ui/mini/native.rs` | Windows 原地切换、置顶与几何恢复；悬停控制条独立重绘，复用当前会话和阅读锚点；返回时释放控制条与原生回调 |
| 打开耗时采样 | `src-gpui/src/open_profile.rs`、`ui/workspace.rs`、`nomo_mark/view.rs` | 显式启用；记录会话、首窗口、首帧和语义恢复，不记录正文或路径 |
| 帧耗时采样 | `src-gpui/src/frame_profile.rs`、`ui/workspace.rs`、`scripts/gen-perf-doc.py` | `NOMO_FRAME_PROFILE` 显式启用；有界样本、帧耗时与按键上屏的中位数／p95，固定种子百万字样本 |
| 文件树 | `src-gpui/src/ui/file_tree.rs`、`ui/workspace/explorer.rs`、`explorer_ui.rs`、`explorer_label.rs`、`explorer_actions.rs`、`explorer_search.rs` | 目录快照预算、后台按需刷新、虚拟滚动、名称省略、交互和键盘搜索；重命名同步现有文档会话 |
| 标题栏和应用内菜单 | `src-gpui/src/ui/chrome.rs`、`ui/window_chrome.rs` | 轻量状态摘要、焦点恢复、窗口拖动与按钮；关闭交回 Workspace 确认 |
| NomoMark 正式文档宿主 | `src-gpui/src/nomo_mark/document_host.rs`、`workspace_host.rs` | 每标签窗口生命周期、控件事件、正式错误恢复；不持有全文 |
| NomoMark 文本剪贴板 | `src-gpui/src/clipboard.rs`、`nomo_mark/session_actions.rs`、`nomo_mark/view.rs` | 局部及跨窗口复制／剪切共用写入确认；Windows 使用活动窗口持有剪贴板，成功后才删除原文 |
| 旧编辑实现与共享动作 | `src-gpui/src/editor.rs`、`buffer.rs`、`document/mod.rs` | 默认 Workspace 不再创建 Buffer/EditorView；保留共用动作、渲染类型与保存前快照工具 |
| Markdown 渲染 | `src-gpui/src/markdown/` | 块结构、行内标记、轻量高亮 |
| 文字布局与 KP | `src-gpui/src/layout.rs`、`kp.rs` | GPUI 塑形、上一帧段落弱引用复用、换行与可见选区几何 |
| 图片／公式／图表 | `src-gpui/src/images.rs`、`embeds.rs`、`cache.rs` | 图片解码预检、有限任务队列、失败与位图预算、CPU／atlas 淘汰；公式和图表沿用共享 Store |
| 主题、动效与平台字体 | `src-gpui/src/theme.rs`、`src-gpui/src/ui/motion.rs` | 平台默认、系统减少动态效果、短时颜色过渡及排版指标 |
| 配置与设置窗口 | `src-gpui/src/settings.rs`、`ui/settings_schema.rs`、`ui/settings_window.rs`、`ui/settings_pages.rs` | 旧配置结构、平台目录、原子写入；八个原版分区、主题预览、字体恢复、图片策略和高级页快捷键；未接通项只读禁用 |
| UI 原件与图标 | `src-gpui/src/ui/components.rs`、`popup.rs`、`icon_assets.rs`、`text_field.rs` | 静态 SVG 内嵌、按钮、菜单、输入框；弹层按触发边界测量、翻转、限高和入场动画 |
| 确认框与短提示 | `src-gpui/src/ui/dialogs.rs`、`dialogs/secondary.rs`、`toast.rs` | 保存、删除、外部变更、恢复、打开位置与关闭选择的旧版布局；焦点与短动效、单条有界 Toast，不接管系统文件选择器 |
| 应用弹窗业务与更新 | `src-gpui/src/ui/workspace/application_dialogs.rs`、`ui/tray.rs`、`ui/updates.rs`、`ui/updates/notice.rs` | 活动文档身份检查、启动恢复、窗口路由与 Windows 托盘；官方 Release、按需通知及有界原生发布说明，下载交给浏览器 |
| 快捷插入 | `src-gpui/src/nomo_mark/quick_insert.rs`、`quick_insert_view.rs`、`quick_insert_host.rs`、`src-gpui/src/ui/quick_insert.rs` | 语义区真实输入触发；14 类／41 种样式，宿主浮层、原子局部事务和过期选择器保护；不持有全文 |
| 本地图片导入 | `src-gpui/src/image_import.rs`、`src-gpui/src/nomo_mark/image_properties.rs` | 三种本地复制策略、串行预检／复制、同内容复用、独占创建、防路径越界及取消；像素／百分比宽度往返，沿用共享解码与缓存预算 |
| NomoMark 协议与局部状态 | `src-gpui/src/nomo_mark/protocol.rs`、`host.rs`、`interaction.rs` | 窗口事务、乐观确认、组字和回收门禁 |
| NomoMark 解析与编辑命令 | `src-gpui/src/nomo_mark/parse.rs`、`dialect.rs`、`commands.rs`、`inline_commands.rs`、`list_commands.rs` | 语义结构、原文坐标和局部补丁 |
| NomoMark 视图与对象 | `src-gpui/src/nomo_mark/view.rs`、`composition.rs`、`source_view.rs`、`paint.rs`、`object_ui.rs`、`table.rs`、`html_table.rs`、`table_inline.rs` | 键鼠、IME、跨窗口临时组字、两种表格共用对象边界、图像与对象交互 |
| 链接编辑与导航 | `src-gpui/src/nomo_mark/link_edit.rs`、`link_target.rs`、`src-gpui/src/ui/workspace.rs` | 局部链接弹层；本地 Markdown 路径解析、复用标签与异步锚点导航；正文仍按窗口加载 |
| 键盘与局部资源失效 | `src-gpui/src/nomo_mark/keyboard.rs`、`keyboard_session.rs`、`navigation.rs`、`resources.rs` | 按词、行、文档与翻页动作；有界跨窗查询；按资源依赖失效，无全文镜像 |
| 连续窗口与预算 | `src-gpui/src/nomo_mark/native_port.rs`、`session_actions.rs`、`session_window.rs`、`viewport_ui.rs`、`resident_budget.rs` | 正式/开发共用会话连接、大编辑重分配、超大结构原文分片、失败重试、预取、回收和驻留预算 |
| 同会话单栏/双栏 | `src-gpui/src/nomo_mark/window_panes.rs` | 按需 Pane、唯一输入控件、局部窗口同步、阅读锚点与取消；不持有完整正文/历史 |
| 页面全文查询 | `src-gpui/src/nomo_mark/workspace_queries.rs` | 后台搜索分页、全文统计、大纲；按文档请求取消和过期校验 |
| 工具栏文档命令与语言 | `src-gpui/src/nomo_mark/toolbar_commands.rs`、`toolbar_global.rs`、`semantic_selection.rs`、`semantic_global.rs`、`toc_update.rs`、`src-gpui/src/ui/toolbar_i18n.rs`、`src-gpui/resources/toolbar-locales.json` | 脚注和目录分页、派生事务、跨窗口语义补丁、四种界面语言；按需和预算内处理 |
| 开发界面 | `src-gpui/src/nomo_mark/demo.rs`、`development_ui.rs` | feature 只控制开发入口和调试工具条 |
| 文档会话与保存 | `src-gpui/src/text_document/mod.rs`、`markdown_session.rs`、`markdown_session/rename.rs`、`markdown_session/external.rs`、`markdown_session/recovery.rs`、`markdown_history.rs` | PieceTree、磁盘正文与历史、快照、保存及重命名事务；外部身份复核、进程租约和原会话按需恢复 |
| 结构索引／搜索／统计 | `src-gpui/src/text_document/structure_index/`、`markdown_search/`、`writing_stats/` | 唯一结构索引、分页查询和有界统计 |
| 编码、文件写入、格式模板 | `src-gpui/src/models.rs`、`file_system.rs`、`markdown_templates.rs` | 编码定义、原子替换、显式模板来源 |
| Emoji 加载 | `src-gpui/src/nomo_mark/native_emoji.rs`、`ui/emoji_picker.rs` | 按需加载；固定版本 JSON、哈希、虚拟列表和原有缓存预算 |
| 策略和既有语料 | `src-gpui/resources/` | 策略 JSON、搜索与结构样本 |
| 应用资源 | `src-gpui/assets/`、`src-gpui/build.rs` | SVG、图标、Windows manifest／版本资源 |
| 打包 | `scripts/package-windows.ps1`、`package-macos.sh`、`package-linux.sh`、`packaging/` | 三平台原生包；任务产物目录组装 |
| 发布 | `.github/workflows/release.yml`、`scripts/verify-release.py` | 六类资产齐备后发布；Homebrew 和下载站同步 |

检查入口：`cargo check --locked --bin nomo`、`cargo build --locked --release --bin nomo`。NomoMark 开发入口附加 `--features nomo-mark-demo`；定向测试为 `cargo test --locked --release --features nomo-mark-demo --bin nomo <过滤条件>`。

平台打包和实机验收见 `docs/native-build.md`；历史结果见 `docs/archive/`，不得作为当前验证证据。
