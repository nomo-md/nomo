# Nomo 原生代码职责索引

以当前代码为准；修改前先查职责入口。旧 Web 索引已归档，不参与构建。

| 职责 | 主入口 | 边界 |
|---|---|---|
| 根构建、版本、锁文件 | `Cargo.toml`、`Cargo.lock` | 单成员 workspace；唯一产品版本；根 target |
| 启动、菜单、平台打开事件 | `src-gpui/src/main.rs` | 默认 Workspace 与 feature 开发入口；本地文件 URL 路由 |
| 默认应用外壳 | `src-gpui/src/ui/workspace.rs`、`workspace/toolbar.rs`、`workspace_surface.rs` | 多标签、连体边框与阴影、文件树、模式、工具栏、搜索、自动保存 |
| 文件树 | `src-gpui/src/ui/file_tree.rs`、`ui/workspace/explorer.rs`、`explorer_ui.rs`、`explorer_label.rs`、`explorer_actions.rs`、`explorer_search.rs` | 目录快照预算、后台按需刷新、虚拟滚动、名称省略、交互和键盘搜索；重命名同步现有文档会话 |
| 标题栏和应用内菜单 | `src-gpui/src/ui/chrome.rs`、`ui/window_chrome.rs` | 轻量状态摘要、焦点恢复、窗口拖动与按钮；关闭交回 Workspace 确认 |
| 正式窗口编辑宿主 | `src-gpui/src/editor_control/document_host.rs`、`workspace_host.rs` | 每标签窗口生命周期、控件事件、正式错误恢复；不持有全文 |
| 旧编辑实现与共享动作 | `src-gpui/src/editor.rs`、`buffer.rs`、`document/mod.rs` | 默认 Workspace 不再创建 Buffer/EditorView；保留共用动作、渲染类型与保存前快照工具 |
| Markdown 渲染 | `src-gpui/src/markdown/` | 块结构、行内标记、轻量高亮 |
| 文字布局与 KP | `src-gpui/src/layout.rs`、`kp.rs` | GPUI 塑形、换行与选区几何 |
| 图片／公式／图表 | `src-gpui/src/images.rs`、`embeds.rs`、`cache.rs` | 按需资源、后台渲染与有界缓存 |
| 主题、动效与平台字体 | `src-gpui/src/theme.rs`、`src-gpui/src/ui/motion.rs` | 平台默认、系统减少动态效果、短时颜色过渡及排版指标 |
| 配置与设置窗口 | `src-gpui/src/settings.rs`、`ui/settings_schema.rs`、`ui/settings_window.rs` | 旧配置结构、平台目录、原子写入；隐藏未支持项 |
| UI 原件与图标 | `src-gpui/src/ui/components.rs`、`icon_assets.rs`、`text_field.rs` | 静态 SVG 内嵌、按钮、菜单、输入框 |
| 控件协议与局部状态 | `src-gpui/src/editor_control/protocol.rs`、`host.rs`、`interaction.rs` | 窗口事务、乐观确认、组字和回收门禁 |
| 控件解析与编辑命令 | `src-gpui/src/editor_control/parse.rs`、`dialect.rs`、`commands.rs`、`inline_commands.rs`、`list_commands.rs` | 语义结构、原文坐标和局部补丁 |
| 控件视图与对象 | `src-gpui/src/editor_control/view.rs`、`source_view.rs`、`paint.rs`、`object_ui.rs`、`table.rs`、`html_table.rs` | 键鼠、IME、表格、图像与对象交互 |
| 连续窗口与预算 | `src-gpui/src/editor_control/native_port.rs`、`session_actions.rs`、`session_window.rs`、`viewport_ui.rs`、`resident_budget.rs` | 正式/开发共用会话连接、预取、回收和驻留预算 |
| 同会话单栏/双栏 | `src-gpui/src/editor_control/window_panes.rs` | 按需 Pane、唯一输入控件、局部窗口同步、阅读锚点与取消；不持有完整正文/历史 |
| 页面全文查询 | `src-gpui/src/editor_control/workspace_queries.rs` | 后台搜索分页、全文统计、大纲；按文档请求取消和过期校验 |
| 工具栏文档命令与语言 | `src-gpui/src/editor_control/toolbar_commands.rs`、`toolbar_global.rs`、`semantic_selection.rs`、`semantic_global.rs`、`toc_update.rs`、`src-gpui/src/ui/toolbar_i18n.rs`、`src-gpui/resources/toolbar-locales.json` | 脚注和目录分页、派生事务、跨窗口语义补丁、四种界面语言；按需和预算内处理 |
| 开发界面 | `src-gpui/src/editor_control/demo.rs`、`development_ui.rs` | feature 只控制开发入口和调试工具条 |
| 文档会话与保存 | `src-gpui/src/text_document/mod.rs`、`markdown_session.rs`、`markdown_session/rename.rs`、`markdown_history.rs` | PieceTree、磁盘正文与历史、快照、保存及重命名事务和恢复格式 |
| 结构索引／搜索／统计 | `src-gpui/src/text_document/structure_index/`、`markdown_search/`、`writing_stats/` | 唯一结构索引、分页查询和有界统计 |
| 编码、文件写入、格式模板 | `src-gpui/src/models.rs`、`file_system.rs`、`markdown_templates.rs` | 编码定义、原子替换、显式模板来源 |
| Emoji 加载 | `src-gpui/src/editor_control/native_emoji.rs`、`ui/emoji_picker.rs` | 按需加载；固定版本 JSON、哈希、虚拟列表和原有缓存预算 |
| 策略和既有语料 | `src-gpui/resources/` | 策略 JSON、搜索与结构样本 |
| 应用资源 | `src-gpui/assets/`、`src-gpui/build.rs` | SVG、图标、Windows manifest／版本资源 |
| 打包 | `scripts/package-windows.ps1`、`package-macos.sh`、`package-linux.sh`、`packaging/` | 三平台原生包；任务产物目录组装 |
| 发布 | `.github/workflows/release.yml`、`scripts/verify-release.py` | 六类资产齐备后发布；Homebrew 和下载站同步 |

检查入口：`cargo check --locked --bin nomo`、`cargo build --locked --release --bin nomo`。开发控件附加 `--features editor-control-demo`；定向测试为 `cargo test --locked --release --features editor-control-demo --bin nomo <过滤条件>`。

平台打包和实机验收见 `docs/native-build.md`；历史结果见 `docs/archive/`，不得作为当前验证证据。
