# Nomo 原生代码职责索引

以当前代码为准；修改前先查职责入口。旧 Web 索引已归档，不参与构建。

| 职责 | 主入口 | 边界 |
|---|---|---|
| 根构建、版本、锁文件 | `Cargo.toml`、`Cargo.lock` | 单成员 workspace；唯一产品版本；根 target |
| 启动、菜单、平台打开事件 | `src-gpui/src/main.rs`、`open_instance.rs` | 默认 Workspace 与 feature 开发入口；系统打开路由、Windows 同数据目录／会话的有界实例消息转交 |
| 文档主页与最近记录 | `src-gpui/src/ui/home.rs`、`home/library.rs`、`home/thumbnail.rs` | 无文件启动进入主页；本地文件夹入口、最近列表、虚拟网格和有界页首缩略图；与编辑页共用外壳面板、侧栏宽度和主页导航行，不创建编辑会话 |
| 默认应用外壳 | `src-gpui/src/ui/workspace.rs`、`workspace/toolbar.rs`、`workspace_surface.rs` | 多标签、连体边框与阴影、文件树、模式、工具栏、搜索、自动保存 |
| 编辑收尾与文档切换 | `src-gpui/src/ui/workspace/document_transition.rs`、`nomo_mark/workspace_host.rs`、`nomo_mark/object_ui.rs` | 组字和对象草稿收尾、确认后继续切换或关闭；单个待执行意图、临时订阅，无正文镜像或轮询 |
| 浮动缩放与统计 | `src-gpui/src/ui/workspace/status.rs` | 原版右下角按钮、向上弹层、四种统计、阅读时间与缩放；复用会话统计和阅读锚点，不持有正文 |
| Markdown Mini 小窗 | `src-gpui/src/ui/workspace/mini.rs`、`ui/mini.rs`、`ui/mini/native.rs` | Windows 原地切换、置顶与几何恢复；悬停控制条独立重绘，复用当前会话和阅读锚点；返回时释放控制条与原生回调 |
| 打开耗时采样 | `src-gpui/src/open_profile.rs`、`ui/workspace.rs`、`nomo_mark/view.rs` | 显式启用；记录会话、首窗口、首帧和语义恢复，不记录正文或路径 |
| 帧耗时采样 | `src-gpui/src/frame_profile.rs`、`ui/workspace.rs`、`scripts/gen-perf-doc.py` | `NOMO_FRAME_PROFILE` 显式启用；有界样本、帧耗时与按键上屏的中位数／p95，固定种子百万字样本 |
| 文件树 | `src-gpui/src/ui/file_tree.rs`、`ui/workspace/explorer.rs`、`explorer_ui.rs`、`explorer_label.rs`、`explorer_actions.rs`、`explorer_search.rs` | 目录快照预算、后台按需刷新、虚拟滚动、名称省略、交互和键盘搜索；重命名同步现有文档会话 |
| 标题栏和应用内菜单 | `src-gpui/src/ui/chrome.rs`、`ui/window_chrome.rs` | 轻量状态摘要、焦点恢复、窗口拖动与按钮；关闭交回 Workspace 确认 |
| NomoMark 正式文档宿主 | `src-gpui/src/nomo_mark/document_host.rs`、`workspace_host.rs`、`save_dialog.rs` | 每标签窗口生命周期、控件事件、正式错误恢复；保存框按当前目录和首个 H1 提供默认值，不持有全文 |
| 文档元数据入口 | `src-gpui/src/nomo_mark/metadata_commands.rs`、`object_ui.rs` | 按需加载文首、已有属性原位编辑、缺少时通过主会话插入模板；复用窗口预算、取消与撤销历史 |
| NomoMark 剪贴板 | `src-gpui/src/clipboard.rs`、`clipboard_rich.rs`、`clipboard_images.rs`、`nomo_mark/clipboard_content.rs`、`session_actions.rs` | 局部及跨窗口语义复制、纯文本／HTML 双格式、受限 HTML 粘贴及有界图片输入；Windows 成功写入后才删除原文，跨窗口输出限制 64 MiB |
| 旧编辑实现与共享动作 | `src-gpui/src/editor.rs`、`buffer.rs`、`document/mod.rs` | 默认 Workspace 不再创建 Buffer/EditorView；保留共用动作、渲染类型与保存前快照工具 |
| Markdown 渲染 | `src-gpui/src/markdown/` | 块结构、行内标记、当前窗口代码布局 |
| 跨行代码高亮 | `src-gpui/src/markdown/code_highlight.rs`、`highlight.rs`、`nomo_mark/source_view.rs` | syntect／two-face 原生语法，代码卡片与源码视图共用；按需语法库、8 MiB／120 项 LRU 和局部解析预算 |
| 表格宽度与内容适配 | `src-gpui/src/nomo_mark/table_widths.rs`、`src-gpui/src/markdown/table_layout.rs` | 共享列宽约束、纯语义测量、表头字体与图片缩放；只作用于当前窗口布局，不写回自动列宽 |
| 代码块原位编辑与语言选择 | `src-gpui/src/nomo_mark/code_blocks.rs`、`code_ui.rs`、`src-gpui/src/markdown/code_layout.rs` | 主会话局部输入与围栏保护；旧版卡片、固定行号与块内双向滚动、头部复制反馈／展开／语言选择；无代码草稿或独立历史 |
| 行内注释标签与原位编辑 | `src-gpui/src/nomo_mark/comments.rs`、`object_ui.rs` | 原版绿色短标签、12 字预览、正文及表格内原位输入；局部草稿按会话确认，IME 收尾后提交，关闭释放输入控件 |
| HTML 容器与属性卡片 | `src-gpui/src/nomo_mark/html_table.rs`、`front_matter.rs`、`object_ui.rs` | 安全 HTML 文本及实体映射、Details 标题边界；Front Matter 字段摘要、块注释完整预览与原位草稿；仅解析当前窗口，不重写未知属性 |
| 文字布局与 KP | `src-gpui/src/layout.rs`、`kp.rs` | GPUI 塑形、上一帧段落弱引用复用、换行与可见选区几何 |
| 图片／公式／图表 | `src-gpui/src/images.rs`、`embeds.rs`、`cache.rs` | 本地／HTTP(S) 图片解码预检、有限任务队列、失败与位图预算、CPU／atlas 淘汰；全屏查看复用现有图片和公式／图表 Store |
| 公式字体回退 | `src-gpui/src/math_fallback.rs`、`vendor/latex-rust/NOMO-PATCH.md` | 固定版 TeX 引擎扩展缺字接口，系统字体度量及轮廓参与正常排版；缓存只活到单次公式完成，复用现有位图缓存 |
| 主题、动效与平台字体 | `src-gpui/src/theme.rs`、`theme_catalog.rs`、`resources/color-themes.json`、`src-gpui/src/ui/motion.rs` | 四套原版主题、有限用户主题目录及 JSON 导入／导出；同明暗配色与字体变更也失效可见区布局，图表配色进入共享缓存键 |
| 配置与设置窗口 | `src-gpui/src/settings.rs`、`ui/settings_schema.rs`、`ui/settings_window.rs`、`ui/settings_pages.rs` | 旧配置结构、平台目录、原子写入；八个原版分区、主题切换／导入／导出、字体恢复、图片策略及默认图表；未接通项只读禁用 |
| 自定义快捷键 | `src-gpui/src/ui/shortcuts.rs`、`ui/settings_shortcuts.rs`、`ui/mod.rs` | 15 项旧配置键（含 HTML／PDF 导出）、输入草稿与冲突校验；提交后替换有限应用键表并更新菜单提示，不累计旧绑定 |
| UI 原件与图标 | `src-gpui/src/ui/components.rs`、`popup.rs`、`icon_assets.rs`、`text_field.rs` | 静态 SVG 内嵌、按钮、菜单、输入框；弹层按触发边界测量、翻转、限高和入场动画 |
| 确认框与短提示 | `src-gpui/src/ui/dialogs.rs`、`dialogs/secondary.rs`、`toast.rs` | 保存、删除、外部变更、恢复、打开位置与关闭选择的旧版布局；焦点与短动效、单条有界 Toast，不接管系统文件选择器 |
| 应用弹窗业务与更新 | `src-gpui/src/ui/workspace/application_dialogs.rs`、`open_requests.rs`、`recovery_ui.rs`、`ui/tray.rs`、`ui/updates.rs`、`ui/updates/notice.rs` | 有界打开队列、跨窗口归属和目标绑定；应用共享恢复列表，主页不自动恢复提示；Windows 托盘、官方 Release 与有界发布说明 |
| 快捷插入 | `src-gpui/src/nomo_mark/quick_insert.rs`、`quick_insert_view.rs`、`quick_insert_host.rs`、`src-gpui/src/ui/quick_insert.rs` | 语义区真实输入触发；14 类／41 种样式，宿主浮层、原子局部事务和过期选择器保护；不持有全文 |
| 图片导入与上传 | `src-gpui/src/image_import.rs`、`image_upload.rs`、`src-gpui/src/nomo_mark/image_properties.rs` | 文件选择、粘贴及拖放共用本地三种复制策略／PicGo Server／Core；有界串行预检、临时文件清理、取消、超时及过期落点保护；沿用共享解码与缓存预算 |
| NomoMark 协议与局部状态 | `src-gpui/src/nomo_mark/protocol.rs`、`host.rs`、`interaction.rs` | 窗口事务、乐观确认、组字和回收门禁 |
| NomoMark 解析与编辑命令 | `src-gpui/src/nomo_mark/parse.rs`、`dialect.rs`、`commands.rs`、`editing_rules.rs`、`inline_commands.rs`、`list_commands.rs` | 语义结构、原文坐标、尾部输入落点、Details／标题键盘规则和局部补丁 |
| NomoMark 视图与对象 | `src-gpui/src/nomo_mark/view.rs`、`composition.rs`、`source_view.rs`、`paint.rs`、`object_ui.rs`、`table.rs`、`html_table.rs`、`table_inline.rs` | 键鼠、IME、跨窗口临时组字、两种表格共用对象边界、图像与对象交互 |
| 表格交互与结构控件 | `src-gpui/src/nomo_mark/table_controls.rs`、`table_clipboard.rs`、`node_controls.rs` | 基于实际单元格几何的行列按钮、尺寸选择、矩形选区；标题、代码语言、Callout、Details、脚注及卡片控件；不创建第二套编辑模型 |
| 编辑区右键菜单 | `src-gpui/src/nomo_mark/context_menu.rs`、`context_menu_ui.rs`、`context_image.rs` | 按正文、选区、链接及对象生成原版菜单；鼠标锚定、二级菜单与键盘导航；目标身份检查、局部事务和按需图片属性输入 |
| 链接编辑与导航 | `src-gpui/src/nomo_mark/link_edit.rs`、`link_target.rs`、`src-gpui/src/ui/workspace.rs` | 局部链接弹层；本地 Markdown 路径解析、复用标签与异步锚点导航；正文仍按窗口加载 |
| 脚注悬停预览 | `src-gpui/src/nomo_mark/footnote_preview.rs`、`workspace_queries.rs` | 共享结构索引定位，最多读取 8 KiB／显示 360 字；提示关闭、文档隐藏和后续悬停取消旧请求 |
| 键盘与局部资源失效 | `src-gpui/src/nomo_mark/keyboard.rs`、`keyboard_session.rs`、`navigation.rs`、`resources.rs` | 按词、行、文档与翻页动作；有界跨窗查询；按资源依赖失效，无全文镜像 |
| 连续窗口与预算 | `src-gpui/src/nomo_mark/native_port.rs`、`session_actions.rs`、`session_window.rs`、`viewport_ui.rs`、`resident_budget.rs` | 正式/开发共用会话连接、大编辑重分配、超大结构原文分片、失败重试、预取、回收和驻留预算 |
| 同会话单栏/双栏 | `src-gpui/src/nomo_mark/window_panes.rs` | 按需 Pane、唯一输入控件、全局导航、局部窗口同步、模式/历史阅读锚点与取消；不持有完整正文/历史 |
| 页面全文查询 | `src-gpui/src/nomo_mark/workspace_queries.rs` | 后台搜索分页、全文统计、大纲；按文档请求取消和过期校验 |
| 工具栏文档命令与语言 | `src-gpui/src/nomo_mark/toolbar_commands.rs`、`toolbar_global.rs`、`semantic_selection.rs`、`semantic_global.rs`、`toc_update.rs`、`src-gpui/src/ui/toolbar_i18n.rs`、`src-gpui/resources/toolbar-locales.json` | 脚注和目录分页、派生事务、跨窗口语义补丁、四种界面语言；按需和预算内处理 |
| 开发界面 | `src-gpui/src/nomo_mark/demo.rs`、`development_ui.rs` | feature 只控制开发入口和调试工具条 |
| 文档导出 | `src-gpui/src/document_export.rs`、`document_export/{html,resources,model,media,pdf,pdf_fonts,word}.rs`、`ui/workspace/export.rs` | 固定修订及单个可取消任务；有界临时模型，HTML 流式输出、PDF 嵌入字体分页、Word 原生 OOXML；原子发布前复核取消，无第二份编辑模型或常驻正文 |
| 外部文件变更策略 | `src-gpui/src/ui/workspace/application_dialogs.rs`、`text_document/markdown_session/external.rs` | 干净文档按偏好自动重载／覆盖／忽略；本地修改和删除仍显式选择，忽略继续阻止普通保存，覆盖校验检测令牌；Mini 返回后处理延迟冲突 |
| 文档会话与保存 | `src-gpui/src/text_document/mod.rs`、`markdown_session.rs`、`markdown_session/rename.rs`、`markdown_session/external.rs`、`markdown_session/recovery.rs`、`markdown_history.rs` | PieceTree、磁盘正文与历史、快照、保存及重命名事务；外部身份复核、进程租约和原会话按需恢复 |
| 结构索引／搜索／统计 | `src-gpui/src/text_document/structure_index/`、`markdown_search/`、`writing_stats/` | 唯一结构索引、分页查询和有界统计 |
| 编码、文件写入、格式模板 | `src-gpui/src/models.rs`、`file_system.rs`、`markdown_templates.rs` | 编码定义、原子替换、显式模板来源 |
| Emoji 加载 | `src-gpui/src/nomo_mark/native_emoji.rs`、`ui/emoji_picker.rs` | 按需加载；固定版本 JSON、哈希、虚拟列表和原有缓存预算 |
| 策略和既有语料 | `src-gpui/resources/` | 策略 JSON、搜索与结构样本 |
| 应用资源 | `src-gpui/assets/`、`src-gpui/build.rs` | SVG、图标、Windows manifest／版本资源 |
| 打包 | `scripts/package-windows.ps1`、`package-macos.sh`、`package-linux.sh`、`packaging/` | Windows x64 环境初始化、三平台原生包与产物目录组装 |
| 发布 | `.github/workflows/release.yml`、`scripts/verify-release.py` | 六类资产齐备后发布；Homebrew 和下载站同步 |

检查入口：`cargo check --locked --bin nomo`、`cargo build --locked --release --bin nomo`。NomoMark 开发入口附加 `--features nomo-mark-demo`；定向测试为 `cargo test --locked --release --features nomo-mark-demo --bin nomo <过滤条件>`。

平台打包和实机验收见 `docs/native-build.md`；历史结果见 `docs/archive/`，不得作为当前验证证据。
