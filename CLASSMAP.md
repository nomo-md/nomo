# CLASSMAP.md

## Purpose

本文件将 Nomo Markdown 编辑器的功能映射到具体代码单元，帮助后续 AI 编码会话快速定位文件，避免大范围搜索。

## How to Use

- 修改行为前先查本文件。
- 用 **Feature Index** 按功能找候选代码。
- 用 **Code Unit Entries** 了解各模块的职责边界与非职责。
- 若本文件与代码冲突，**以代码为准**并更新本文件。

---

## Feature Index

### 应用启动与窗口生命周期

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 前端入口挂载 | `src/main.ts` | `src/app/services/themeManager.ts`, `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte` | 添加新入口视图、主题启动快照或全局样式加载 |
| Windows 自绘窗口 chrome | `src/app/components/WindowsCaptionControls.svelte` | `src/app/components/AppTitleBar.svelte`, `src/app/components/SettingsWindow.svelte`, `src/app/services/platform.ts`, `src-tauri/tauri.windows.conf.json`, `src-tauri/src/window/os/windows.rs` | 修改 Windows 自绘按钮、无边框窗口或标题栏布局 |
| 应用装配中心 | `src/app/App.svelte` | 所有 service、editor-core、组件 | 标签页/文件系统/编辑器之间的协调逻辑变更 |
| 桌面窗口生命周期 | `src/app/services/desktopWindow.ts` | `src-tauri/src/window/` | 窗口事件、关闭行为、托盘交互变更 |
| Rust 后端入口 | `src-tauri/src/lib.rs` | `src-tauri/src/main.rs` | 新增 IPC 命令、插件、窗口事件 |
| 自定义标题栏菜单 | `src/app/components/AppTitleBar.svelte` | `src/app/App.svelte`, `src/app/services/appCommands.ts` | 添加/移除菜单项、修改菜单文案 |
| 工作区与选中标签联合轮廓 | `src/app/components/WorkspaceSurface.svelte` | `src/app/components/AppShell.svelte`, `src/app/components/DocumentTabs.svelte`, `src/app/actions/motion.ts`, `src/app/styles/app-chrome.css` | 修改连续外轮廓、主题描边、柔和投影或活动标签几何同步 |
| Markdown 工具栏显示与响应式布局 | `src/app/components/EditorToolbar.svelte` | `src/app/components/AppShell.svelte`, `src/app/actions/motion.ts`, `src/app/styles/app-chrome.css`, `src/app/styles/app-responsive.css`, `src/app/services/settings.ts` | 修改工具栏收展、窄宽度隐藏优先级或内容宽度控件 |
| 窗口状态持久化 | `src-tauri/src/window/state.rs` | `src-tauri/src/lib.rs`, `src-tauri/src/models.rs` | 窗口位置/尺寸/最大化状态恢复逻辑变更 |
| Markdown 文档小窗 | `src/app/App.svelte` | `src/app/components/AppShell.svelte`, `src/app/components/AppTitleBar.svelte`, `src/app/components/MarkdownMiniLargePreview.svelte`, `src/app/styles/markdown-mini.css`, `src/app/services/markdownMiniTransition.ts`, `src/app/services/desktopWindow.ts`, `src-tauri/src/window/state.rs` | 修改小窗进入/返回、置顶、只读降级、快捷键、过渡就绪与阅读位置恢复或原生窗口几何切换 |
| 全局滚动条显隐 | `src/app/services/scrollbarVisibility.ts` | `src/main.ts`, `src/app/styles/global.css`, `src/app/styles/app-layout.css`, `src/app/styles/editor-segmented.css` | 修改滚动时显示、边缘触发、延时隐藏或局部滚动容器覆盖 |
| 外部打开路由 | `src-tauri/src/window/external_open.rs` | `src-tauri/src/lib.rs` | 单实例/启动参数/macOS open 事件、连续请求短暂合并、追加暂存及按窗口原子消费 |
| 跨窗口打开目标去重 | `src-tauri/src/window/open_targets.rs` | `src/app/services/openTargetRouting.ts`, `src/app/App.svelte`, `src/app/services/desktopWindow.ts`, `src-tauri/src/window/external_open.rs` | 修改文件/文件夹打开优先级、空窗口复用、窗口聚焦或目标预留 |
| 系统文件与文件夹拖放打开 | `src/app/App.svelte`, `src-tauri/src/window/open_targets.rs` | `src/app/services/desktopWindow.ts`, `src/lib/desktop/tauriStorage.ts`, `src/app/services/openTargetRouting.ts` | 修改元数据识别、按首个有效目标类型选择单目录或整批文档、文档固定在拖入窗口打开并保留文件去重、延迟打开或目录复用现有打开方式与未保存确认 |

### 编辑器核心（ProseMirror）

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 精细段落排版 | `src/lib/typography/knuthPlass.ts` | `types.ts`, `rules.ts`, `hyphenation.ts`, `measure.ts`, `renderPlan.ts`, `positions.ts`, `solver.ts`, `typography.worker.ts` | 修改 KP 间距质量评分、两端/左对齐选择、中文规则、英文 Liang 断词、可取消分批/有界同步测量、目标几何快照、文字测量 LRU 或位置映射 |
| 精细排版编辑适配 | `src/lib/editor-core/plugins/typography.ts` | `nodeViews/TypographyBreakNodeView.ts`, `InlineSourceCodec.ts`, `ProseMirrorEditorCore.ts`, `src/lib/typography/options.ts`, `rules.ts`, `interaction.ts`, `src/app/styles/editor-document.css`, `src/app/services/settings.ts` | 修改输入与导航保护、字素光标移动、局部缓存、正文输入与符号显隐同帧排版、编辑态真实空格宽度和长空格串换行、超预算编辑降级与空闲追补、宽度持续 KP、缩放停止后追补与可见区更新、增量装饰提交、手势阅读锚点、DOM 绑定校验、视觉断点、软换行、原生 BR 与 KP 软换行投影的局部切换及 IME 延后、KP 选区背景或设置开关 |
| 排版交互生命周期 | `src/lib/typography/interaction.ts` | `src/app/actions/typographyRange.ts`, `src/app/services/settings.ts`, `src/app/services/editorSettingsController.ts`, `src/app/services/markdownMiniTransition.ts`, `src/app/components/StatusBar.svelte`, `src/app/App.svelte` | 修改窗口内重叠交互 token、150 ms 静默收束、即时 CSS 与几何版本通知、阅读锚点回调、缩放滑块直接更新、小窗过渡排版暂停或跨窗口几何更新 |
| 只读与离线排版 | `src/lib/typography/dom.ts` | `renderPlan.ts`, `selectionPaint.ts`, `exportRuntime.ts`, `exportPrint.ts`, `dom.test.ts`, `scripts/typography-runtime-plugin.ts`, `src/quicklook/preview-entry.ts`, `src/app/components/MarkdownMiniLargePreview.svelte` | 修改只读布局、图片克隆的加载失效、字距与连续选区背景、离线运行时或打印就绪契约 |
| 编辑器工厂与 API | `src/lib/editor-core/createEditorCore.ts` | `src/lib/editor-core/index.ts` | EditorCore 创建参数或对外接口变更 |
| ProseMirror 核心实现 | `src/lib/editor-core/ProseMirrorEditorCore.ts` | `src/lib/editor-core/clipboardMarkdown.ts`, `markdown.ts`, `schema.ts`, plugins, nodeViews | EditorView 生命周期、事务、模式切换、命令执行、选区 Markdown 通知、剪贴板负载与右键目标事务 |
| 剪贴板 Markdown 判定 | `src/lib/editor-core/clipboardMarkdown.ts` | `src/lib/editor-core/ProseMirrorEditorCore.ts`, `markdown.ts` | 修改纯文本/Markdown 等价判定、粘贴 Slice 或纯文本事务标记 |
| Schema 定义 | `src/lib/editor-core/schema.ts` | `src/lib/editor-core/callout/calloutSchema.ts` | 新增/修改节点或 mark 类型 |
| Markdown 解析与序列化 | `src/lib/editor-core/markdown.ts` | `markdownTokenizer.ts`, `markdownSerialization.ts`, `src/lib/editor-core/callout/`, `html/` | Markdown 与 ProseMirror doc 互转规则变更；Word 可选解析临时放行图片来源，默认编辑与链接校验保持不变；纯 tokenizer 与序列化可供 Worker 复用 |
| Details/Summary 折叠块 | `src/lib/editor-core/details/` | `nodeViews/DetailsNodeView.ts`, `markdown.ts`, `markdownTokenizer.ts`, `markdownSerialization.ts`, `ProseMirrorEditorCore.ts`, `details.test.ts` | 修改单段富文本标题、嵌套正文、HTML/Markdown 往返、键盘进出、默认展开历史或按位置映射的文档会话临时展开状态 |
| Markdown 源码行到语义块导航 | `src/lib/editor-core/markdown.ts`, `src/lib/editor-core/ProseMirrorEditorCore.ts` | `src/app/App.svelte` | 诊断、大纲等功能需要从源码行定位到最近的语义顶层块 |
| 双栏内容滚动跟随 | `src/app/services/markdownScrollSync.ts`, `src/app/services/markdownScrollSyncWorkspace.ts` | `src/app/components/EditorWorkspace.svelte`, `src/app/components/MarkdownSourceEditor.svelte`, `src/lib/editor-core/ProseMirrorEditorCore.ts`, `src/lib/editor-core/scrollSyncMapping.ts` | 修改30%参考线插值、滚动主栏接管、光标安全区、内容修订门控或几何失效处理 |
| 双栏旧等高兼容接口 | `src/app/services/markdownBlockAlignment.ts` | `src/lib/editor-core/plugins/blockAlignment.ts`, `src/lib/editor-core/blockAlignment.test.ts` | 维护旧补偿接口与测试；当前双栏运行路径不调用这些接口 |
| HTML 安全策略 | `src/lib/editor-core/html/htmlPolicy.ts` | `src/lib/editor-core/html/htmlClassifier.ts` | 可编辑 HTML 标签/属性白名单变更 |
| HTML 块分类 | `src/lib/editor-core/html/htmlClassifier.ts` | `src/lib/editor-core/html/htmlPolicy.ts` | HTML 块可编辑性判断/属性提取规则变更 |
| HTML 锚点保留 | `src/lib/editor-core/html/htmlAnchor.ts` | `schema.ts`, `markdown.ts`, `html/htmlToPmLogic.ts`, `html/pmToHtml.ts`, `markdownSerialization.ts` | 空锚点节点、含文本锚点 mark、`id/name` 属性及安全 HTML 输出规则变更 |
| 编辑器命令 | `src/lib/editor-core/editorCommands.ts` | `src/lib/editor-core/tableCommands.ts`, `codeBlockCommands.ts`, `callout/calloutCommands.ts` | 新增或修改编辑命令、Shift+Enter 显式段内硬换行（保留源码软换行语义） |
| 列表创建、层级与类型切换 | `src/lib/editor-core/listCommands.ts` | `ProseMirrorEditorCore.ts`, `markdown.ts`, `editorCommands.ts` | 语义模式 Tab/Shift+Tab、列表符号输入与粘贴规则、选中项类型转换和任务列表退出清理；各入口共用局部事务 |
| 图表模板 | `src/lib/editor-core/diagramTemplates.ts` | `src/app/components/EditorToolbar.svelte` | 新增/修改 Mermaid 图表模板 |
| 链接安全与规范化 | `src/lib/editor-core/link.ts` | `src/lib/editor-core/plugins/linkInteraction.ts`, `src/quicklook/preview.ts` | 链接协议白名单/序列化规则变更 |
| 编辑器类型定义 | `src/lib/editor-core/types.ts` | 所有使用 EditorCore 的模块 | EditorCommand、EditorMode、SetMarkdownOptions 等类型变更 |

### NodeView 渲染

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 代码块 NodeView | `src/lib/editor-core/nodeViews/CodeBlockNodeView.ts` | `src/lib/services/shikiCodeTokenizer.ts`, `src/lib/services/logger.ts`, `renderers.ts`, `ProseMirrorEditorCore.ts`, `nodeViews/activeEditRegistry.ts` | 代码块展示/编辑/高亮、WebView2 中文输入边界与展示换行偏移、Tab / Shift+Tab 选区整行缩进与原生撤销、后台提交保留选区、滚动位置保留及同文档刷新恢复 |
| 图片 NodeView | `src/lib/editor-core/nodeViews/ImageNodeView.ts` | `src/app/services/desktopImageLoader.ts`, `ProseMirrorEditorCore.ts`, `App.svelte` | 图片加载/对齐/尺寸/右键、资源更新等待与导出快照变更 |
| 公式 NodeView | `src/lib/editor-core/nodeViews/MathBlockNodeView.ts`, `MathInlineNodeView.ts` | `src/lib/services/katexMathRenderer.ts` | 公式渲染/编辑体验变更 |
| 图表 NodeView | `src/lib/editor-core/nodeViews/MermaidBlockNodeView.ts` | `src/lib/services/mermaidDiagramRenderer.ts` | Mermaid 图表渲染变更 |
| Callout NodeView | `src/lib/editor-core/nodeViews/CalloutNodeView.ts` | `src/lib/editor-core/callout/` | 提示块展示/编辑变更 |
| 折叠块 NodeView | `src/lib/editor-core/nodeViews/DetailsNodeView.ts` | `details/detailsState.ts`, `details/detailsCommands.ts`, `src/app/styles/editor-document.css`, `src/lib/typography/exportPrint.ts`, `exportRuntime.ts` | 修改箭头/块菜单、组字结束后收起与光标恢复、共享正文 DOM 显隐、原生 HTML 展开重排及打印前递归展开 |
| HTML 块 NodeView | `src/lib/editor-core/nodeViews/HtmlBlockNodeView.ts` | `src/lib/editor-core/html/` | 可编辑 HTML 块行为变更 |
| TOC 块 NodeView | `src/lib/editor-core/nodeViews/TocBlockNodeView.ts` | `src/lib/toc/tocService.ts` | 文档内目录展示变更 |
| 注释块/行内注释 NodeView | `src/lib/editor-core/nodeViews/CommentBlockNodeView.ts`, `CommentInlineNodeView.ts` | `src/lib/editor-core/nodeViews/activeEditRegistry.ts` | 注释卡片展示/编辑/编辑态协调 |
| 脚注 NodeView | `src/lib/editor-core/nodeViews/FootnoteDefNodeView.ts`, `FootnoteRefNodeView.ts` | — | 脚注定义/引用展示、跳转、预览 |
| 分割线 NodeView | `src/lib/editor-core/nodeViews/HorizontalRuleNodeView.ts` | — | 水平分割线渲染/选中 |
| 编辑态注册表 | `src/lib/editor-core/nodeViews/activeEditRegistry.ts` | 代码、公式、图表、图片和注释 NodeView，`ProseMirrorEditorCore.ts` | 编辑对象切换、组字等待、只读取消与保存提交协调 |

### 编辑器插件

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 代码块导航 | `src/lib/editor-core/plugins/codeBlockNavigation.ts` | — | 代码块内外光标移动 |
| 代码高亮装饰 | `src/lib/editor-core/plugins/codeHighlight.ts` | `src/lib/services/shikiCodeTokenizer.ts` | 语法高亮装饰逻辑 |
| 公式输入规则 | `src/lib/editor-core/plugins/displayMathInput.ts`, `mathInlineInput.ts`, `mathBlock.ts` | — | 公式快捷输入 |
| 行内公式插入草稿 | `src/lib/editor-core/plugins/mathInlineDraft.ts` | `editorCommands.ts`, `ProseMirrorEditorCore.ts`, `nodeViews/inlineMathUi.ts`, `nodeViews/activeEditRegistry.ts`, `src/app/services/documentActionsController.ts`, `src/app/App.svelte` | 临时输入与预览、空内容/Esc 取消、选区映射、确认历史与保存/IME/只读生命周期；hasPendingMathInlineDraft 防止后台轮询、自动保存及工作区持久化提前确认；共享 UI 原件也供 MathInlineNodeView 使用 |
| 八类行内原文与投影 | `src/lib/editor-core/InlineSourceCodec.ts`, `inlineSourceCommands.ts` | `markdownTokenizer.ts`, `markdown.ts`, `markdownSerialization.ts`, `schema.ts`, `ProseMirrorEditorCore.ts` | 原文/格式与位置映射、上下标语法与投影、代码字面粘贴和清除、外层格式组合、HTML 导入与输出语义 |
| 八类行内显隐与组字 | `src/lib/editor-core/plugins/inlineSourceEditing.ts` | `src/app/styles/editor-document.css`, `plugins/typography.ts` | 光标/选区所在格式局部展开、单击与实际拖选区分、拖选保持、IME 冻结、真实符号装饰与空模板追踪 |
| 当前标题层级角标 | `src/lib/editor-core/plugins/headingLevelIndicator.ts` | `src/app/styles/editor-document.css` | H1-H6 当前标题角标的定位、显隐或 GSAP 动效 |
| 正文外侧整段选择 | `src/lib/editor-core/plugins/paragraphSelectionGutter.ts` | `src/lib/editor-core/ProseMirrorEditorCore.ts`, `src/app/styles/editor-document.css`, `src/app/i18n.ts`, `src/app/i18n.ja.ts` | 顶层普通段落与标题左侧的悬停短竖线、真实整段选区、缩放命中和浮层生命周期；不接管列表、表格、代码块或正文拖选 |
| 表格控件 | `src/lib/editor-core/plugins/tableControls.ts` | `src/lib/editor-core/plugins/tableControlDom.ts`, `tableCommands.ts`, `markdownSerialization.ts` | 表格行列控制 UI、共用行增删事务内的列对齐与首行表头归一化、单元格内容保存 |
| 表格自动列宽与局部滚动 | `src/lib/table-layout/widths.ts`, `src/lib/table-layout/dom.ts` | `src/lib/editor-core/nodeViews/TableNodeView.ts`, `src/lib/table-layout/export.ts`, `src/lib/table-layout/table-layout.css`, `plugins/tableControls.ts`, `plugins/typography.ts` | 语义内容度量、合并格约束、按需列宽、视图级调度与缓存、精细排版失效、HTML/PDF 目标宽度重排；自动宽度不写入 Markdown |
| 不可编辑 HTML 回退展示 | `src/lib/editor-core/plugins/tableHtml.ts` | `html/htmlClassifier.ts`, `utils/html.ts`, `plugins/tableHtml.test.ts`, `src/app/services/export/render.ts` | 原始 HTML widget 展示、未变化预览的 DOM 复用与事务位置映射；编辑器与导出共用对齐段落和 HTML 空白处理；Markdown 表格由统一 parser 与 schema 渲染 |
| 任务列表 | `src/lib/editor-core/plugins/taskList.ts` | `src/lib/editor-core/InlineSourceCodec.ts`, `editorCommands.ts`, `ProseMirrorEditorCore.ts`, `markdownSerialization.ts` | 修改任务首段标记识别、列表切换/续行、只读复选框或保存边界 |
| 链接交互 | `src/lib/editor-core/plugins/linkInteraction.ts` | `src/app/components/LinkQuickEditor.svelte` | 链接点击/悬浮/编辑 |
| 八类行内字符命令 | `src/lib/editor-core/inlineSourceCommands.ts` | `editorCommands.ts`, `ProseMirrorEditorCore.ts` | 工具栏/快捷键包裹、上下标互斥切换、局部取消、清除格式与空模板；pendingInlineMarks 为兼容派生接口 |
| 搜索高亮 | `src/lib/editor-core/plugins/searchHighlight.ts` | `src/app/services/searchReplace.ts` | 搜索/替换高亮 |
| 尾部段落补全 | `src/lib/editor-core/plugins/trailingParagraph.ts` | — | 非段落块插入后自动追加空段落 |
| 正文目录事务同步 | `src/lib/editor-core/plugins/tocSync.ts` | `src/lib/toc/tocService.ts` | 标题变化后的 TOC 派生更新与撤销历史保持 |
| 编辑器上下文菜单插件 | `src/lib/editor-core/plugins/contextMenu.ts` | `src/app/App.svelte`, `src/app/components/ContextMenu.svelte` | 语义编辑区目标命中、选区定位与右键菜单事件分发 |
| 快捷插入菜单 | `src/lib/editor-core/plugins/quickInsert.ts` | `src/lib/editor-core/quickInsert.ts`, `editorCommands.ts`, `src/app/styles/editor-quick-insert.css` | 行首/空格后触发、类别/样式键盘导航、浮层定位与生命周期，无搜索 |
| Emoji 短代码建议 | `src/lib/editor-core/plugins/emojiSuggestions.ts`, `src/lib/emoji/sourceSuggestions.ts` | `src/lib/emoji/suggestions.ts`, `sourceContext.ts`, `ProseMirrorEditorCore.ts`, `MarkdownSourceEditor.svelte` | 语义/源码的冒号补全、代码与公式排除、组字保护、确认与撤销、浮层生命周期 |
| Emoji 搜索与联网缓存 | `src/app/services/emojiCatalog.ts`, `src-tauri/src/emoji_catalog.rs` | `src/lib/emoji/catalog.ts`, `src/app/i18n.emoji.ts` | 固定版本元数据、双语搜索索引、三源错峰获取、SHA-256 校验、4 MiB 原子缓存与共享加载 |
| Emoji 选择器与插入目标 | `src/app/components/EmojiPicker.svelte`, `src/app/services/emojiInsertion.ts` | `src/app/App.svelte`, `EditorToolbar.svelte`, `src/app/styles/editor-emoji.css` | 九类分类/虚拟网格、键盘选择、双栏/组合标签目标、修订校验与纯文本插入 |
| 行内代码语法高亮装饰 | `src/lib/editor-core/plugins/codeHighlightDecorationPlugin.ts` | `InlineSourceCodec.ts`, `plugins/inlineCodeSelectionBridge.ts` | 派生代码范围的 token 着色与选区背景 |
| 正文与行内对象选区背景 | `src/lib/editor-core/plugins/inlineCodeSelectionBridge.ts` | `src/lib/typography/selectionPaint.ts`, `src/app/styles/editor-document.css`, `plugins/typography.ts` | 修改正文/行内代码选区背景、原子对象及其外边距背景包裹；桥接样式与行绘制共享浏览器选区有效性，保留工具栏所需的模型选区 |
| 三击选块 | `src/lib/editor-core/tripleClickSelection.ts` | `ProseMirrorEditorCore.ts`, `src/app/styles/editor-document.css` | 修改三击判定（按 event.detail 与系统连击一致）、选中目标块，或正文原生选区透明以遮蔽 WebKit 残留选中状态 |
| 连续行选区绘制 | `src/lib/typography/selectionPaint.ts` | `dom.ts`, `src/lib/editor-core/plugins/inlineCodeSelectionBridge.ts` | 合并可见行矩形、按实际行内内容限制背景宽度，保持首帧行高与像素对齐；选区折叠、转移或原生输入聚焦时同步清理本编辑区补色，释放监听与共享 CSSOM；不改正文内容或导出快照 |
| 旧语义块间距装饰（兼容） | `src/lib/editor-core/plugins/blockAlignment.ts` | `src/lib/editor-core/ProseMirrorEditorCore.ts` | 维护不入文档、不入 history 的旧 spacer 接口，双栏不再启用 |

### 文件系统与文档操作

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 文档操作控制器 | `src/app/services/documentActionsController.ts` | `src/app/services/documentFiles.ts`, `tabs.ts`, `recoveryDraft.ts` | 打开/保存/另存/自动保存/外部变更 |
| 编辑器链接目标解析 | `src/app/services/documentLinkNavigation.ts` | `src/app/App.svelte`, `src-tauri/src/external_link.rs` | 相对路径、标题锚点、应用内文档与本地附件导航规则变更 |
| 标签页状态管理 | `src/app/services/tabs.ts` | `src/app/types.ts` | 标签页创建/复用/状态写入 |
| 组合标签模型 | `src/app/services/workspaceItems.ts` | `src/app/types.ts`, `src/app/App.svelte`, `src/app/components/DocumentTabs.svelte` | 两篇 Markdown 的归属、成组、拆组、交换与损坏元数据降级 |
| 双文档运行时与分栏 | `src/app/components/ComparisonWorkspace.svelte`, `ComparisonPane.svelte` | `src/app/services/markdownDocumentRuntime.ts`, `src/app/App.svelte`, `EditorWorkspace.svelte` | 组合共享模式、成员焦点、独立编辑 / 滚动和宽度恢复 |
| 标签跨窗口移交 | `src/app/services/workspaceTransferController.ts` | `src/app/services/tabTransfer.ts`, `src-tauri/src/window/tab_transfer.rs`, `src-tauri/src/window/open_targets.rs` | prepare / target-ready / commit / cancel、文档原子归属、异常恢复和空源窗口关闭 |
| Windows 标签屏幕拖动 | `src-tauri/src/window/tab_drag.rs` | `src/app/components/DocumentTabs.svelte`, `src/app/App.svelte`, `src/app/services/tabTransfer.ts` | Pointer 转原生鼠标追踪、DPI / 屏幕落点和拖动取消 |
| 标签标题拖影 | `src/app/components/TabDragGhost.svelte` | `src/app/App.svelte`, `src/app/components/DocumentTabs.svelte`, `src/app/components/ComparisonPane.svelte`, `src-tauri/src/window/tab_drag.rs` | 窗口内跟随原生鼠标逻辑坐标的普通 / 组合 / 成员标题预览及结束清理 |
| 启动工作区与统一退出 | `src-tauri/src/window/workspace_lifecycle.rs` | `src/app/App.svelte`, `src-tauri/src/window/commands.rs`, `src-tauri/src/lib.rs` | 最后实际关闭窗口快照、跨窗口退出确认、保存失败与超时取消 |
| 最近打开与空白页入口 | `src/app/components/RecentOpenPopover.svelte` | `src/app/components/EmptyWorkspace.svelte`, `src/app/components/AppTitleBar.svelte`, `src-tauri/src/config/commands.rs` | 最近记录浮层、原子删除、跨窗口同步与主动打开排序 |
| 工作区持久化 | `src/app/services/workspacePersistence.ts` | `src/app/App.svelte`, `src/lib/desktop/tauriStorage.ts` | 工作区 v4 元数据、组合项、草稿引用、v3/v2/旧 workspaceTabs 迁移 |
| 阅读位置持久化 | `src/app/services/readingPosition.ts` | `src/app/App.svelte`, `src/app/services/outlineNavigation.ts` | Markdown 文件按路径保存/恢复统一阅读语义锚点 |
| 恢复草稿 | `src/app/services/recoveryDraft.ts` | `src/app/services/documentActionsController.ts` | 异常退出后草稿写入/恢复 |
| Markdown 桥接 | `src/lib/markdown/MarkdownBridge.ts` | `src/lib/markdown/frontMatter.ts` | front matter 与正文分离/合并规则变更 |
| 图片插入协调 | `src/app/services/imageInsertion.ts` | `src/app/services/imageMarkdown.ts`, `src/lib/editor-core/renderers.ts` | 粘贴/拖放图片导入、策略选择、源码插入 |
| 剪贴板协调 | `src/app/services/clipboard.ts` | `src/app/App.svelte`, `@tauri-apps/plugin-clipboard-manager`, `src-tauri/src/file_clipboard.rs` | 文本/安全 HTML/图片的 Web Clipboard 与 Tauri 读写、降级和 RGBA 转 PNG，以及原生文件复制 IPC |
| 原生文件剪贴板 | `src-tauri/src/file_clipboard.rs` | `src/app/services/clipboard.ts`, `src-tauri/src/lib.rs`, `arboard` | 校验已保存文件路径、写入系统文件列表并保持剪贴板实例存活 |
| 图片 Markdown 路径 | `src/app/services/imageMarkdown.ts` | `src/app/services/imageInsertion.ts` | 图片文件过滤、路径/Markdown 语法生成 |
| 文件存储与文档仓库接口 | `src/lib/services/storage.ts` | `src/lib/desktop/tauriStorage.ts` | FileStorage、DocumentRepository、Markdown 源编码契约变更 |
| 渲染服务类型接口 | `src/lib/services/render.ts` | `src/lib/services/shikiCodeTokenizer.ts`, `katexMathRenderer.ts`, `mermaidDiagramRenderer.ts` | ImageLoader、CodeTokenizer、MathRenderer、DiagramRenderer 接口变更 |
| 文档文件 IO | `src/app/services/documentFiles.ts` | `src/lib/desktop/tauriStorage.ts` | 文件选择框多选、串行批次打开失败收集及文件读取/保存/最近文件/目录树前端调用 |
| 文件夹资源管理 | `src/app/services/folderExplorerController.ts` | `src/app/services/folderTree.ts`, `explorerRows.ts` | 目录树按需加载/展开/串行刷新已加载目录 |
| 目录树纯函数 | `src/app/services/folderTree.ts` | — | 树的归一化/查找/更新 |
| 资源管理器展示 | `src/app/services/explorerRows.ts` | — | 树形拍平为可渲染行 |
| 资源管理器重命名规则 | `src/app/services/explorerRename.ts` | — | 行内重命名输入框选区范围 |
| Rust 文件系统 | `src-tauri/src/file_system.rs` | `src-tauri/src/models.rs` | 后端文件读写、Markdown 编码转换/保持、短句柄目录快照与按需枚举 |
| 图片资源后端 | `src-tauri/src/file_system/image_assets.rs` | — | 图片导入/解析/PicGo 上传/删除 |

### Markdown 统一会话迁移

主应用由 `MarkdownDocumentRuntime.session` / `MarkdownApplicationSession` 装配统一会话，`editor` 门面转发当前局部视图。打开确认后标签只存会话引用与元数据；完整正文通过异步快照读取，门面的同步 `getMarkdown()` / `flushMarkdown()` 会拒绝调用。

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 完整正文、编码与磁盘历史 | `src-tauri/src/text_document/markdown_session.rs` | `markdown_codec.rs`, `markdown_history.rs`, `markdown_commands.rs`, `piece_tree.rs` | 修改原始字节、UTF-8 工作坐标、幂等事务、快照保存、保存意图恢复或历史热缓存；UTF-8 直接复用 raw 基线 |
| 应用会话装配 | `src/app/services/markdownApplicationSession.ts` | `markdownDocumentRuntime.ts`, `App.svelte`, `EditorWorkspace.svelte`, `ComparisonPane.svelte`, `documentActionsController.ts` | 修改打开、模式切换、主动源码定位、宿主展示通知、异步正文命令门禁、图片书签、保存、恢复或双栏/组合标签会话绑定 |
| 跨窗会话归属 | `src-tauri/src/window/tab_transfer.rs` | `markdown_session.rs`, `src-tauri/src/lib.rs`, `workspaceTransferController.ts`, `workspacePersistence.ts` | 修改移交准备、同一提交中的多文档归属、取消恢复、窗口销毁释放、迟到 detach 或工作区会话引用 |
| 前端会话与视图事务 | `src/lib/markdown-document/controller.ts` | `port.ts`, `session.ts`, `utf8Text.ts`, `windowBridge.ts` | 修改待确认输入、重试、保存期间输入、关闭门禁、浏览器内存适配器或视图失效与重定位 |
| 有界语义和原文窗口 | `src/lib/markdown-document/semanticWindow.ts`, `sourceWindow.ts` | `sourceProjection.ts`, `positions.ts`, `textChanges.ts`, `src/lib/editor-core/ProseMirrorEditorCore.ts` | 修改同实例语义预览、初始就绪与统一写入门禁、PM/CM 共用历史、源文精确补丁、混合换行、UTF-8/UTF-16 映射或局部编辑保留 |
| 渐进索引与运行时 | `src/lib/markdown-document/runtime.ts` | `structure.ts`, `structure.worker.ts`, `structureIndex.ts`, `incrementalIndex.ts`, `indexInvalidation.ts` | 修改纯文本结构检查点、可撤销可见结构需求、全局引用、标题 ID 去重、增量收敛、版本门禁、后台取消或加载阶段计时 |
| 全文准入与共享驻留预算 | `src/lib/markdown-document/mountPolicy.ts`, `residentBudget.ts`, `windowingLimits.ts` | `residentTracker.ts`, `enhancements.ts`, `structure.ts`, `structureIndex.ts`, `runtime.ts`, `viewportHost.ts` | 修改窗口化比例（75%–400%，默认 100% 对应 1 MiB / 6000 排版单元）、共享容量派生与原位更新、增强分类统计、同窗语义/源码宿主的可撤销预占与 DOM/图片增量计量、压力锁存和恢复；窗口挂载与回收仍由现有宿主执行 |
| 真实窗口与布局策略 | `src/lib/markdown-document/viewportHost.ts` | `mountWindow.ts`, `layoutCache.ts`, `mountPolicy.ts`, `viewport.ts`, `scheduler.ts`, `editor-document.css` | 修改首屏呈现与覆盖计时、受限结构提示与导航、稳定块组、3–6 屏自适应准备、回收滞回、原位 DOM 更新、交互保护、全局选区、边界导航、大纲挂载后的精确定位与几何高亮、阅读锚点或有界高度/表格布局缓存 |
| 语义窗口标题锚点 | `src/lib/editor-core/plugins/headingAnchors.ts` | `src/lib/editor-core/ProseMirrorEditorCore.ts`, `src/lib/markdown-document/viewportHost.ts` | 修改全局标题 ID 的 PM decoration、跨窗口重名标题定位或大纲点击后的显式滚动；宿主不得直接修改标题 DOM 属性 |
| 全文选区与任务 | `src/lib/markdown-document/selection.ts`, `tasks.ts` | `runtime.ts`, `controller.ts`, `port.ts` | 修改跨窗口选区、完整快照复制、剪切冲突保护、全文搜索替换及取消 |
| 自动目录派生 | `src/lib/markdown-document/automaticToc.ts` | `runtime.ts`, `controller.ts`, `session.ts`, `markdown_session.rs`, `src/lib/toc/tocService.ts` | 修改完整索引物化目录、原始目录保留、派生事务及撤销/重做语义 |
| 实现边界与实机验收 | `docs/markdown-loading.md` | `scripts/generate-markdown-loading-fixtures.mjs`, `src/lib/markdown-document/*.test.ts` | 更新迁移进度、固定文档、代码验证入口或双平台验收记录 |

### 大文件 TXT/JSON 分段编辑

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 分段编辑工作区与全文滚动 | `src/app/components/SegmentedTextEditorWorkspace.svelte` | `src/app/styles/editor-segmented.css`, `src/lib/text-editor/virtualScroll.ts` | 修改 TXT/JSON 视口、全文滚动、快速定位、加载提示或任务 UI |
| CodeMirror 分段核心 | `src/lib/text-editor/SegmentedTextEditorCore.ts` | `positionMapping.ts`, `editBatch.ts`, `jsonLexer.ts` | 修改局部窗口编辑、全局选区/锚点映射、临时只读或历史门禁 |
| 窗口读取、缓存与预取 | `src/lib/text-editor/viewportController.ts` | `chunkCache.ts`, `protocol.ts` | 修改主读取乱序保护、窗口大小、LRU、预览扩展或前后预取 |
| 分段文件后端会话 | `src-tauri/src/text_document/session.rs` | `src-tauri/src/text_document/mod.rs`, `line_index.rs`, `task_runner.rs` | 修改首窗口、窗口读取上限、revision、编辑日志或后台索引/任务 |

### 大纲与导航

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 大纲服务 | `src/lib/outline/outlineService.ts` | `src/lib/outline/writingStats.ts`, `src/lib/editor-core/InlineSourceCodec.ts` | 标题语义内容与 slug、八类行内符号排除、大纲与同步统计兼容入口 |
| 正文统计与范围索引 | `src/lib/outline/writingStats.ts` | `sourceTokenPositions.ts`, `writingStatsEngine.ts`, `writingStatsProtocol.ts` | 中英计数、可见字数、源码/语义选区上下文和索引查询 |
| 后台统计调度 | `src/app/services/writingStatsController.ts` | `src/lib/outline/writingStats.worker.ts`, `src/app/App.svelte` | 120 ms 防抖、单 Worker、快照缓存、失效门禁及统计状态 |
| 章节结构重排 | `src/lib/outline/outlineReorder.ts` | `src/lib/editor-core/editorCommands.ts` | 计算章节子树、落点、层级变化、Markdown 重排与标题索引映射 |
| 大纲交互控制器 | `src/app/services/outlineInteractionController.ts` | `src/app/services/outlineNavigation.ts`, `src/lib/outline/outlineReorder.ts` | 点击定位、章节拖拽编排、源码模式可撤销替换 |
| 大纲滚动定位 | `src/app/services/outlineNavigation.ts`, `src/lib/markdown-document/viewportHost.ts` | `src/lib/outline/outlineViewport.ts`, `src/app/services/markdownApplicationSession.ts`, `src/lib/markdown-document/mountPolicy.ts`, `src/app/services/editorInteractionController.ts` | 会话大纲在语义/源码/双栏按编辑区高度 20%（64–180px）定位与高亮；仅受限窗口模式的未加载目标准备就绪后直跳，其他情况保留固定动画；双栏逐栏判定、原位置保护与过期导航取消；兼容视图的 PM/DOM 标题匹配 |
| Markdown 源码 CodeMirror | `src/app/components/MarkdownSourceEditor.svelte` | `src/app/components/markdownSourceEditor.ts`, `src/app/components/EditorWorkspace.svelte` | 修改源码输入、选区、历史、行坐标、滚动容器或块 spacer |
| 大纲状态 | `src/app/services/outlineState.ts` | — | 大纲展开/折叠/可见性/激活项计算 |
| 大纲浮窗定位 | `src/app/actions/outlinePanelDrag.ts`, `src/app/services/outlinePanelPosition.ts` | `EditorWorkspace.svelte`, `AppTitleBar.svelte`, `appCommands.ts` | 顶部区域拖动、编辑区边界约束、位置记忆与菜单重置；不参与章节重排或正文排版 |
| TOC 服务 | `src/lib/toc/tocService.ts` | `src/lib/editor-core/nodeViews/TocBlockNodeView.ts`, `src/lib/outline/outlineService.ts` | 生成 TOC Markdown/目录项数据，标题字面符号转义与统一跳转 |

### 渲染服务

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 渲染器注册表 | `src/lib/editor-core/renderers.ts` | `src/lib/services/render.ts` | 全局渲染器注册/获取 |
| 代码高亮 | `src/lib/services/shikiCodeTokenizer.ts` | `src/lib/editor-core/nodeViews/CodeBlockNodeView.ts` | Shiki 代码 token 化 |
| 公式渲染 | `src/lib/services/katexMathRenderer.ts` | `src/lib/editor-core/nodeViews/MathBlockNodeView.ts`, `MathInlineNodeView.ts` | KaTeX 公式渲染 |
| 图表渲染 | `src/lib/services/mermaidDiagramRenderer.ts` | `src/lib/editor-core/nodeViews/MermaidBlockNodeView.ts` | Mermaid 图表渲染 |
| 增强渲染调度与可见性 | `src/lib/services/renderTaskQueue.ts`, `src/lib/editor-core/nodeViews/renderVisibility.ts` | 代码块与 Mermaid NodeView、图表渲染器、`plugins/typography.ts`、`markdown-document/viewportHost.ts` | 有界并发、排队取消、视口及后台暂停、宿主导航目标首屏资格与截止预算、结果提交阅读锚点；导出主动补齐 |
| 图表查看与缩放 | `src/lib/services/mermaidDiagramView.ts` | `src/lib/editor-core/nodeViews/MermaidBlockNodeView.ts`, `src/quicklook/preview.ts`, `src/app/services/exportService.ts` | 可读适配、全图/原始尺寸、全屏交互、导出尺寸还原 |
| 图片加载器 | `src/app/services/desktopImageLoader.ts` | `src/lib/editor-core/nodeViews/ImageNodeView.ts` | 本地/asset/远程图片解析 |

### 设置与偏好

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 设置模型与持久化 | `src/app/services/settings.ts` | `src/lib/desktop/tauriStorage.ts` | AppPreferences 定义/默认值/加载/保存 |
| 主题公共契约 | `src/lib/theme/types.ts` | `src/app/services/themeRegistry.ts`, `src/app/services/themeManager.ts`, 编辑器渲染器 | 颜色/样式令牌、样式档案、文档样式或渲染配置契约变更 |
| 主题注册表 | `src/app/services/themeRegistry.ts` | `src/lib/theme/types.ts` | 内置配色、样式档案、文档样式、令牌映射或主题校验变更 |
| 主题运行时 | `src/app/services/themeManager.ts` | `src/app/services/themeRegistry.ts`, `src/lib/editor-core/EditorCore.ts` | 主题解析、根颜色/样式令牌、启动快照、系统同步或运行时刷新变更 |
| 用户自定义主题 | `src/app/services/userThemes.ts`, `src/app/services/userThemeDefinition.ts` | `src/app/services/themeRegistry.ts`, `src/app/services/themeManager.ts`, `src/app/services/settings.ts`, `src-tauri/src/config/themes.rs`, `src/app/components/SettingsWindow.svelte`, `src/quicklook/preview.ts` | 主题 JSON 格式与校验、按基础主题补齐令牌、目录加载与跨窗口重载、导入/删除/导出模板、启动快照内嵌与 Quick Look 读取 |
| 自定义字体覆盖 | `src/lib/theme/fonts.ts` | `src/app/services/themeManager.ts`, `src/app/services/settings.ts`, `src/app/services/fontAvailability.ts`, `src/app/components/SettingsWindow.svelte` | 界面/正文/代码字体名校验、叠加到主题字体令牌、设置页本地字体探测与候选 |
| 编辑器设置应用 | `src/app/services/editorSettingsController.ts` | `src/app/services/settings.ts` | 字体、行高和内容宽度同步到编辑器 |
| 设置窗口 UI | `src/app/components/SettingsWindow.svelte` | `src/app/components/SettingsSelect.svelte`, `src/app/services/settings.ts`, `src/app/services/themeManager.ts`, `src/lib/desktop/tauriUpdater.ts` | 设置界面/主题预览/更新/文件关联/图片配置；SettingsSelect 统一普通下拉与可输入字体候选的主题样式、键盘导航和弹层生命周期 |
| Markdown 格式检查 | `src/app/services/markdownLintController.ts`, `src/app/workers/markdownLint.worker.ts` | `src/lib/markdown-lint/`, `src/app/components/StatusBar.svelte`, `src/app/App.svelte`, `vite.config.ts` | 修改检查调度、Worker 依赖解析、规则集、诊断状态、结果本地化或状态栏详情 |
| Rust 配置管理 | `src-tauri/src/config/mod.rs` | `src-tauri/src/models.rs` | 应用配置 JSON 持久化、设置读写、启动前读取 |
| 用户主题文件存储 | `src-tauri/src/config/themes.rs` | `src/lib/desktop/tauriStorage.ts` | `themes/<id>.json` 列出/保存/删除、外部主题读取与模板写出、ID/大小/数量上限 |

### 搜索与替换

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 搜索替换逻辑 | `src/app/services/searchReplace.ts` | `src/app/components/SearchReplacePanel.svelte` | 搜索/替换算法和状态管理 |
| 搜索替换面板 | `src/app/components/SearchReplacePanel.svelte` | `src/app/services/searchReplace.ts`, `SegmentedTextEditorWorkspace.svelte` | Markdown、TXT、JSON 的统一浮窗布局和选项交互 |
| 全词边界判定 | `src/lib/search/textSearch.ts` | `searchReplace.ts`, `ProseMirrorEditorCore.ts` | Unicode 全词匹配边界规则 |

### 确认对话框

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 确认对话框状态管理 | `src/app/services/confirmAction.ts` | `src/app/components/ConfirmDialog.svelte` | 确认对话框 Promise 模式/三按钮模式变更 |
| 通用确认对话框 | `src/app/components/ConfirmDialog.svelte` | `src/app/services/confirmAction.ts` | 确认/放弃/保存按钮 UI 变更 |
| 未保存确认对话框 | `src/app/components/UnsavedConfirmDialog.svelte` | `src/app/services/confirmAction.ts` | 未保存文档丢弃确认 UI |
| 外部变更对话框 | `src/app/components/ExternalChangeDialog.svelte` | `src/app/services/documentActionsController.ts` | 外部文件修改/删除提示 UI |
| 关闭窗口行为对话框 | `src/app/components/CloseWindowBehaviorDialog.svelte` | `src-tauri/src/window/tray.rs` | 关闭窗口 vs 关闭到托盘选择 UI |

### 平台与首次运行

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 平台检测 | `src/app/services/platform.ts` | `src/app/services/desktopWindow.ts`, `src/app/components/AppTitleBar.svelte` | 平台判断/窗口 chrome 模式变更 |
| 首次运行样本文档 | `src/app/services/firstRunSample.ts` | `src/app/App.svelte` | 首次启动判断/示例文档打开逻辑变更 |
| 应用 UI 状态 | `src/app/services/appUiState.ts` | `src/app/components/ExplorerSidebar.svelte` | 菜单切换/侧边栏 resize 纯函数 |

### 日志与导出

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 前端日志工具 | `src/lib/services/logger.ts` | `src-tauri/src/app_logger.rs` | 日志级别/缓冲区/性能计时/DevTools 输出 |
| 后端日志系统 | `src-tauri/src/app_logger.rs` | `src/lib/services/logger.ts` | 日志文件落盘/轮转/终端输出 |
| HTML 导出后端 | `src-tauri/src/export.rs` | `src/app/services/exportService.ts` | HTML 文件写入/Base64 读取 |
| Windows PDF 导出 | `src-tauri/src/export_windows.rs` | `src-tauri/src/export.rs`, `src/lib/typography/exportRuntime.ts` | WebView2 PrintToPdf、目标版心和排版就绪检查 |
| macOS PDF 导出 | `src-tauri/src/export_macos.rs` | `src-tauri/src/export.rs`, `src/lib/typography/exportRuntime.ts` | WKWebView 模态打印、目标版心和排版就绪检查 |

### 导出

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| HTML/PDF 导出 | `src/app/services/exportService.ts` | `src/app/services/export/`, `src/lib/desktop/tauriStorage.ts`, `src/app/styles/document-content.css`, `src/app/styles/export-document.css` | 冻结文档与外观、独立只读渲染、资源内嵌、HTML 模板或 PDF 打印变更 |
| 导出快照与只读正文 | `src/app/services/export/snapshot.ts`, `src/app/services/export/render.ts` | `export/types.ts`, `export/images.ts`, `src/lib/editor-core/codeLanguages.ts`, `src/lib/editor-core/callout/calloutIcons.ts` | IME/图片插入等待、文档隔离、主题快照、无挂载视图的代码/公式/图表/目录渲染、资源预算与取消 |
| 共用正文样式与打印准备 | `src/app/styles/document-content.css`, `src/lib/typography/exportPrint.ts` | `editor-document.css`, `export-document.css`, `src/lib/typography/exportRuntime.ts` | 编辑器与导出共用展示样式（含内嵌徽章及链接布局）、目标版心、跨页分类、安全表头提升及媒体尺寸约束 |
| Word 导出 | `src/app/services/wordExportService.ts` | `src/app/services/word/`, `src/app/App.svelte`, `src/lib/desktop/tauriStorage.ts`, `src-tauri/src/export.rs` | Markdown 快照、DOCX 内容、资源预算、Worker 打包与二进制原子保存 |
| 导出图片路径解析 | `src/app/services/exportImagePath.ts` | `exportService.ts`, `word/resources.ts` | HTML/PDF/Word 共用本地图片绝对路径与相对路径解析 |

### 本地化

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 前端本地化 | `src/app/i18n.ts` | `src/app/i18n.ja.ts`, `src/paraglide/` | 界面文案/语言切换 |
| 后端本地化 | `src-tauri/src/i18n.rs` | — | 菜单/托盘/系统集成文案 |
| Inlang 生成物 | `src/paraglide/messages*.js` | `project.inlang/messages/*.json` | 运行时语言消息（自动生成，不要手改） |

### Quick Look 预览（macOS）

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| Quick Look 渲染入口 | `src/quicklook/preview-entry.ts` | `src/quicklook/preview.ts` | 读取 payload 并渲染 |
| Quick Look Markdown 渲染 | `src/quicklook/preview.ts` | `src/lib/editor-core/callout/calloutParser.ts`, `src/lib/editor-core/link.ts` | markdown-it 渲染/Callout/公式/图片/链接安全 |
| Quick Look 样式 | `src/quicklook/preview.css` | — | 预览页面样式 |
| macOS 扩展入口 | `src-tauri/macos/NomoQuickLookPreview/PreviewViewController.swift` | `src/quicklook/` | Swift 扩展加载前端并注入 payload |

### 软件更新

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 更新共享协调 | `src/app/services/softwareUpdate.ts` | `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`, `src/lib/desktop/tauriUpdater.ts` | 跨窗口状态订阅、强制检查、下载或安装入口变更 |
| 更新日志安全渲染 | `src/app/services/softwareUpdateReleaseNotes.ts` | `src/app/components/SoftwareUpdateDialog.svelte`, `src/app/components/SoftwareUpdateNotice.svelte` | Release Markdown 过滤、清理、摘要逻辑变更 |
| 更新提醒界面 | `src/app/components/SoftwareUpdateNotice.svelte`, `src/app/components/SoftwareUpdateDialog.svelte` | `src/app/App.svelte`, `src/app/components/AppTitleBar.svelte` | 启动通知、更新日志弹窗、下载状态展示变更 |
| 更新前端 IPC 适配 | `src/lib/desktop/tauriUpdater.ts` | `src/app/services/softwareUpdate.ts` | 检查/下载/安装/共享快照 IPC 契约变更 |
| 更新后端 | `src-tauri/src/software_update.rs` | `src-tauri/src/window/tray.rs` | GitHub Release、安装形态、共享状态、下载/校验/安装器 |
| 升级后首次启动日志 | `src-tauri/src/software_update.rs`, `src/app/components/SoftwareUpdateSuccessDialog.svelte` | `src-tauri/src/lib.rs`, `src/app/App.svelte`, `src/lib/desktop/tauriUpdater.ts` | 内置日志、最高版本与待展示记录、跨窗口领取、实际挂载确认及启动时序 |
| 版本发布与 Pages 同步 | `.github/workflows/release.yml` | `.github/workflows/backfill-linux-release.yml`, `.github/workflows/sync-pages-notes.yml`, `.github/release-notes/` | 安装包发布、已有稳定版的 Linux 补发、MD5 生成、发布说明维护、Cloudflare/GitHub Pages 重建与线上校验 |

### 原生系统集成

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 原生菜单 | `src-tauri/src/window/menu.rs` | `src-tauri/src/i18n.rs` | 菜单构建/快捷键/事件处理 |
| 系统托盘与平台图标 | `src-tauri/src/window/tray.rs` | `src-tauri/src/window/commands.rs` | 托盘安装/刷新/关闭到托盘；仅 macOS 随主题替换窗口和 Dock 图标，其他平台保留打包图标 |
| 外部打开路由 | `src-tauri/src/window/external_open.rs` | `src-tauri/src/lib.rs` | 单实例/启动参数/macOS open 事件、连续请求短暂合并、追加暂存及按窗口原子消费 |
| Windows 文件关联 | `src-tauri/src/window/file_association.rs` | — | 注册/注销默认打开方式和右键菜单 |
| 平台适配 | `src-tauri/src/window/os/macos.rs`, `os/windows.rs` | `src-tauri/src/window/os/mod.rs`, `src-tauri/src/lib.rs` | macOS/Windows 窗口行为差异、Windows 无边框/阴影设置与应用窗口浏览器默认快捷键禁用 |
| 外部链接与附件安全 | `src-tauri/src/external_link.rs` | `src/lib/desktop/tauriStorage.ts` | 打开外部链接、白名单本地附件或文件管理器定位 |
| 配置命令 IPC | `src-tauri/src/config/commands.rs` | `src-tauri/src/config/mod.rs` | 设置读写/最近文件/快照/应用设置 IPC |
| 窗口命令 IPC | `src-tauri/src/window/commands.rs` | `src-tauri/src/window/menu.rs`, `src-tauri/src/window/tray.rs` | 窗口状态保存/设置窗口/菜单安装/强制关闭 IPC |

### 发布与打包

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| Linux 本地构建与安装 | `scripts/build-linux.sh` | `src-tauri/tauri.conf.json` | 修改 DEB/RPM 构建、产物定位或安装；DEB 优先通过 apt-get 解析依赖，失败时保留错误 |
| SVG 图标与平台资源生成 | `scripts/generate-icons.mjs` | `src-tauri/icons/nomo/source/nomo-app-light.svg`, `scripts/compile-macos-appicon.sh`, `src-tauri/src/window/tray.rs` | 修改应用 Logo、浅深色配色、平台边距、多尺寸位图或托盘图标；浅色 SVG 为造型母版，桌面与系统文件图标单独派生描边，应用内 Logo 和移动端保持母版造型 |
| Windows Store MSIX 构建与校验 | `scripts/msix/Build-Msix.ps1` | `scripts/msix/Build-IconResources.ps1`, `.github/workflows/msix.yml`, `src-tauri/msix/`, `src-tauri/tauri.conf.json` | 修改 Store 包身份、版本映射、生产前端嵌入校验、图标 PRI 生成、Shell 扩展封装或上传产物 |

### UI 通用组件

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 通用上下文菜单 | `src/app/components/ContextMenu.svelte` | `src/lib/editor-core/plugins/contextMenu.ts` | 右键菜单 UI 渲染/定位 |
| 状态栏 | `src/app/components/StatusBar.svelte` | `src/lib/outline/outlineService.ts`, `src/lib/markdown-lint/`, `src/app/styles/editor-outline.css` | 字数统计、缩放百分比或 Markdown 格式检查状态与详情展示 |
| Front Matter 卡片 | `src/app/components/FrontMatterCard.svelte` | `src/lib/markdown/frontMatter.ts` | YAML 元数据展示/编辑/删除 |
| 空工作区 | `src/app/components/EmptyWorkspace.svelte` | — | 无文档时的新建/打开引导 |
| 文件夹打开对话框 | `src/app/components/FolderOpenDialog.svelte` | `src/app/services/folderExplorerController.ts` | 打开文件夹窗口选择 UI |
| 链接快速编辑器 | `src/app/components/LinkQuickEditor.svelte` | `src/lib/editor-core/plugins/linkInteraction.ts` | 链接文字/地址编辑弹出层 |
| Windows 窗口控制按钮 | `src/app/components/WindowsCaptionControls.svelte` | `AppTitleBar.svelte`, `SettingsWindow.svelte`, `src/app/services/platform.ts` | 最小化、最大化/还原、关闭/返回按钮及状态同步 |

### Svelte Actions

| Responsibility | Primary code | Related code | Change when |
|---|---|---|---|
| 点击外部检测 | `src/app/actions/clickOutside.ts` | `ContextMenu.svelte`, `FrontMatterCard.svelte`, `StatusBar.svelte` | 下拉菜单/弹出层外部点击关闭 |
| 过渡动画 | `src/app/actions/motion.ts` | 多个对话框/弹出层组件 | fade/slide 动画统一配置 |

---

## Code Unit Entries

### `src/main.ts`

**Kind:** entry

**Owns:**
- 前端应用入口挂载逻辑
- 根据 URL 参数 (`view=settings`) 决定加载主应用或设置窗口
- 在动态加载窗口组件前同步应用经过校验的主题启动快照
- 全局样式（theme.css, global.css）加载
- KaTeX 和 ProseMirror 样式在 main 视图下按需加载
- 安装全局滚动条显隐交互并在卸载时清理监听器

**Does not own:**
- 不拥有具体业务组件逻辑（委派给 App.svelte / SettingsWindow.svelte）
- 不拥有编辑器初始化

**Called by:** `index.html`

**Depends on:** `src/app/services/themeManager.ts`, `src/app/services/scrollbarVisibility.ts`, `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`, `src/lib/services/logger.ts`

**Change this when:**
- 添加新的入口视图
- 修改全局样式加载顺序
- 调整入口挂载逻辑
- 修改页面级全局交互的安装与清理

**Do not change this when:**
- 修改具体业务组件行为
- 修改编辑器功能

**Related tests:** —

**Confidence:** high

---

### `src/app/services/scrollbarVisibility.ts`

**Kind:** DOM interaction service

**Owns:**
- 捕获任意滚动容器的滚动事件，并在停止滚动后延时隐藏滚动条
- 逐帧判断鼠标是否接近可滚动区域的右侧或底部边缘
- 管理 `is-scrollbar-visible` 状态类、窗口失焦清理和全局监听器生命周期

**Does not own:**
- 不拥有滚动条颜色、宽度或具体视觉样式（在全局及局部 CSS 中）
- 不改变滚动位置、编辑器内容或窗口模式

**Called by:** `src/main.ts`

**Depends on:** Browser DOM APIs

**Change this when:**
- 修改全局滚动条的触发区域、显隐时机或交互清理规则

**Related tests:** —

**Confidence:** high

---

### `src/app/App.svelte`

**Kind:** component / app shell

**Owns:**
- 应用核心装配：连接 Tauri、编辑器核心、文件系统、设置、标签页
- 初始化渲染服务（Shiki、KaTeX、Mermaid、图片加载器）
- 按文档 ID 创建、挂起和恢复 `EditorCore` 运行时，保留窗口内编辑历史和阅读位置
- 新建空白标签在对应运行时挂载后聚焦正文一次，切换文档取消过期聚焦
- 加载设置和 v4 工作区状态，协调旧版迁移、组合恢复、草稿恢复与启动冲突选择
- 协调主题运行时初始化、系统明暗同步、快捷键切换和保存失败回滚
- 协调打开、保存、自动保存、模式切换、外部文件打开、关闭确认
- 统一路由菜单、最近记录、系统文件管理器和文件树打开请求，并串行协调空窗口复用与跨窗口去重
- 订阅编辑器内容变化并同步 dirty/统计/大纲状态
- 工作区恢复后发起一次启动更新检查，并按目标窗口协调通知卡片与安装确认
- 协调当前文档在同一窗口内进入/返回 Markdown 小窗，按最终尺寸恢复阅读位置并统一收尾过渡，复用原编辑器、撤销栈和自动保存状态
- 统一持有全应用上下文菜单状态，并为各 UI 区域提供单一打开入口、剪贴板和路径定位动作
- 快捷插入的图片/链接选择窗口与来源文档、选区及内容修订校验

**Does not own:**
- 不拥有具体 UI 子组件渲染逻辑（委派给 AppShell.svelte）
- 不拥有编辑器内部实现细节（通过 EditorCore API 交互）
- 不拥有文件系统直接 IO（通过 documentFiles.ts 调用）

**Called by:** `src/main.ts`

**Depends on:** `src/app/services/*`, `src/lib/editor-core/*`, `src/lib/desktop/*`, `src/lib/services/*`, `src/app/components/AppShell.svelte`

**Change this when:**
- 添加新的全局事件监听
- 修改编辑器初始化流程
- 修改文件打开/保存/自动保存协调逻辑
- 修改工作区恢复、草稿冲突选择、标签页恢复流程
- 修改标签页管理流程
- 修改 Markdown 小窗进入、返回、置顶或快捷键协调

**Do not change this when:**
- 修改纯 UI 组件内部样式或布局
- 修改编辑器内部 ProseMirror 逻辑

**Related tests:** `src/app/App.layout.test.ts`

**Confidence:** high

---

### `src/app/components/AppShell.svelte`

**Kind:** component

**Owns:**
- 应用顶层布局：标题栏、侧边栏、标签栏、编辑区、状态栏、对话框
- 通过 props 和回调将 App.svelte 的状态下发给子组件
- Markdown 工具栏的收展区域和右侧展开柄
- 装配工作区联合轮廓，并转发活动标签几何与主题更新键
- 装配双文档 ComparisonWorkspace、成员运行时与焦点回调，并禁用组合的小窗 / 同文档双栏入口
- 在 Markdown 小窗模式下隐藏常规 chrome，提供根元素引用和切换输入门禁，并在可编辑编辑器与大文档只读预览之间选择内容视图
- 挂载全窗口右键菜单兜底策略，并把统一菜单入口下发给标题栏、文件树、标签栏和大纲

**Does not own:**
- 不拥有业务状态管理（由 App.svelte 传入）
- 不拥有具体编辑器逻辑

**Called by:** `src/app/App.svelte`

**Depends on:** `AppTitleBar.svelte`, `MarkdownMiniLargePreview.svelte`, `ExplorerSidebar.svelte`, `DocumentTabs.svelte`, `WorkspaceSurface.svelte`, `EditorToolbar.svelte`, `EditorWorkspace.svelte`, `ComparisonWorkspace.svelte`, `StatusBar.svelte`, `ConfirmDialog.svelte`, `CloseWindowBehaviorDialog.svelte`, `FolderOpenDialog.svelte`, `EmptyWorkspace.svelte`, `SearchReplacePanel.svelte`

**Change this when:**
- 调整整体应用布局结构
- 添加/移除顶层 UI 区域

**Do not change this when：**
- 修改具体业务逻辑
- 修改编辑器内部行为

**Related tests:** `src/app/App.layout.test.ts`

**Confidence:** high

---

### `src/app/components/MarkdownMiniLargePreview.svelte`

**Kind:** component / read-only renderer

**Owns:**
- 大文档小窗的安全只读 HTML 渲染
- 延迟加载代码高亮与 Mermaid 渲染器，并在主题变化时刷新预览
- 正文渲染完成前保持轻量加载状态
- 原生窗口切换期间延后精细排版和高亮增强，过渡结束后恢复

**Does not own:**
- 不拥有普通文档编辑器
- 不决定窗口状态、文件保存或外部冲突处理

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/quicklook/preview.ts`, `src/lib/editor-core/renderers.ts`

**Change this when:**
- 修改大文档小窗的只读渲染、加载状态或主题刷新行为

**Related tests:** —

**Confidence:** high

---

### `src/app/components/SettingsWindow.svelte`

**Kind:** component

**Owns:**
- 设置中心 UI：通用、编辑器、外观、文件、图片、统计、高级、关于等设置页
- 加载/保存 `AppPreferences`
- 提供明暗模式分段控制、内置主题颜色/造型预览卡片、即时跨窗口预览与防抖保存
- 软件更新、文件关联、右键菜单、图片上传配置等设置项交互
- Windows 设置窗口标题栏与共享自绘窗口按钮的接入

**Does not own:**
- 不拥有设置持久化后端逻辑（通过 settings.ts）
- 不拥有软件更新共享状态和后端（通过 softwareUpdate.ts / tauriUpdater.ts）

**Called by:** `src/main.ts`（当 `view=settings` 时）

**Depends on:** `src/app/services/settings.ts`, `src/app/services/themeManager.ts`, `src/app/services/softwareUpdate.ts`, `src/lib/desktop/tauriStorage.ts`, `src/app/i18n.ts`, `WindowsCaptionControls.svelte`

**Change this when:**
- 添加新的设置项 UI
- 修改设置分类布局
- 修改设置项交互方式

**Do not change this when:**
- 修改设置模型结构（在 settings.ts 中）
- 修改后端配置结构（在 config/mod.rs 中）

**Related tests:** —

**Confidence:** high

---

### `src/app/components/SoftwareUpdateNotice.svelte`

**Kind:** component

**Owns:**
- 12 秒自动收起的启动更新通知卡片
- 查看、稍后提醒和关闭当前版本提醒三个入口

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/i18n.ts`, `@lucide/svelte`

**Change this when:** 修改更新卡片视觉、计时或按钮交互

**Confidence:** high

---

### `src/app/components/SoftwareUpdateSuccessDialog.svelte`

**Kind:** component

**Owns:**
- 升级后的当前版本日志弹窗、开始使用/关闭、焦点约束及恢复
- 实际挂载后的展示确认回调；不提供下载或安装动作

**Does not own:** 版本记录与跨窗口领取（在 software_update.rs）、Markdown 安全策略（在 softwareUpdateReleaseNotes.ts）

**Called by:** `src/app/App.svelte`（启动恢复完成且已有确认弹窗关闭后）

**Depends on:** `src/app/services/softwareUpdateReleaseNotes.ts`, `src/lib/desktop/tauriStorage.ts`, `src/app/i18n.ts`

**Change this when:** 修改升级完成页的布局、关闭行为或实际展示回调

**Confidence:** high

---

### `src/app/components/SoftwareUpdateDialog.svelte`

**Kind:** component

**Owns:**
- 更新日志弹窗、下载进度、安装版与免安装版操作区
- 遮罩、Escape、外部 HTTPS 链接交互

**Does not own:** Release Markdown 安全策略（在 softwareUpdateReleaseNotes.ts）

**Called by:** `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`

**Depends on:** `src/app/services/softwareUpdateReleaseNotes.ts`, `src/lib/desktop/tauriStorage.ts`, `src/app/i18n.ts`

**Change this when:** 修改更新弹窗布局或状态动作

**Confidence:** high

---

### `src/app/components/ExplorerSidebar.svelte`

**Kind:** component

**Owns:**
- 资源管理器侧边栏 UI
- 目录树、最近文件、行内重命名、创建文件/文件夹，以及节点、根目录、单文件和空白区域菜单项
- 文件节点与单文件视图提供“复制文件”，经 App.svelte 调用原生文件剪贴板；浏览器和未保存文件禁用
- 当前文件选中底板及其虚拟树坐标滑动动画
- 侧栏隐藏时的键盘焦点与无障碍交互隔离
- 已展开节点的键入定位、临时匹配提示与中文输入法接收，不触发目录扫描或正文搜索
- 侧边栏 resize 逻辑

**Does not own:**
- 不拥有目录树数据管理（由 folderExplorerController.ts 提供）
- 不拥有文件系统直接操作

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/services/explorerRows.ts`, `src/app/services/explorerRename.ts`, `src/app/actions/motion.ts`, `src/lib/editor-core/plugins/contextMenu.ts`, `src/app/types.ts`

**Change this when:**
- 修改侧边栏布局或交互
- 修改目录树行展示逻辑
- 修改右键菜单项

**Do not change this when：**
- 修改目录扫描后端逻辑
- 修改文件树数据结构定义

**Related tests:** `src/app/App.layout.test.ts`

**Confidence:** high

---

---

### `src/app/components/AppTitleBar.svelte`

**Kind:** component

**Owns:**
- Nomo 标题栏内容、Windows 应用内菜单及共享自绘窗口按钮的接入。
- 将菜单点击转换为应用命令或调用传入的业务处理函数。
- Markdown 小窗的文件名、冲突/只读状态、置顶和返回按钮，以及覆盖整行的单一路径拖动区域。
- Windows 自绘标题栏拖动区的最小化、最大化/还原、关闭及小窗视图菜单项。

**Does not own：**
- 不拥有具体业务逻辑（由 App.svelte 通过 props 注入）。
- 不拥有 macOS 原生菜单（在 Rust `window/menu.rs` 中）。
- 不拥有 Windows 无边框窗口、阴影与缩放边界（由 Tauri/Tao 及 `os/windows.rs` 负责）。

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/i18n.ts`, `src/app/services/platform.ts`, `src/app/services/outlinePanelPosition.ts`, `WindowsCaptionControls.svelte`, `@lucide/svelte`

**Change this when：**
- 添加/移除自定义标题栏菜单项。
- 修改菜单快捷键展示文案。
- 修改 Markdown 小窗标题栏信息、按钮或拖动区域。

**Do not change this when：**
- 修改菜单命令后端处理逻辑。

**Related tests:** —

**Confidence:** high

---

### `src/app/components/WindowsCaptionControls.svelte`

**Kind:** component

**Owns:**
- Windows 自绘最小化、最大化/还原和关闭/返回按钮。
- 当前窗口最大化状态查询、resize 后图标同步和监听器清理。
- Windows 风格 hover、active、focus 与关闭按钮危险态样式。

**Does not own:**
- 不决定关闭业务；关闭或返回操作始终调用父组件传入的 `onClose`。
- 不拥有窗口拖动、原生阴影、缩放边界或 Snap 行为。

**Called by:** `AppTitleBar.svelte`, `SettingsWindow.svelte`

**Depends on:** `@tauri-apps/api/window`, `src/app/i18n.ts`, `src/lib/services/logger.ts`

**Change this when:** 修改 Windows 窗口按钮行为、图标、状态同步或交互样式。

**Related tests:** `src/app/App.layout.test.ts`

**Confidence:** high

---

### `src/app/components/EditorWorkspace.svelte`

**Kind:** component

**Owns:**
- 编辑工作区 UI：CodeMirror 源码编辑器、ProseMirror 挂载点
- 可编辑纸面左右、上下和文末留白点击折叠选区并按坐标定位；在段落侧栏整选之后处理，排除控件、修饰键、触摸和滚动条
- 各栏自然高度、适量尾部阅读空间、30%参考线与模式切换可测量就绪事件的协调
- 为独立同步 action 提供当前文档、模式、分栏拖动状态与两个编辑器句柄
- Front Matter 卡片、大纲面板
- 大纲标题与空白区域的导航、展开/折叠、复制标题和隐藏大纲菜单项
- 大纲整行 Pointer 拖拽状态机、三区落点提示、延时展开与边缘滚动
- 大纲顶部区域（含标题及周围留白）通过 `src/app/actions/outlinePanelDrag.ts` 移动面板；`src/app/services/outlinePanelPosition.ts` 保存编辑区内偏移并提供统一重置入口，拖动和归位沿用同一正文避让规则，不改变正文排版
- 外部变更提示
- 模式切换（语义编辑/源码编辑/只读/大文档提示）

**Does not own:**
- 不拥有编辑器核心实现（由 EditorCore 管理）
- 不拥有大纲数据计算（由 outlineService.ts 提供）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `FrontMatterCard.svelte`, `MarkdownSourceEditor.svelte`, `src/app/services/markdownScrollSyncWorkspace.ts`, `src/lib/editor-core/types.ts`, `src/lib/outline/outlineReorder.ts`, `src/app/actions/outlinePanelDrag.ts`, `src/app/services/outlinePanelPosition.ts`

**Change this when：**
- 修改编辑区布局
- 修改模式切换 UI 行为
- 修改大纲面板展示
- 修改大纲拖拽手势和落点反馈

**Do not change this when：**
- 修改 ProseMirror 内部逻辑
- 修改 Markdown 解析规则

**Related tests:** `src/app/components/EditorWorkspace.test.ts`, `src/app/services/markdownScrollSync.test.ts`

**Confidence:** high

---

### `src/app/components/MarkdownSourceEditor.svelte`

**Kind:** component

**Owns:**
- Markdown 源码的单一 CodeMirror 6 `EditorView`、输入同步、只读态和 IME 生命周期
- 首次进入源码或双栏时由工作区挂载；缓存快照只保留文档、选区与历史，重挂载时绑定当前 UI，静态主题按需复用以避免 CSS 规则累积
- 源码选区、视口坐标光标定位、聚焦、范围显示、undo/redo、行/offset 与滚动 DOM 的 handle；坐标定位不改变正文和历史，保护只读与组字态
- 统一逻辑行比较，重复更新不发事务；通过 `markdownSourceEditor.ts#getSourceTextChanges` 按行和字符细分多处差异，一次事务保留中间未改动文本的选区与视口锚点
- 当前内容的测量就绪修订，等待 CodeMirror 视口锚点调整后通知双栏跟随
- 不改 Markdown offset 和 history 的块级 CodeMirror widget spacer，以及测量完成信号

**Does not own:**
- 不决定语义块两侧应补多少高度（由 `markdownBlockAlignment.ts` 计算）
- 不写入文件、脏状态或持久化偏好

**Called by:** `src/app/components/EditorWorkspace.svelte`

**Depends on:** `@codemirror/state`, `@codemirror/view`, `@codemirror/commands`, `markdownSourceEditor.ts`

**Related tests:** `src/app/components/MarkdownSourceEditor.test.ts`

**Confidence:** high

---

### `src/app/services/markdownBlockAlignment.ts`

**Kind:** service

**Owns:**
- Front Matter、顶层 Markdown 块和 EOF 合成块的稳定 alignment anchor
- 扣除已有虚拟间距后的自然推进高度比较与两侧 gap 计算
- 1px 写入容差、映射不完整时的 fallback 结果

**Does not own:**
- 不读取 DOM、不插入 decoration、不控制模式动画

**Called by:** `src/app/components/EditorWorkspace.svelte`

**Depends on:** `src/lib/editor-core/markdown.ts`, `src/lib/markdown/frontMatter.ts`

**Related tests:** `src/app/services/markdownBlockAlignment.test.ts`

**Confidence:** high

---

### `src/app/components/WorkspaceSurface.svelte`

**Kind:** component (SVG decoration)

**Owns:**
- 工作区与活动标签共用的闭合外轮廓、主题描边及两层柔和投影
- 工作区尺寸与圆角测量，以及凹圆角靠近边缘时的收缩
- 空工作区普通面板轮廓与减少动画偏好下的颜色过渡降级

**Does not own:**
- 不拥有标签业务状态、切换动画或编辑内容裁剪
- 不修改公共主题契约或持久化设置

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `TabIndicatorGeometry`（`src/app/actions/motion.ts` 的类型）、现有主题 CSS 变量

**Change this when:** 修改联合轮廓、描边、投影或装饰层尺寸同步

**Related tests:** —；视觉效果需实窗验收

**Confidence:** high

---

### `src/app/components/DocumentTabs.svelte`

**Kind:** component

**Owns:**
- 标签页 UI：展示打开文档、切换标签、关闭标签
- 活动标签指示器的定位与 GSAP 滑动动画，以及相对工作区的逐帧几何回调
- 固定预览标签状态
- 投影 single / comparison 外层项、组合菜单、Pointer 起点、可见标签落点与插入 / 成组提示
- 标签项及标签栏空白区域的菜单项，并通过应用级菜单入口显示

**Does not own:**
- 不拥有文档或外层项状态管理（由 tabs.ts、workspaceItems.ts 和 App.svelte 管理）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/lib/editor-core/plugins/contextMenu.ts`, `src/app/actions/motion.ts`, `src/app/types.ts`

**Change this when：**
- 修改标签页展示样式
- 修改标签切换/关闭交互

**Do not change this when：**
- 修改标签页数据管理逻辑

**Related tests:** —

**Confidence:** high

---

### `src/app/components/EditorToolbar.svelte`

**Kind:** component

**Owns:**
- 编辑工具栏 UI：标题、行内格式、列表、表格、公式、图表等命令按钮
- 将按钮操作转换为 `EditorCommand` 传给编辑器核心
- 按编辑区实际宽度分级隐藏操作按钮，并提供紧凑内容宽度面板和收起入口
- 双栏30%同步参考线的运行期开关（默认隐藏）

**Does not own：**
- 不拥有命令具体实现（在 editorCommands.ts 中）
- 不拥有图表模板定义（在 diagramTemplates.ts 中）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/lib/editor-core/types.ts`, `src/lib/editor-core/diagramTemplates.ts`

**Change this when：**
- 添加新的工具栏按钮
- 修改工具栏布局
- 修改命令触发方式

**Do not change this when：**
- 修改命令内部实现逻辑

**Related tests:** `src/app/services/appCommands.test.ts`

**Confidence:** high

---

### `src/app/components/SearchReplacePanel.svelte`

**Kind:** component

**Owns:**
- 搜索/替换面板 UI
- 搜索输入、替换输入、选项控制
- Markdown、TXT、JSON 共用浮窗向 `document.body` 的 Portal 挂载与销毁
- 浮窗拖拽、位置持久化，以及按真实尺寸处理打开、缩放和内容变化时的视口约束

**Does not own：**
- 不拥有搜索替换算法（在 searchReplace.ts 中）
- 不拥有编辑器内高亮逻辑（在 searchHighlight.ts 中）

**Called by:** `src/app/components/AppShell.svelte`, `src/app/components/SegmentedTextEditorWorkspace.svelte`

**Depends on:** 浏览器 DOM、Pointer Events、ResizeObserver

**Change this when：**
- 修改搜索替换面板布局
- 修改搜索选项交互
- 修改浮窗层级、拖拽或位置恢复规则

**Do not change this when：**
- 修改搜索替换核心算法

**Related tests:** `src/app/services/searchReplace.test.ts`

**Confidence:** high

---

### `src/app/services/documentActionsController.ts`

**Kind:** controller

**Owns:**
- 文档操作协调：打开、拖拽打开、保存、另存、自动保存
- 外部文件变化检测与处理
- 资源管理器重命名期间跳过受影响路径检测，并丢弃旧路径检测结果
- 标签页文档状态切换
- 恢复草稿管理

**Does not own：**
- 不拥有文件系统直接 IO（通过 documentFiles.ts）
- 不拥有标签页纯函数操作（通过 tabs.ts）
- 不拥有大纲计算（通过 outlineService.ts）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/documentFiles.ts`, `src/app/services/tabs.ts`, `src/lib/markdown/normalize.ts`, `src/lib/outline/outlineService.ts`, `src/app/services/recoveryDraft.ts`

**Change this when：**
- 修改打开/保存/自动保存流程
- 修改外部文件变化处理逻辑
- 修改草稿恢复逻辑

**Do not change this when：**
- 修改文件系统后端实现
- 修改编辑器内部行为

**Related tests:** `src/app/services/externalFileChangeFlow.test.ts`

**Confidence:** high

---

### `src/app/services/documentLinkNavigation.ts`

**Kind:** service

**Owns:**
- 编辑器链接分类：外部 URL、当前标题锚点、Nomo 文档与白名单附件
- 以当前 Markdown 文件目录解析相对路径，并分离 URI fragment
- 本地链接的协议、查询参数、UNC 路径和扩展名边界

**Does not own:**
- 不直接打开标签页或系统应用（由 `App.svelte` 与桌面 IPC 协调）
- 不检查目标是否真实存在（由文档读取端口或附件后端命令校验）

**Called by:** `src/app/App.svelte`

**Depends on:** `@tauri-apps/api/path`, `src/app/services/tabs.ts`, `src/app/types.ts`

**Change this when:**
- 修改相对路径解析、支持的本地链接类型或 fragment 分类规则

**Related tests:** `src/app/components/EditorWorkspace.test.ts`

**Confidence:** high

---

### `src/app/services/markdownScrollSync.ts`

**Kind:** controller

**Owns:** 滚动主栏与编辑意图区分、代码块局部操作接管并取消旧定位、30%参考线插值、光标15%～85%安全区、程序滚动抑制、修订切换与单帧任务取消。

**Does not own:** 不解析 Markdown，不访问 ProseMirror 内部，不改正文高度、选区、焦点或历史。

**Called by:** `src/app/services/markdownScrollSyncWorkspace.ts`

**Related tests:** `src/app/services/markdownScrollSync.test.ts`

**Confidence:** high

---

### `src/app/services/markdownScrollSyncWorkspace.ts`

**Kind:** Svelte action

**Owns:** 工作区生命周期、兼容 CRLF/LF 的内容一致性门控及修订/文本缓存、源码测量就绪门控、CodeMirror/EditorCore 测量适配、滚动容器坐标换算、布局失效、输入/拖动/组合事件；不绘制跨栏光标标记。

**Does not own:** 不修改 Markdown 或提交 NodeView 草稿，不计算等高补偿。

**Called by:** `src/app/components/EditorWorkspace.svelte`

**Depends on:** `markdownScrollSync.ts`, `EditorCore`, `MarkdownSourceEditorHandle`

**Related tests:** `src/app/components/EditorWorkspace.test.ts`, `src/app/services/markdownScrollSync.test.ts`, `src/app/services/markdownScrollSyncWorkspace.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/scrollSyncMapping.ts`

**Kind:** parser instrumentation

**Owns:** 包装同一 MarkdownParser 的实际 token handler，记录真正创建的节点及嵌套源码来源，生成块边界和代码逻辑行锚点；定义只属于当前修订的只读同步数据。

**Does not own:** 不更改解析规则、持久化节点属性或历史，不测量 DOM。

**Called by:** `src/lib/editor-core/markdown.ts`

**Related tests:** `src/lib/editor-core/scrollSyncMapping.test.ts`

**Confidence:** high

---

### `src/app/services/editorInteractionController.ts`

**Kind:** controller

**Owns:**
- 编辑/源码模式切换
- 滚动锚点恢复（按大纲锚点恢复视觉焦点）
- 等待目标编辑器可测量，并用 generation 取消过期切换，不等待全文等高收敛
- 通过 MarkdownSourceEditor handle 执行源码 undo/redo、输入同步和 TOC 插入

**Does not own：**
- 不拥有编辑器核心创建（在 App.svelte 中）
- 不拥有大纲数据计算（通过 outlineNavigation.ts）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/editor-core/types.ts`, `src/app/services/outlineNavigation.ts`, `src/lib/toc/tocService.ts`

**Change this when：**
- 修改模式切换逻辑
- 修改滚动恢复行为
- 修改 TOC 插入流程

**Do not change this when：**
- 修改编辑器核心实现

**Related tests:** `src/app/services/editorInteractionController.test.ts`

**Confidence:** high

---

### `src/app/services/folderExplorerController.ts`

**Kind:** controller

**Owns:**
- 文件夹资源管理：加载文件夹、展开祖先目录
- 懒加载子目录
- 合并并同步已加载目录的重叠刷新
- 新建、删除、重命名和手动刷新保留展开状态；用户操作等待后台刷新后读取新快照

**Does not own：**
- 不拥有目录树纯函数（在 folderTree.ts 中）
- 不拥有展示行拍平（在 explorerRows.ts 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/documentFiles.ts`, `src/app/services/folderTree.ts`, `src/lib/desktop/tauriStorage.ts`

**Change this when：**
- 修改文件夹加载逻辑
- 修改目录展开/折叠行为
- 修改已加载目录的刷新协调

**Do not change this when：**
- 修改目录树纯数据结构操作

**Related tests:** `src/app/services/folderExplorerController.test.ts`

**Confidence:** high

---

### `src/app/services/explorerRename.ts`

**Kind:** service / pure helper

**Owns:**
- 资源管理器行内重命名输入框的默认选区范围
- 文件夹全选、文件默认保留最后扩展名的规则

**Does not own：**
- 不拥有重命名输入框 DOM 挂载与焦点时序（在 ExplorerSidebar.svelte 中）
- 不拥有文件系统重命名落盘

**Called by:** `src/app/components/ExplorerSidebar.svelte`

**Depends on:** —

**Change this when：**
- 修改资源管理器重命名时默认选中文本的规则

**Do not change this when：**
- 修改右键菜单、输入框布局或焦点时序
- 修改后端文件重命名逻辑

**Related tests:** `src/app/services/explorerRename.test.ts`

**Confidence:** high

---

### `src/app/services/settings.ts`

**Kind:** model / service

**Owns:**
- `AppPreferences` 定义与默认值
- 设置归一化、加载、保存和外观模型一次性迁移
- 无效主题、文档样式与旧字段映射的持久化修复
- 工具栏显示偏好与可自定义快捷键的默认值和持久化
- 快捷插入开关和单个触发符号的默认值、校验、加载与持久化
- 统一打开行为偏好的新键持久化，以及旧 `folderOpenDefaultBehavior` 的读取兼容

**Does not own：**
- 不拥有设置 UI（在 SettingsWindow.svelte 中）
- 不拥有后端配置存储直接操作（通过 tauriStorage.ts）

**Called by:** `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`, `src/app/services/editorSettingsController.ts`

**Depends on:** `src/app/services/themeRegistry.ts`, `src/lib/desktop/tauriStorage.ts`, `src/lib/editor-core/quickInsert.ts`

**Change this when：**
- 添加新的设置项
- 修改设置默认值
- 修改设置归一化逻辑
- 修改外观设置迁移或持久化修复逻辑

**Do not change this when：**
- 修改设置 UI 展示方式
- 修改后端配置结构（在 config/mod.rs 中）

**Related tests:** `src/app/services/settings.test.ts`

**Confidence:** high

---

### `src/lib/theme/types.ts`

**Kind:** public contract

**Owns:**
- 明暗模式、有效配色、外观偏好、颜色令牌、主题变体和文档样式的公共类型
- Shiki 与 Mermaid 的主题渲染配置契约

**Does not own:**
- 不拥有具体主题色值或注册逻辑
- 不拥有主题运行时应用与持久化

**Called by:** 主题注册表、主题管理器、设置模型、EditorCore 和代码/图表渲染器

**Change this when:** 修改 v1 主题契约、必填令牌或渲染器主题配置

**Confidence:** high

---

### `src/app/services/themeRegistry.ts`

**Kind:** registry

**Owns:**
- `nomo-default`、`nomo-amber-paper`、`nomo-classic-gray`、`nomo-github` 的亮暗变体、样式档案和预览
- `nomo-classic`、`nomo-modern` 文档样式稳定 ID
- 必填颜色/样式令牌到 CSS 变量的映射与主题完整性校验
- 内置与用户主题的区分：用户主题可替换、注销，内置主题不可

**Does not own:**
- 不拥有用户当前选择或 DOM 应用
- 不拥有配置持久化

**Called by:** `src/app/services/themeManager.ts`, `src/app/services/settings.ts`, `src/app/components/SettingsWindow.svelte`

**Depends on:** `src/lib/theme/types.ts`

**Change this when:** 新增内置主题、调整主题令牌、文档样式或注册校验

**Related tests:** `src/app/services/themeRegistry.test.ts`

**Confidence:** high

---

### `src/app/services/themeManager.ts`

**Kind:** service

**Owns:**
- 外观选择归一化、有效明暗解析和无效值回退
- 根节点主题属性、样式档案及颜色/样式令牌应用
- EditorCore、窗口图标和跨窗口运行时主题刷新
- `ThemeBootSnapshot` v2 校验、v1 兼容读取、读写与首屏同步应用（含可缺省的字体覆盖字段）
- 字体覆盖与主题样式令牌合并，字体令牌变化经排版交互通道写入以触发正文重排
- 当前为用户主题时在启动快照内嵌主题源文，首屏在读取主题目录前恢复
- `matchMedia`、窗口聚焦和页面恢复时的系统主题同步

**Does not own:**
- 不拥有内置主题定义（在 themeRegistry.ts 中）
- 不拥有 config.json 持久化（在 settings.ts 中）
- 不拥有设置界面

**Called by:** `src/main.ts`, `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`

**Depends on:** `src/app/services/themeRegistry.ts`, `src/lib/editor-core/EditorCore.ts`, `src/app/services/desktopWindow.ts`

**Change this when:** 修改主题解析、启动快照、系统同步、根令牌或运行时刷新

**Related tests:** `src/app/services/themeManager.test.ts`

**Confidence:** high

---

### `src/app/services/softwareUpdate.ts`

**Kind:** service / store

**Owns:**
- 当前 WebView 对进程级更新快照的订阅和 Svelte store
- 检查、下载、安装命令的统一调用与状态刷新

**Does not own:** 更新候选选择、下载缓存或安装器启动

**Called by:** `src/app/App.svelte`, `src/app/components/SettingsWindow.svelte`

**Depends on:** `src/lib/desktop/tauriUpdater.ts`

**Change this when:** 修改前端共享更新状态或命令协调流程

**Confidence:** high

---

### `src/app/services/softwareUpdateReleaseNotes.ts`

**Kind:** service

**Owns:**
- Release 正文的安装包章节裁剪
- 禁用原始 HTML/图片、仅保留 HTTPS 链接的 Markdown 安全渲染
- 通知卡片摘要提取

**Called by:** `src/app/components/SoftwareUpdateDialog.svelte`, `src/app/components/SoftwareUpdateSuccessDialog.svelte`, `src/app/App.svelte`

**Depends on:** `markdown-it`, DOM API

**Change this when:** 修改更新日志过滤、允许标签或摘要规则

**Confidence:** high

---

### `src/app/services/searchReplace.ts`

**Kind:** service

**Owns:**
- 搜索/替换算法和状态管理
- 搜索高亮与编辑器交互

**Does not own：**
- 不拥有搜索面板 UI（在 SearchReplacePanel.svelte 中）
- 不拥有编辑器内高亮插件（在 searchHighlight.ts 中）

**Called by:** `src/app/components/SearchReplacePanel.svelte`, `src/app/App.svelte`

**Depends on:** `src/lib/editor-core/types.ts`

**Change this when：**
- 修改搜索算法
- 修改替换逻辑
- 修改搜索状态管理

**Do not change this when：**
- 修改搜索面板 UI

**Related tests:** `src/app/services/searchReplace.test.ts`

**Confidence:** high

---

### `src/app/services/exportService.ts`

**Kind:** service

**Owns:**
- HTML/PDF 导出：构建自包含 HTML 文档
- 调度 `export/` 中的快照正文渲染和资源内嵌；HTML/PDF 接收同一份文档与外观值快照
- 编辑器 UI 痕迹清理（cleanEditorArtifacts）
- 完整 HTML 文档外壳生成（createExportHtmlDocument）
- 文件保存对话框调用（pickSavePath）

**Does not own：**
- 不拥有文件系统写入（通过 tauriStorage.ts 的 invoke 调用）
- 不拥有 PDF 渲染（通过 tauriStorage.ts 的 exportPdfFromHtml）
- 不拥有导出 UI 触发（在 AppShell.svelte / AppTitleBar.svelte 中）

**Called by:** `src/app/App.svelte`（通过菜单/快捷键触发导出）

**Depends on:** `src/app/services/export/`, `src/lib/desktop/tauriStorage.ts`, `src/app/styles/export-document.css`

**Change this when：**
- 修改图片内嵌策略（新增/移除图片来源类型支持）
- 修改 HTML 模板结构或样式
- 修改编辑器痕迹清理规则

**Do not change this when：**
- 修改导出触发 UI
- 修改后端文件写入逻辑

**Related tests:** `src/app/services/exportService.test.ts`, `src/app/services/export/snapshot.test.ts`, `src/app/services/export/render.test.ts`, `src/app/services/export/images.test.ts`, `src/lib/typography/exportPrint.test.ts`

**Confidence:** high

---

### `src/app/services/wordExportService.ts` / `src/app/services/word/`

**Kind:** export service / task-scoped worker

**Owns:** Word 保存对话框、导出结果与警告；`snapshot.ts` 固定文档并等待输入提交，`model.ts` 复用 Markdown 解析及行内投影，`resources.ts` 处理图片与公式/图表 PNG，`document.ts` 生成 OOXML，`wordExport.worker.ts` 在独立 Worker 中打包。任务内媒体最多 128 MiB，Canvas 最多 1600 万像素，结束清理 Worker 与临时 DOM。

**Does not own:** 编辑器视图、撤销历史、HTML/PDF 样式或原生文件写入。

**Called by:** `src/app/App.svelte` 的 `docx` 导出分支；菜单命令为 `export-word`。

**Related tests:** `word/document.test.ts`, `word/resources.test.ts`, `word/snapshot.test.ts`, `wordExportService.test.ts`；原生写入覆盖 `export.rs` 的 `word_export_tests`。

**Confidence:** high

---

### `src/app/services/appCommands.ts`

**Kind:** service

**Owns:**
- 应用命令与快捷键分发：连接菜单命令、全局快捷键和编辑器命令
- 处理 `new-file`、`save-file`、`menu-link`、`insert-diagram:*` 等命令

**Does not own：**
- 不拥有编辑器命令具体实现（在 editorCommands.ts 中）
- 不拥有菜单 UI 构建（在 Rust 后端 menu.rs 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/editor-core/types.ts`, `src/app/services/settings.ts`, `src/app/services/outlinePanelPosition.ts`

**Change this when：**
- 添加新的应用级命令
- 修改快捷键映射
- 修改命令分发逻辑

**Do not change this when：**
- 修改编辑器内部命令实现

**Related tests:** `src/app/services/appCommands.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/index.ts`

**Kind:** entry

**Owns:**
- 编辑器核心导出入口
- 集中导出 `EditorCore`、类型、命令和创建函数

**Does not own：**
- 不拥有具体实现（委派给内部模块）

**Called by:** 应用层所有使用编辑器的地方

**Depends on:** `src/lib/editor-core/createEditorCore.ts`, `src/lib/editor-core/types.ts`

**Change this when：**
- 新增/移除对外暴露的编辑器 API
- 调整导出结构

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/createEditorCore.ts`

**Kind:** factory

**Owns：**
- `ProseMirrorEditorCore` 实例创建
- 对外提供稳定创建入口

**Does not own：**
- 不拥有编辑器运行时逻辑（在 ProseMirrorEditorCore.ts 中）

**Called by:** `src/app/App.svelte`, 应用层

**Depends on:** `src/lib/editor-core/ProseMirrorEditorCore.ts`

**Change this when：**
- 修改编辑器创建参数
- 修改初始化配置

**Do not change this when：**
- 修改编辑器运行时行为

**Related tests:** `src/lib/editor-core/createEditorCore.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/ProseMirrorEditorCore.ts`

**Kind:** service

**Owns：**
- `EditorView` 生命周期管理
- 编辑器状态创建与事务派发
- Markdown 同步（编辑后序列化为 Markdown 通知应用层）
- 双栏锚点只读快照；用结构差异确认的前后缀换算复杂块造成的位置偏移，不沿用不可靠区间
- 语义选区变化与选中 Markdown 片段通知
- 按视口坐标折叠选区，先提交节点输入再定位；使用实际内容边界区分文末和 min-height 留白，不增加撤销步骤
- 文档脏状态管理
- 模式切换（语义/源码）
- 命令执行、只读事务门禁、撤销前活动编辑提交与节点组字等待
- 插件和 NodeView 注册
- 提供独立内容/渲染修订的同步快照、嵌套锚点几何与光标坐标；不改选区或历史
- 保留旧块对齐 decoration 兼容接口，双栏运行路径不再调用
- 生成剪贴板文本/HTML、协调 Markdown/HTML/纯文本粘贴事务，并对右键命中的块级对象执行定位、编辑或删除事务

**Does not own：**
- 不拥有 Markdown 解析/序列化具体规则（在 markdown.ts 中）
- 不拥有 Schema 定义（在 schema.ts 中）
- 不拥有渲染服务实现（通过 renderers.ts 注册）

**Called by:** `src/lib/editor-core/createEditorCore.ts`

**Depends on:** `src/lib/editor-core/schema.ts`, `src/lib/editor-core/markdown.ts`, `src/lib/editor-core/clipboardMarkdown.ts`, `src/lib/editor-core/editorCommands.ts`, `src/lib/editor-core/renderers.ts`, `src/lib/editor-core/plugins/*`, `src/lib/editor-core/nodeViews/*`

**Change this when：**
- 修改编辑器生命周期管理
- 修改事务处理逻辑
- 修改模式切换流程
- 修改插件/NodeView 注册方式
- 修改剪贴板或右键目标的最小公共 API

**Do not change this when：**
- 修改具体 Markdown 语法规则
- 修改具体渲染服务实现

**Related tests:** `src/lib/editor-core/*.test.ts`（含 `blockAlignment.test.ts`）

**Confidence:** high

---

### `src/lib/editor-core/clipboardMarkdown.ts`

**Kind:** service

**Owns：**
- 通过 Markdown 树与纯文本树的语义等价比较识别剪贴板内容
- 构造结构化 Markdown Slice 和保留换行/活动 marks 的纯文本 Slice
- 定义纯文本粘贴事务标记，供语法输入插件跳过自动转换

**Does not own：**
- 不派发 EditorView 事务（在 `ProseMirrorEditorCore.ts` 中）
- 不定义 Markdown 语法（在 `markdown.ts` 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`, Markdown 输入插件

**Depends on:** `src/lib/editor-core/markdown.ts`, `src/lib/editor-core/schema.ts`

**Change this when：**
- 修改自动识别 Markdown、纯文本回退或粘贴 Slice 构造规则

**Related tests:** `src/lib/editor-core/clipboardMarkdown.test.ts`, `src/lib/editor-core/createEditorCore.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/markdown.ts`

**Kind:** service

**Owns：**
- 基于 `markdown-it` 和 `prosemirror-markdown` 的 Markdown 解析
- ProseMirror doc 序列化回 Markdown
- 表格文本的反斜杠保留与行内代码围栏，避免序列化丢字导致后续同步锚点偏移
- 合并格、无表头等保真表格 HTML 的完整校验与 schema 表格恢复；未知或不合法表格 HTML 保留原文
- 顶层 Front Matter 文档属性与正文的解析、序列化
- 表格、图片扩展属性、公式、脚注、HTML、Callout、TOC、Mermaid、注释等语法处理

**Does not own：**
- 不拥有基础 markdown-it 规则（依赖库提供）
- 不拥有 Callout 具体解析规则（在 callout/calloutParser.ts 中）
- 不拥有 HTML 转换逻辑（在 html/ 目录中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`

**Depends on:** `src/lib/editor-core/schema.ts`, `src/lib/editor-core/callout/calloutParser.ts`, `src/lib/editor-core/callout/calloutSerializer.ts`, `src/lib/editor-core/html/htmlToPmLogic.ts`, `src/lib/editor-core/html/pmToHtml.ts`

**Change this when：**
- 修改 Markdown 解析规则
- 修改 Markdown 序列化输出格式
- 新增 Markdown 语法支持

**Do not change this when：**
- 修改编辑器 UI 行为
- 修改渲染样式

**Related tests:** `src/lib/editor-core/markdown.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/schema.ts`

**Kind:** model

**Owns：**
- ProseMirror Schema 定义
- 扩展 Markdown 基础 schema：图片属性、表格、公式、HTML、注释、脚注、TOC、Mermaid、Callout 等节点/mark

**Does not own：**
- 不拥有解析/序列化规则（在 markdown.ts 中）
- 不拥有 Callout schema 细节（在 callout/calloutSchema.ts 中）

**Called by:** `src/lib/editor-core/markdown.ts`, `src/lib/editor-core/ProseMirrorEditorCore.ts`, `src/lib/editor-core/editorCommands.ts`, nodeViews, plugins

**Depends on:** `src/lib/editor-core/callout/calloutSchema.ts`

**Change this when：**
- 新增/修改节点或 mark 类型
- 修改文档结构约束

**Do not change this when：**
- 修改解析规则
- 修改 UI 展示

**Related tests:** `src/lib/editor-core/*.test.ts`（涉及 schema 的测试）

**Confidence:** high

---

### `src/lib/editor-core/quickInsert.ts`

**Kind:** module

**Owns：**
- 快捷插入类别、默认样式及对应命令，无搜索索引
- 图片/链接/Emoji 选择动作、LaTeX 公式块、图表模板和行内/块注释类别
- 默认触发符号与单个标点/符号校验

**Called by:** `plugins/quickInsert.ts`, `src/app/services/settings.ts`, `src/app/components/SettingsWindow.svelte`

**Depends on:** `types.ts`, `diagramTemplates.ts`, `src/app/i18n.ts`

**Change this when：** 调整快捷插入类别、样式、默认符号或符号校验。

**Confidence:** high

---

### `src/lib/editor-core/plugins/quickInsert.ts`

**Kind:** plugin

**Owns：**
- 语义编辑的输入触发、菜单会话及类别/样式键盘导航
- 菜单 DOM、光标附近定位、界面语言更新及挂起/销毁清理
- 提交输入法字符后的触发及行内代码范围排除
- 滚动列表中保持键盘选中行可见，向应用层传递图片/链接插入选区
- 向应用层传递 Emoji 选择器的替换选区；ARIA 属性只清理本菜单持有的值

**Does not own：** 内容插入/块转换事务在 `editorCommands.ts`，图片/链接选择窗口、偏好保存和设置 UI 在应用层。

**Called by:** `ProseMirrorEditorCore.ts`

**Depends on:** `quickInsert.ts`, `editorCommands.ts`, `InlineSourceCodec.ts`, `plugins/inlineSourceEditing.ts`, `src/app/i18n.ts`, `src/app/styles/editor-quick-insert.css`

**Change this when：** 调整触发边界、菜单交互、浮层定位或生命周期。

**Confidence:** high

---

### `src/lib/editor-core/editorCommands.ts`

**Kind:** service

**Owns：**
- 编辑器命令实现：标题、粗体、斜体、链接、列表、引用、代码块、表格、公式、图表、TOC 等
- 将应用层 `EditorCommand` 转换为 ProseMirror transaction
- 以单个事务移动章节顶层节点切片并调整后代标题层级
- 快捷插入以单个事务删除触发符号并插入或转换内容，复用表格生成、图表模板、列表/引用包装规则和公式/注释编辑入口
- 行内公式命令委托 `plugins/mathInlineDraft.ts`；快捷插入的行内公式仅在确认草稿时替换触发符，取消保留原文

**Does not own：**
- 不拥有表格命令细节（在 tableCommands.ts 中）
- 不拥有代码块命令细节（在 codeBlockCommands.ts 中）
- 不拥有 Callout 命令细节（在 callout/calloutCommands.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`, `src/app/services/appCommands.ts`

**Depends on:** `src/lib/editor-core/schema.ts`, `src/lib/editor-core/tableCommands.ts`, `src/lib/editor-core/codeBlockCommands.ts`, `src/lib/editor-core/callout/calloutCommands.ts`, `src/lib/outline/outlineReorder.ts`

**Change this when：**
- 新增编辑命令
- 修改现有命令行为
- 修改命令参数

**Do not change this when：**
- 修改编辑器事务底层逻辑
- 修改 UI 触发方式

**Related tests:** `src/lib/editor-core/editorCommands.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/renderers.ts`

**Kind:** registry

**Owns：**
- 渲染器注册表：保存代码高亮、图表、公式、图片加载器等全局适配器
- 让 NodeView 不直接依赖具体第三方库实例

**Does not own：**
- 不拥有具体渲染服务实现（在 src/lib/services/ 中）

**Called by:** `src/app/App.svelte`（初始化注册）, `src/lib/editor-core/nodeViews/*`（读取使用）

**Depends on:** `src/lib/services/render.ts`

**Change this when：**
- 新增渲染器类型
- 修改渲染器注册方式

**Do not change this when：**
- 修改具体渲染算法

**Related tests:** —

**Confidence:** high

---

### `src/lib/services/shikiCodeTokenizer.ts`

**Kind:** service

**Owns：**
- 调用 Shiki 对代码进行 token 化；结果缓存同时限制条数和估算字节，超预算单条结果不保留
- 为代码块 NodeView 和装饰插件提供高亮数据

**Does not own：**
- 不拥有代码块展示逻辑（在 CodeBlockNodeView.ts 中）

**Called by:** `src/app/App.svelte`（注册到 renderers.ts）

**Depends on:** `shiki`

**Change this when：**
- 修改代码高亮算法
- 修改 token 缓存策略

**Do not change this when：**
- 修改代码块 UI

**Related tests:** `src/lib/services/shikiCodeTokenizer.test.ts`

**Confidence:** high

---

### `src/lib/services/katexMathRenderer.ts`

**Kind:** service

**Owns：**
- 调用 KaTeX 渲染行内/块级公式
- 为公式 NodeView 提供 HTML 结果

**Does not own：**
- 不拥有公式编辑交互（在 MathBlockNodeView.ts / MathInlineNodeView.ts 中）

**Called by:** `src/app/App.svelte`（注册到 renderers.ts）

**Depends on:** `katex`

**Change this when：**
- 修改公式渲染配置
- 修改错误处理方式

**Do not change this when：**
- 修改公式输入规则

**Related tests:** `src/lib/services/katexMathRenderer.test.ts`

**Confidence:** high

---

### `src/lib/services/mermaidDiagramRenderer.ts`

**Kind:** service

**Owns：**
- 调用 Mermaid 渲染图表
- 为 Mermaid NodeView 提供 SVG
- 按当前主题传入 Mermaid `theme` 与 `themeVariables`；初始化和渲染串行调度，允许取消过期的排队任务
- 首次加载 Mermaid 时注册本地 `logos` 图标包，按需加载品牌图标供主编辑器与 Quick Look 离线复用

**Does not own：**
- 不拥有图表代码编辑交互（在 MermaidBlockNodeView.ts 中）

**Called by:** `src/app/App.svelte`（注册到 renderers.ts）

**Depends on:** `mermaid`, `@iconify-json/logos`

**Change this when：**
- 修改 Mermaid 配置
- 修改图表渲染错误处理

**Do not change this when：**
- 修改图表插入流程

**Related tests:** `src/lib/editor-core/diagramBlock.test.ts`

**Confidence:** high

---

### `src/lib/outline/outlineService.ts`

**Kind:** service

**Owns：**
- 复用 `tocService.ts` 的真实 Markdown 标题集合、来源行号与统一 ID，提供大纲兼容入口
- 同步统计兼容入口；词数、字数与范围查询由 `writingStats.ts` 负责

**Does not own：**
- 不拥有大纲 UI 展示（在 EditorWorkspace.svelte 中）
- 不拥有大纲交互（在 outlineInteractionController.ts 中）

**Called by:** `src/app/App.svelte`, `src/app/services/documentActionsController.ts`

**Depends on:** `src/lib/outline/writingStats.ts`, `src/lib/toc/tocService.ts`

**Change this when：**
- 修改大纲提取算法
- 修改同步统计兼容入口（计数规则在 writingStats.ts）

**Do not change this when：**
- 修改大纲面板 UI

**Related tests:** `src/lib/outline/outlineService.test.ts`

**Confidence:** high

---

### `src/lib/outline/outlineReorder.ts`

**Kind:** utility

**Owns：**
- 按标题索引计算章节子树范围、前/内/后落点和统一层级增量
- 保真重排 Markdown 行记录并返回原标题索引到新索引的映射
- 拒绝自身/后代、H6 越界、无效索引和无变化落点

**Does not own：**
- 不拥有 Pointer 手势与视觉反馈（在 EditorWorkspace.svelte 中）
- 不拥有 ProseMirror 事务执行（在 editorCommands.ts 中）

**Called by:** `src/app/services/outlineInteractionController.ts`, `src/app/components/EditorWorkspace.svelte`, `src/lib/editor-core/editorCommands.ts`

**Depends on:** `src/lib/outline/outlineService.ts`

**Change this when：**
- 修改章节边界、树形落点、标题层级或源码重排规则

**Related tests:** `src/lib/outline/outlineReorder.test.ts`

**Confidence:** high

---

### `src/lib/toc/tocService.ts`

**Kind:** service

**Owns：**
- 生成 TOC Markdown 块
- 基于真实 Markdown 纯预处理与 token 生成目录项及来源行号，统一大纲与文档标题 ID
- 提供标题归一化与全局唯一 ID 分配，DOM 标题属性和目录导航复用文档位置映射

**Does not own：**
- 不拥有 TOC 块展示（在 TocBlockNodeView.ts 中）

**Called by:** `src/app/services/editorInteractionController.ts`, `src/lib/editor-core/nodeViews/TocBlockNodeView.ts`, `src/lib/editor-core/plugins/tocSync.ts`, `src/lib/outline/outlineService.ts`, `src/app/services/outlineNavigation.ts`

**Depends on:** `src/lib/editor-core/markdownTokenizer.ts`, `src/lib/editor-core/markdownSerialization.ts`, `src/lib/editor-core/InlineSourceCodec.ts`, `src/lib/editor-core/markdown.ts`（纯 tokenizer 与来源行号 API）

**Change this when：**
- 修改 TOC 生成逻辑
- 修改目录项数据结构

**Do not change this when：**
- 修改 TOC 块渲染样式

**Related tests:** `src/lib/toc/tocService.test.ts`

**Confidence:** high

---

### `src/lib/markdown/frontMatter.ts`

**Kind:** utility

**Owns：**
- 识别 Markdown 开头的 YAML front matter
- 拆出元数据块和正文

**Does not own：**
- 不拥有 front matter UI（在 FrontMatterCard.svelte 中）

**Called by:** `src/lib/markdown/MarkdownBridge.ts`, 编辑器/文档流程

**Depends on:** —

**Change this when：**
- 修改 front matter 解析规则
- 修改 YAML 边界检测

**Do not change this when：**
- 修改 front matter 展示 UI

**Related tests:** `src/lib/markdown/frontMatter.test.ts`

**Confidence:** high

---

### `src/lib/markdown/normalize.ts`

**Kind:** utility

**Owns：**
- 保存前规范化 Markdown 文本
- 末尾换行等格式统一

**Does not own：**
- 不拥有保存流程（在 documentActionsController.ts 中）

**Called by:** `src/app/services/documentActionsController.ts`

**Depends on:** —

**Change this when：**
- 修改保存规范化规则

**Related tests:** `src/lib/markdown/normalize.test.ts`

**Confidence:** high

---

### `src/lib/desktop/tauriStorage.ts`

**Kind:** service

**Owns：**
- 封装配置存储、文件系统、图片资源、窗口设置等 Tauri `invoke` 调用
- 封装工作区草稿、快照正文文件相关 IPC 调用
- 为浏览器环境提供 fallback

**Does not own：**
- 不拥有后端具体实现（在 Rust 中）

**Called by:** `src/app/services/settings.ts`, `src/app/services/documentFiles.ts`, `src/app/services/folderExplorerController.ts`, `src/app/services/workspacePersistence.ts`, `src/app/App.svelte`

**Depends on:** `@tauri-apps/api`

**Change this when：**
- 新增/修改前端 IPC 调用封装
- 修改 fallback 逻辑

**Do not change this when：**
- 修改后端 IPC 命令实现

**Related tests:** `src/lib/desktop/tauriStorage.test.ts`

**Confidence:** high

---

### `src/quicklook/preview.ts`

**Kind:** service

**Owns：**
- Quick Look Markdown 渲染
- 支持 Callout、公式、图片属性、任务列表、Mermaid 占位和链接安全处理
- 生成经过 sanitizer 过滤的预览 HTML
- 导出可供大文档小窗复用的安全正文 HTML

**Does not own：**
- 不拥有主编辑器 Markdown 解析（在 markdown.ts 中）

**Called by:** `src/quicklook/preview-entry.ts`, `src/app/components/MarkdownMiniLargePreview.svelte`

**Depends on:** `katex`, `markdown-it`, `src/lib/editor-core/callout/calloutParser.ts`, `src/lib/editor-core/link.ts`

**Change this when：**
- 修改 Quick Look 预览渲染规则
- 新增 Quick Look 支持的 Markdown 扩展

**Do not change this when：**
- 修改主编辑器行为

**Related tests:** `src/quicklook/preview.test.ts`

**Confidence:** high

---

### `src-tauri/src/lib.rs`

**Kind:** entry

**Owns：**
- Tauri 后端装配：初始化日志、插件、配置管理器、窗口、菜单、托盘
- 渲染模式检测（启动前从 config.json 读取 software render 设置）
- 外部打开路由
- 关闭拦截（`WindowEvent::CloseRequested`）
- IPC command 注册

**Does not own：**
- 不拥有具体业务模块实现（委派给 config/、file_system/、window/ 等子模块）

**Called by:** `src-tauri/src/main.rs`

**Depends on:** `src-tauri/src/config/`, `src-tauri/src/file_system/`, `src-tauri/src/window/`, `src-tauri/src/software_update.rs`, `src-tauri/src/external_link.rs`, `src-tauri/src/app_logger.rs`, `src-tauri/src/i18n.rs`, `src-tauri/src/export.rs`

**Change this when：**
- 新增 IPC 命令
- 新增插件
- 修改窗口事件处理（如 `CloseRequested` 关闭确认逻辑）
- 修改启动流程

**Do not change this when：**
- 修改具体业务逻辑（在子模块中）

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/models.rs`

**Kind:** model

**Owns：**
- 跨端序列化数据结构：DocumentPayload、RecentEntry、SnapshotRecord、SettingRecord、WindowStateInput、FileTreeEntry、ImageAssetPayload、DesktopActionPayload 等
- Markdown 文件编码枚举及其 IPC 序列化值
- 窗口事件通信 payload（如 `WindowLabelPayload`）

**Does not own：**
- 不拥有具体业务逻辑

**Called by:** 前后端各模块

**Depends on:** `serde`

**Change this when：**
- 新增/修改 IPC 参数或返回值结构
- 新增前后端通信数据结构

**Do not change this when：**
- 修改业务逻辑

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/config/mod.rs`

**Kind:** service / data store

**Owns：**
- 应用配置 JSON 持久化：`config.json` 的读取、写入、备份
- `ConfigManager`：线程安全的配置管理器（`Arc<RwLock<AppConfig>>`）
- `AppConfig` 结构定义：app（设置）、editor（编辑器设置）、window（窗口状态）、recent（最近打开）、workspace（工作区标签元数据）、snapshots（文档快照索引）
- 草稿/快照正文仓库路径规则，以及旧 snapshot 正文迁移到独立内容文件
- 设置键值存储：为前端偏好设置提供后端读写
- 设置路由：按 key 前缀将设置分发到对应 section（`windowState:`、`workspaceTabs:`、`pendingFolder:` 等）
- 启动前设置读取：在 `AppHandle` 可用前从磁盘读取配置（渲染模式、开发者模式）

**Does not own：**
- 不拥有具体 IPC 命令实现（在 `config/commands.rs` 中）
- 不拥有前端设置模型与归一化（在 `settings.ts` 中）

**Called by:** `src-tauri/src/lib.rs`（setup 中初始化）, `src-tauri/src/config/commands.rs`, 启动流程

**Depends on:** `src-tauri/src/models.rs`, `src-tauri/src/app_logger.rs`, Tauri path API

**Change this when：**
- 新增/修改 AppConfig section 或字段
- 新增启动前需要读取的设置项
- 修改配置备份/恢复逻辑
- 修改草稿/快照正文文件目录或旧配置迁移规则
- 修改设置路由规则

**Do not change this when：**
- 修改前端设置 UI
- 修改具体业务逻辑

**Related tests:** `src-tauri/src/config/mod.rs` 模块内测试

**Confidence:** high

---

### `src-tauri/src/file_system.rs`

**Kind:** service

**Owns：**
- 后端文件系统：读写 Markdown、创建/重命名/删除文件夹和文件
- Markdown 的 UTF-8/BOM、UTF-16 BOM、GBK 解码与按源编码安全写回
- 使用短生命周期目录快照按需枚举根目录与已加载子目录
- 路径存在性检查
- 示例文档安装
- Markdown-like 文件过滤、忽略规则、`.gitignore`、目录优先排序

**Does not own：**
- 不拥有图片资源处理（在 file_system/image_assets.rs 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）, 前端 `documentFiles.ts`

**Depends on:** `src-tauri/src/models.rs`, `encoding_rs`

**Change this when：**
- 修改文件读写逻辑
- 修改目录扫描规则
- 修改文件过滤/忽略规则

**Do not change this when：**
- 修改前端文件操作封装

**Related tests:** `src-tauri/src/file_system.rs` 内联测试

**Confidence:** high

---

### `src-tauri/src/file_system/image_assets.rs`

**Kind:** service

**Owns：**
- 导入/解析/删除本地图片
- PicGo-Core 上传
- PicGo Server 上传和连接测试
- 路径策略、文件名清洗、SHA-256 去重、临时文件处理

**Does not own：**
- 不拥有前端图片插入流程（在 imageInsertion.ts 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）

**Depends on:** `src-tauri/src/models.rs`

**Change this when：**
- 修改图片导入策略
- 修改 PicGo 上传逻辑
- 修改路径策略

**Do not change this when：**
- 修改前端图片展示 UI

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/software_update.rs`

**Kind:** service

**Owns：**
- 检查 GitHub Release
- 区分 Windows 安装版、免安装版和不支持环境，选择对应资产
- 维护进程级更新快照并通过 `nomo://software-update-state` 同步所有窗口
- 下载、校验 MD5、启动安装器
- Windows 安装版支持应用内更新，免安装版只提供 zip 直链
- 在 setup 首次写入运行数据前识别既有用户，使用既有 settings 保存最高运行版本与待展示版本，兼容旧版本配置读写
- 提供当前运行版本的离线日志领取/挂载确认 IPC；进程级领取权防止多窗口重复展示，窗口销毁或前端卸载时释放

**Does not own：**
- 不拥有前端更新 UI（在 SoftwareUpdateNotice / SoftwareUpdateDialog / SoftwareUpdateSuccessDialog 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）

**Depends on:** `reqwest`, `semver`, `sha2`, `md5`

**Change this when：**
- 修改更新检查逻辑
- 修改下载/安装流程
- 修改校验逻辑

**Do not change this when：**
- 修改前端更新界面

**Related tests:** `src/lib/desktop/tauriUpdater.test.ts`

**Confidence:** high

---

### `src-tauri/src/window/menu.rs`

**Kind:** service

**Owns：**
- 构建应用原生菜单
- 绑定快捷键
- 处理菜单事件（普通命令 emit 为 `nomo://menu-command`）

**Does not own：**
- 不拥有菜单命令前端处理（在 appCommands.ts 中）
- 不拥有后端特殊处理（quit、open-settings 在 lib.rs / commands.rs 中）

**Called by:** `src-tauri/src/lib.rs`, `src-tauri/src/window/commands.rs`

**Depends on:** `src-tauri/src/i18n.rs`, `src-tauri/src/config/commands.rs`, Tauri menu API

**Change this when：**
- 添加/修改菜单项
- 修改快捷键绑定
- 修改菜单事件处理

**Do not change this when：**
- 修改前端命令处理逻辑

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/window/tray.rs`

**Kind:** service

**Owns：**
- 安装系统托盘
- 刷新托盘菜单
- 切换图标
- 处理托盘点击
- 关闭到托盘行为

**Does not own：**
- 不拥有窗口关闭逻辑（在 lib.rs 的 WindowEvent::CloseRequested 中）

**Called by:** `src-tauri/src/lib.rs`, `src-tauri/src/window/commands.rs`

**Depends on:** `src-tauri/src/config/commands.rs`, `src-tauri/src/i18n.rs`

**Change this when：**
- 修改托盘菜单项
- 修改托盘图标切换逻辑
- 修改关闭到托盘行为

**Do not change this when：**
- 修改窗口关闭确认流程

**Related tests:** —

**Confidence:** high

---

### `src/app/services/openTargetRouting.ts`

**Kind:** service

**Owns:** 文件与文件夹打开策略的调用顺序；确定使用当前窗口或显示询问框后才激活接收窗口；当前窗口已打开的目录仅激活，不重新恢复工作区；显式选择目录窗口后继续处理混合批次中的剩余目标。

**Does not own:** 不改变路径归属、初始空窗口判定、启动队列或原生窗口创建流程。

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/desktopWindow.ts`, `src/app/services/settings.ts`

**Change this when:** 修改打开方式的分支或窗口激活时机。

**Related tests:** `src/app/services/openTargetRouting.test.ts`

**Confidence:** high

---

### `src-tauri/src/window/open_targets.rs`

**Kind:** service / registry

**Owns:**
- 维护进程内文档窗口当前文件夹、全部已打开文件及在建窗口目标预留
- 文件只自动匹配实际已打开路径；已登记的直接父目录窗口作为询问时的显式候选，不覆盖打开偏好
- 规范化绝对路径，原子判断目标归属并聚焦已有窗口或生成唯一新窗口标签
- 提供同步目标、准备打开窗口、显式目录窗口打开和创建失败释放预留的 IPC
- 提供拖放路径识别 IPC：读取元数据、跳过不存在或不支持的路径，首个有效目标为文件夹时返回单目录，为文档时收集全部支持的文档并去重；读取异常明确上报

**Does not own:**
- 不拥有具体文件、文件夹加载与标签定位逻辑（在 `src/app/App.svelte` 中）
- 不拥有启动参数和单实例事件解析（在 `external_open.rs` 中）

**Called by:** `src/app/services/desktopWindow.ts`, `src-tauri/src/lib.rs`, `src-tauri/src/window/commands.rs`

**Depends on:** `src-tauri/src/window/external_open.rs`, `src-tauri/src/config/commands.rs`, Tauri 窗口与事件系统

**Change this when:**
- 修改跨窗口目标匹配、预留超时、已有窗口激活或新窗口标签规则

**Related tests:** 同文件 Rust 测试（文件优先、直接父目录匹配、批次目标划分和拖放路径识别）

**Confidence:** high

---

### `src-tauri/src/window/external_open.rs`

**Kind:** service

**Owns：**
- 解析启动参数、单实例参数
- macOS open 事件中的文件/文件夹解析
- 把待打开路径写入 pending 设置并 emit `nomo://open-document` / `nomo://open-folder`
- 接收文件请求时不显示或激活窗口，由前端确定最终去向后执行激活

**Does not own：**
- 不拥有前端打开处理（在 App.svelte 中）

**Called by:** `src-tauri/src/lib.rs`（单实例插件、setup、RunEvent）

**Depends on:** `src-tauri/src/config/commands.rs`, Tauri 事件系统

**Change this when：**
- 修改启动参数解析逻辑
- 修改外部打开事件路由

**Do not change this when：**
- 修改前端文件打开流程

**Related tests:** —

**Confidence:** high

---

### `src/app/services/tabs.ts`

**Kind:** utility

**Owns:**
- 标签页纯函数操作：创建空白标签、判断可复用标签、获取目标标签
- `ActiveTabState` 定义与写入
- 标签 ID 生成

**Does not own:**
- 不拥有标签页 UI 展示（在 DocumentTabs.svelte 中）
- 不拥有标签页业务协调（在 documentActionsController.ts 中）

**Called by:** `src/app/services/documentActionsController.ts`, `src/app/App.svelte`

**Depends on:** `src/app/types.ts`, `src/app/i18n.ts`

**Change this when:**
- 修改标签页创建/复用策略
- 修改标签状态数据结构

**Do not change this when:**
- 修改标签页 UI 样式

**Related tests:** —

**Confidence:** high

---

### `src/app/services/workspacePersistence.ts`

**Kind:** service

**Owns:**
- `PersistedWorkspaceState v4` 的生成、组合元数据归一化及 v3/v2/旧 `workspaceTabs:*` 配置迁移
- 工作区标签元数据持久化边界：禁止把 `markdown` / `savedMarkdown` 写回配置
- 根据 dirty、未命名标签和 `draftId` 写入或清理工作区草稿正文文件
- 将持久化标签元数据还原为运行时标签所需的基础结构；活动组合两篇立即加载，其他项延迟恢复

**Does not own:**
- 不拥有启动时磁盘文件读取与冲突 UI 决策（在 `App.svelte` 中协调）
- 不拥有草稿文件后端路径安全与实际读写（通过 `tauriStorage.ts` 调用 Rust IPC）
- 不拥有异常退出恢复草稿 localStorage 逻辑（在 `recoveryDraft.ts` 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/types.ts`, `src/lib/desktop/tauriStorage.ts`

**Change this when:**
- 修改工作区配置版本、标签元数据字段或迁移策略
- 修改哪些标签需要生成草稿正文文件
- 修改配置正文剥离规则

**Related tests:** `src/app/services/workspacePersistence.test.ts`

**Confidence:** high

---

### `src/app/services/workspaceItems.ts`

**Kind:** model / utility

**Owns:** single / comparison 外层项创建、文档唯一归属、两篇不同 Markdown 约束、无损降级、移除成员、取消组合与交换左右。

**Does not own:** 编辑器运行时、DOM、跨窗口协议或磁盘持久化。

**Called by:** `src/app/App.svelte`, `workspacePersistence.ts`, `workspaceTransferController.ts`

**Depends on:** `src/app/types.ts`

**Change this when:** 修改组合成员约束、外层项 ID 或归属归一化策略。

**Related tests:** 现有 `workspacePersistence.test.ts` 间接覆盖持久化入口。

**Confidence:** high

---

### `src/app/services/markdownDocumentRuntime.ts`

**Kind:** runtime contract

**Owns:** 按文档 ID 保存 EditorCore、脱离 UI 的 CodeMirror 状态、DOM 挂载引用、交互 / 图片处理器、大纲、光标和阅读位置的运行时契约；语义工作区可先就绪，源码句柄与滚动容器在首次使用后补齐。

**Does not own:** 实际创建、挂起、销毁或归属变更（由 `App.svelte` 协调）；不写入持久化配置。

**Called by:** `src/app/App.svelte`, `ComparisonPane.svelte`, `ComparisonWorkspace.svelte`, `AppShell.svelte`

**Depends on:** `EditorCore`, `markdownSourceEditor.ts`, `editorInteractionController.ts`, `imageInsertion.ts`, `outlineInteractionController.ts`

**Change this when:** 修改窗口内编辑器状态保留或成员视图重挂载需要的字段。

**Related tests:** —

**Confidence:** high

---

### `src/app/components/ComparisonWorkspace.svelte` / `ComparisonPane.svelte`

**Kind:** component

**Owns:** 两文档分栏、共享模式和大文件源码降级、比例拖动 / 键盘调整、成员标题拖动、聚焦回调、独立视图挂载、源码状态及阅读位置恢复。

**Does not own:** 成组 / 拆组数据变更、保存 / 关闭确认、跨窗口提交或文件归属。

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `EditorWorkspace.svelte`, `markdownDocumentRuntime.ts`, `outlineInteractionController.ts`, `outlineNavigation.ts`

**Change this when:** 修改组合分栏交互、成员挂载清理、焦点或阅读位置恢复。

**Related tests:** —

**Confidence:** high

---

### `src/app/services/workspaceTransferController.ts`

**Kind:** controller

**Owns:** 单篇、整组和组合成员移交的前端暂存、接收完整性、原子可见状态应用、超时协调、幂等确认和提交后查询 / 重放。

**Does not own:** Win32 屏幕命中、磁盘事务或编辑器实例生命周期；通过 App 回调刷新临时编辑并应用工作区。

**Called by:** `src/app/App.svelte`

**Depends on:** `workspaceItems.ts`, `workspacePersistence.ts`, `tabTransfer.ts`

**Change this when:** 修改跨窗口协议协调、前端事务冻结、接收落点或失败恢复。

**Related tests:** —

**Confidence:** high

---

### `src/app/services/tabTransfer.ts`

**Kind:** IPC service / protocol

**Owns:** 移交与落点类型、原生拖动和移交 IPC / 事件适配、当前窗口 revision / ownership epoch 缓存及提交后的 epoch 更新。

**Does not own:** 前端工作区暂存或后端事务实现。

**Called by:** `src/app/App.svelte`, `workspaceTransferController.ts`, `desktopWindow.ts`, `DocumentTabs.svelte`

**Depends on:** `@tauri-apps/api/core`, `@tauri-apps/api/event`, `src/app/types.ts`

**Change this when:** 修改移交命令、事件契约或旧异步快照失效规则。

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/window/tab_transfer.rs`

**Kind:** transaction / command service

**Owns:** prepare / target-ready / commit / cancel 状态机、两窗工作区与文件归属原子提交、revision / epoch 校验、持久化移交记录、异常退出恢复、新目标窗口登记和空源窗口内部关闭。

**Does not own:** Win32 拖动命中、文档编辑器历史或普通窗口关闭 UI。

**Called by:** `src-tauri/src/lib.rs` 命令注册、`config/commands.rs` 写入门禁、`window/open_targets.rs`、`window/commands.rs`

**Depends on:** `config`, `window/open_targets.rs`, `window/tab_drag.rs`, `window/workspace_lifecycle.rs`, `window/state.rs`

**Change this when:** 修改事务完整性、归属校验、磁盘记录恢复或迁移空窗口关闭规则。

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/window/tab_drag.rs`

**Kind:** Windows input / command service

**Owns:** 文档窗口落点登记、Win32 鼠标与真实顶层窗口命中、物理屏幕坐标 / DPI 换算、原生拖动 move / drop / cancel 事件；排除未就绪窗口和 Markdown 小窗。

**Does not own:** WebView 原生文件拖入、标签成员校验或跨窗口提交。

**Called by:** `src-tauri/src/lib.rs` 命令注册、`window/tab_transfer.rs`、`window/commands.rs`

**Depends on:** `windows-sys`, `tauri`, `window/tab_transfer.rs`, `window/external_open.rs`, `window/state.rs`

**Change this when:** 修改 Windows 鼠标追踪、跨屏落点、窗口遮挡命中或拖动取消。

**Related tests:** —

**Confidence:** high

---

### `src/app/services/readingPosition.ts`

**Kind:** service

**Owns:**
- 按标准化 filePath 维护 Markdown 阅读位置内存缓存
- 每个文件保存单一统一阅读语义锚点与锚点来源模式
- 兼容旧版 `semanticAnchor` / `sourceAnchor` 并按目标模式迁移
- 滚动停止后防抖写入应用配置，强制 flush 时立即持久化
- 多窗口配置合并时按 `updatedAt` 保留最新记录
- 按 `updatedAt` 裁剪最近 300 个文件的阅读位置

**Does not own:**
- 不拥有滚动锚点计算与滚动执行（在 outlineNavigation.ts 中）
- 不拥有标签页状态写入（在 tabs.ts / App.svelte 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/desktop/tauriStorage.ts`, `src/app/services/outlineNavigation.ts`

**Change this when:**
- 修改阅读位置持久化结构
- 修改 filePath 标准化或多窗口合并策略
- 修改防抖写入、强制 flush 或记录数量限制

**Related tests:** `src/app/services/readingPosition.test.ts`

**Confidence:** high

---

### `src/app/services/confirmAction.ts`

**Kind:** service

**Owns:**
- 确认对话框 Promise 式状态管理
- 二按钮模式（确认/取消）和三按钮模式（保存/放弃/取消）
- `confirmDialogStore` Svelte store
- 默认按钮文案的本地化兜底

**Does not own:**
- 不拥有确认对话框 UI（在 ConfirmDialog.svelte 中）

**Called by:** `src/app/App.svelte`, `src/app/services/documentActionsController.ts`

**Depends on:** `svelte/store`, `src/app/i18n.ts`

**Change this when:**
- 修改确认对话框交互模式
- 添加新的确认对话框类型
- 修改确认对话框默认按钮文案

**Do not change this when:**
- 修改对话框 UI 样式

**Related tests:** `src/app/services/confirmAction.test.ts`

**Confidence:** high

---

### `src/app/services/recoveryDraft.ts`

**Kind:** utility

**Owns:**
- 恢复草稿数据结构定义
- 写入 localStorage 的恢复草稿

**Does not own:**
- 不拥有草稿恢复触发逻辑（在 documentActionsController.ts 中）

**Called by:** `src/app/services/documentActionsController.ts`

**Depends on:** —

**Change this when:**
- 修改恢复草稿数据结构
- 修改草稿存储方式

**Related tests:** —

**Confidence:** high

---

### `src/app/services/outlineState.ts`

**Kind:** utility

**Owns:**
- 大纲展开/折叠状态计算纯函数
- 大纲项可见性判断
- 按行号查找当前大纲项
- 折叠 ID 集合裁剪

**Does not own:**
- 不拥有大纲数据提取（在 outlineService.ts 中）
- 不拥有大纲 UI（在 EditorWorkspace.svelte 中）

**Called by:** `src/app/services/outlineInteractionController.ts`, `src/app/components/EditorWorkspace.svelte`

**Depends on:** `src/lib/outline/outlineService.ts`

**Change this when:**
- 修改大纲展开/折叠逻辑
- 修改可见性计算算法

**Related tests:** `src/app/services/outlineState.test.ts`

**Confidence:** high

---

### `src/app/services/imageInsertion.ts`

**Kind:** controller

**Owns:**
- 图片粘贴/拖放导入流程协调
- 图片策略判断（本地复制 vs 图床上传）
- 源码模式 Markdown 图片语法插入
- 图片属性文本生成（width/align）
- 快捷插入图片的异步选区恢复；正文修订或编辑模式改变时取消替换

**Does not own:**
- 不拥有图片文件过滤（在 imageMarkdown.ts 中）
- 不拥有图片后端导入（在 desktopImageLoader.ts 中）
- 不拥有编辑器 insertImage 命令（在 editorCommands.ts 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/imageMarkdown.ts`, `src/lib/editor-core/renderers.ts`, `src/lib/services/render.ts`, `src/lib/services/logger.ts`

**Change this when:**
- 修改图片导入流程
- 修改图片策略选择逻辑
- 修改源码模式图片插入行为

**Do not change this when:**
- 修改图片后端处理
- 修改图片 NodeView 展示

**Related tests:** —

**Confidence:** high

---

### `src/app/services/clipboard.ts`

**Kind:** service

**Owns:**
- 语义编辑器剪贴板文本、安全 HTML 与图片读写协调
- 优先使用激活上下文中的 Web Clipboard API，并在桌面端降级到 Tauri 官方剪贴板插件
- 为普通粘贴和纯文本粘贴提供富内容优先/文本优先读取策略
- 将 Tauri RGBA 图片编码为 PNG `File`，交回既有图片导入流程

**Does not own:**
- 不拥有 ProseMirror 选区序列化和粘贴解析（在 `ProseMirrorEditorCore.ts` 中）
- 不拥有图片保存、资源目录或图床策略（在 `imageInsertion.ts` 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `@tauri-apps/plugin-clipboard-manager`, Web Clipboard API

**Change this when:**
- 修改桌面/浏览器剪贴板优先级、权限降级或图片编码桥接

**Related tests:** —

**Confidence:** high

---

### `src/app/services/imageMarkdown.ts`

**Kind:** utility

**Owns:**
- 从 FileList 过滤图片文件
- 生成图片相对路径（`./assets/` 下）
- 生成 Markdown 图片语法 `![alt](src)`

**Does not own:**
- 不拥有图片导入流程（在 imageInsertion.ts 中）

**Called by:** `src/app/services/imageInsertion.ts`

**Depends on:** —

**Change this when:**
- 修改图片路径生成策略
- 修改图片 Markdown 语法格式

**Related tests:** `src/app/services/imageMarkdown.test.ts`

**Confidence:** high

---

### `src/app/services/platform.ts`

**Kind:** utility

**Owns:**
- 平台检测（macOS / Windows / Linux）
- `PlatformCapabilities` 计算：Windows 自绘标题栏、窗口装饰与应用内菜单能力

**Does not own:**
- 不拥有具体窗口操作（在 desktopWindow.ts 中）

**Called by:** `src/app/services/desktopWindow.ts`, `src/app/components/AppTitleBar.svelte`, `src/app/components/AppShell.svelte`

**Depends on:** —

**Change this when:**
- 新增平台支持
- 修改窗口 chrome 模式判断

**Related tests:** `src/app/services/platform.test.ts`

**Confidence:** high

---

### `src/app/services/firstRunSample.ts`

**Kind:** utility

**Owns:**
- 首次运行样本文档状态判断
- `shouldOpenFirstRunSample` / `hasHandledFirstRunSample` 纯函数

**Does not own:**
- 不拥有样本文件复制（在 Rust 后端 file_system.rs 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/desktop/tauriStorage.ts`

**Change this when:**
- 修改首次运行判断条件
- 修改样本文档打开策略

**Related tests:** `src/app/services/firstRunSample.test.ts`

**Confidence:** high

---

### `src/app/services/appUiState.ts`

**Kind:** utility

**Owns:**
- 菜单展开/关闭切换纯函数
- 侧边栏 resize 事件处理工厂

**Does not own:**
- 不拥有侧边栏 UI（在 ExplorerSidebar.svelte 中）

**Called by:** `src/app/components/ExplorerSidebar.svelte`, `src/app/components/AppTitleBar.svelte`

**Depends on:** —

**Change this when:**
- 修改菜单切换逻辑
- 修改侧边栏 resize 策略

**Related tests:** —

**Confidence:** high

---

### `src/app/services/contextMenuPolicy.ts`

**Kind:** policy

**Owns:**
- 全窗口未处理右键事件的兜底规则：应用 chrome 禁用浏览器菜单，文本输入控件保留原生菜单
- `data-context-menu="native|none"` 显式覆盖，以及 TXT/JSON 分段编辑器的既有行为隔离

**Does not own:**
- 不拥有任何自定义菜单项或菜单状态

**Called by:** `src/app/components/AppShell.svelte`, `src/app/components/SettingsWindow.svelte`

**Depends on:** —

**Change this when:** 修改应用级“自定义、原生、禁用”右键边界

**Related tests:** —

**Confidence:** high

---

### `src/app/components/ContextMenu.svelte`

**Kind:** component

**Owns:**
- 通用上下文菜单 UI：语义图标、禁用/选中状态、一级与二级菜单、视口边界调整
- 完整键盘导航、可见焦点、滚动关闭与焦点恢复

**Does not own:**
- 不拥有菜单项定义和业务动作（由应用层或 NodeView 提供）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/editor-core/plugins/contextMenu.ts`

**Change this when:**
- 修改菜单样式或定位逻辑
- 修改菜单项渲染方式
- 修改二级菜单或键盘交互

**Do not change this when:**
- 修改菜单项数据来源

**Related tests:** —

**Confidence:** high

---

### `src/app/components/StatusBar.svelte`

**Kind:** component

**Owns:**
- 状态栏 UI：行数/词数/字数/字符统计展示、缩放百分比控制

**Does not own:**
- 不拥有统计数据计算（正文与范围索引在 writingStats.ts，后台调度在 writingStatsController.ts）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/lib/outline/outlineService.ts`, `src/app/actions/clickOutside.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改状态栏展示指标
- 修改缩放控件交互

**Do not change this when:**
- 修改统计计算逻辑

**Related tests:** `src/app/components/StatusBar.test.ts`

**Confidence:** high

---

### `src/app/components/FrontMatterCard.svelte`

**Kind:** component

**Owns:**
- Front matter YAML 元数据卡片 UI：展示/编辑/删除
- textarea 编辑态管理

**Does not own:**
- 不拥有 front matter 解析（在 frontMatter.ts 中）

**Called by:** `src/app/components/EditorWorkspace.svelte`

**Depends on:** `src/lib/markdown/frontMatter.ts`, `src/app/actions/clickOutside.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改 front matter 卡片样式
- 修改编辑交互

**Do not change this when:**
- 修改 YAML 解析规则

**Related tests:** —

**Confidence:** high

---

### `src/app/components/EmptyWorkspace.svelte`

**Kind:** component

**Owns:**
- 空工作区占位 UI：新建文件、打开文件、打开文件夹引导按钮

**Does not own:**
- 不拥有文件操作逻辑（通过回调传入）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/i18n.ts`

**Change this when:**
- 修改空工作区引导文案或布局

**Related tests:** —

**Confidence:** high

---

### `src/app/components/FolderOpenDialog.svelte`

**Kind:** component

**Owns:**
- 打开文件或文件夹的窗口选择对话框：当前窗口、新窗口、已有文件夹窗口；仅当前/新窗口选择可记为默认偏好

**Does not own:**
- 不拥有目标打开逻辑（通过 dispatch 事件传给父组件）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/i18n.ts`

**Change this when:**
- 修改打开目标的窗口选择 UI

**Related tests:** —

**Confidence:** high

---

### `src/app/components/LinkQuickEditor.svelte`

**Kind:** component

**Owns:**
- 链接快速编辑器弹出层：文字/地址输入、确认/删除/关闭

**Does not own:**
- 不拥有链接编辑逻辑（通过 props 回调传入）
- 不拥有链接安全校验（在 link.ts 中）

**Called by:** `src/app/components/AppShell.svelte`（通过 linkInteraction 插件回调）

**Depends on:** `src/app/actions/clickOutside.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改链接编辑器 UI 或交互

**Related tests:** —

**Confidence:** high

---

### `src/app/components/ConfirmDialog.svelte`

**Kind:** component

**Owns:**
- 通用确认对话框 UI：二按钮（确认/取消）和三按钮（保存/放弃/取消）模式

**Does not own:**
- 不拥有对话框状态管理（在 confirmAction.ts 中）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/services/confirmAction.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改确认对话框布局或按钮样式

**Related tests:** —

**Confidence:** high

---

### `src/app/components/UnsavedConfirmDialog.svelte`

**Kind:** component

**Owns:**
- 未保存文档确认对话框 UI：丢弃/取消按钮

**Does not own:**
- 不拥有确认逻辑（通过 props 回调传入）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/i18n.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改未保存确认 UI

**Related tests:** —

**Confidence:** high

---

### `src/app/components/ExternalChangeDialog.svelte`

**Kind:** component

**Owns:**
- 外部文件变更提示对话框 UI：重载/覆盖/忽略

**Does not own:**
- 不拥有外部变更处理逻辑（在 documentActionsController.ts 中）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/types.ts`, `src/app/i18n.ts`, `src/app/actions/motion.ts`

**Change this when:**
- 修改外部变更提示 UI

**Related tests:** —

**Confidence:** high

---

### `src/app/components/CloseWindowBehaviorDialog.svelte`

**Kind:** component

**Owns:**
- 关闭窗口行为选择对话框 UI：关闭窗口 vs 关闭到托盘、记住选择

**Does not own:**
- 不拥有关闭行为执行（通过 dispatch 事件传给父组件）

**Called by:** `src/app/components/AppShell.svelte`

**Depends on:** `src/app/actions/motion.ts`

**Change this when:**
- 修改关闭行为选择 UI

**Related tests:** —

**Confidence:** high

---

### `src/app/actions/clickOutside.ts`

**Kind:** utility (Svelte action)

**Owns:**
- Svelte `use:clickOutside` action：检测点击元素外部并触发回调

**Does not own:**
- 不拥有具体关闭逻辑（由使用方定义回调）

**Called by:** `ContextMenu.svelte`, `FrontMatterCard.svelte`, `StatusBar.svelte`, `LinkQuickEditor.svelte`

**Depends on:** —

**Change this when:**
- 修改外部点击检测策略

**Related tests:** —

**Confidence:** high

---

### `src/app/actions/motion.ts`

**Kind:** utility (Svelte action)

**Owns:**
- Svelte 过渡动画工具：`motionIn`、`transitionDuration`、`pulseOnChange`
- 工具栏收展、侧边栏、标签/目录选中底板和模式切换的 GSAP 动画
- 侧栏收展为推挤式：网格列只在起止各切换一次，期间侧栏与编辑区以 Web Animations 同步平移 transform（合成器执行），居中正文的重排位移由 .document-layout 反向平移补偿；兼容拖拽、小窗、中途反向与减少动态效果设置
- 活动标签共用插值的几何通知，供工作区轮廓同步填充、描边和阴影
- 统一的 fade/slide 动画配置与 reduced-motion 降级

**Does not own:**
- 不拥有具体组件的动画触发逻辑

**Called by:** `AppShell.svelte`、标签/侧边栏/编辑区组件及多个对话框和弹出层组件

**Depends on:** `gsap`, `svelte/transition`

**Change this when:**
- 修改全局动画时长或效果

**Related tests:** `src/app/actions/motion.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/types.ts`

**Kind:** model

**Owns:**
- 编辑器核心类型定义：`EditorMode`、`EditorCommand`（union type）、`SetMarkdownOptions`、`EditorSelectionSnapshot`、`EditorLinkSnapshot`、`InlinePendingMarks` 等
- `EditorThemeOptions`、`EditorRuntimeOptions`
- 快捷插入选择窗口事件与快照内容修订号，用于确认异步插入仍对应原文档选区

**Does not own:**
- 不拥有具体命令实现（在 editorCommands.ts 中）

**Called by:** 所有使用 EditorCore 的前端模块

**Depends on:** `src/lib/editor-core/diagramTemplates.ts`, `src/lib/services/render.ts`, `src/lib/editor-core/plugins/contextMenu.ts`

**Change this when:**
- 新增/修改 EditorCommand 类型
- 修改编辑器选项接口

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/diagramTemplates.ts`

**Kind:** model

**Owns:**
- Mermaid 图表模板定义：flowchart、sequenceDiagram、classDiagram、stateDiagram、pie、gantt、erDiagram
- `DiagramType` 类型

**Does not own:**
- 不拥有图表渲染（在 mermaidDiagramRenderer.ts 中）
- 不拥有图表插入命令（在 editorCommands.ts 中）

**Called by:** `src/app/components/EditorToolbar.svelte`, `src/lib/editor-core/types.ts`

**Depends on:** —

**Change this when:**
- 新增图表类型模板
- 修改现有模板代码

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/link.ts`

**Kind:** utility

**Owns:**
- 超链接安全校验与规范化：拒绝 `javascript:` 等脚本协议，允许 `https?`/`mailto`/相对路径
- 链接属性创建与序列化

**Does not own:**
- 不拥有链接交互 UI（在 linkInteraction.ts 和 LinkQuickEditor.svelte 中）

**Called by:** `src/lib/editor-core/plugins/linkInteraction.ts`, `src/lib/editor-core/markdown.ts`, `src/quicklook/preview.ts`

**Depends on:** —

**Change this when:**
- 修改链接协议白名单
- 修改链接序列化规则

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/html/htmlPolicy.ts`

**Kind:** model

**Owns:**
- HTML 安全白名单：可编辑块级标签（section/div）、内联标签、允许属性
- 危险标签集合（script/iframe/form 等）
- 内联标签到 ProseMirror mark 的映射表

**Does not own:**
- 不拥有 HTML 分类逻辑（在 htmlClassifier.ts 中）

**Called by:** `src/lib/editor-core/html/htmlClassifier.ts`, `src/lib/editor-core/html/htmlToPmLogic.ts`

**Depends on:** —

**Change this when:**
- 新增可编辑 HTML 标签
- 修改安全白名单

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/html/htmlClassifier.ts`

**Kind:** utility

**Owns:**
- 无 DOM 的 HTML 块分类：仅单根及可保真的行内结构可编辑；其它结构/属性保留原文 fallback
- 按实际属性边界提取、实体解码与安全协议校验
- 与 HTML 导入共用现有行内原子节点的属性识别（图片、换行、数学、注释、脚注）

**Does not own:**
- 不拥有标签/mark 映射白名单（在 htmlPolicy.ts 中）
- 不拥有 DOM 导入与 HTML 输出（在 htmlToPmLogic.ts、pmToHtml.ts 中）

**Called by:** `src/lib/editor-core/markdown.ts`、`src/lib/editor-core/html/htmlToPmLogic.ts`、`src/lib/outline/writingStats.ts`

**Depends on:** `htmlPolicy.ts`、`../markdownTokenizer.ts`、`htmlAnchor.ts`、`../link.ts`

**Change this when:**
- 修改 HTML 块可编辑性判断规则
- 修改属性提取逻辑

**Related tests:** `src/lib/editor-core/html/__tests__/htmlClassifier.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/utils/html.ts`

**Kind:** utility

**Owns:**
- `escapeHtml`：HTML 特殊字符转义
- `sanitizeHtml`：简单 HTML 安全过滤（检测 script/iframe/on* 事件）
- `renderHtmlPreview`：编辑器与导出共用安全 HTML 的展示结构、对齐段落与空白折叠

**Does not own:**
- 不拥有 HTML 块分类（在 htmlClassifier.ts 中）

**Called by:** `src/quicklook/preview.ts`, `src/lib/editor-core/plugins/tableHtml.ts`, `src/app/services/export/render.ts`, 渲染相关模块

**Depends on:** —

**Change this when:**
- 修改 HTML 转义/过滤规则

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/plugins/trailingParagraph.ts`

**Kind:** plugin

**Owns:**
- 尾部段落补全插件：顶层非段落块插入后自动追加空段落
- 插入范围追踪与映射

**Does not own:**
- 不拥有具体块插入逻辑

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** `src/lib/editor-core/schema.ts`

**Change this when:**
- 修改尾部段落补全策略
- 修改插入范围检测逻辑

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/plugins/tocSync.ts`

**Kind:** plugin

**Owns:**
- 标题内容或等级变化后的 TOC 节点属性同步
- 基于同一文档标题映射派生 DOM heading id Decoration，不写持久节点属性
- 将 TOC 派生事务排除在独立撤销步骤之外
- 基于事务变更范围跳过普通正文编辑

**Does not own:**
- 不拥有 TOC Markdown 生成规则（由 tocService.ts 提供）
- 不拥有 TOC 展示和跳转（由 TocBlockNodeView.ts 提供）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** `src/lib/editor-core/schema.ts`, `src/lib/toc/tocService.ts`, `prosemirror-view`

**Change this when:**
- 修改标题变化后的目录同步时机
- 修改 TOC 派生事务的撤销历史策略

**Related tests:** `src/lib/editor-core/createEditorCore.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/plugins/contextMenu.ts`

**Kind:** plugin

**Owns:**
- 编辑器上下文菜单 ProseMirror 插件与 DOM 菜单工厂挂载/查找机制
- 将右键坐标映射到 ProseMirror 位置，并按“对象 → 选区 → 正文”识别语义目标
- 在当前选区内保留选区，选区外将光标移动到命中位置
- `ContextMenuItem`、`ContextMenuTarget`、`ContextMenuOpenEvent`、`ContextMenuRequest`、`ContextMenuCapable` 类型定义

**Does not own:**
- 不拥有菜单 UI 渲染（在 ContextMenu.svelte 中）
- 不拥有通用菜单项和应用级动作生成（在 `App.svelte` 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** `prosemirror-state`, `prosemirror-view`

**Change this when:**
- 修改右键菜单事件处理
- 修改菜单工厂挂载机制
- 修改目标命中优先级或选区定位语义

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/plugins/headingLevelIndicator.ts`

**Kind:** plugin

**Owns:**
- 当前 H1-H6 标题层级角标的选区识别、浮层定位与显隐
- 标题角标的 GSAP 动画、响应式安全留白和生命周期清理

**Does not own:**
- 不拥有标题 Schema、Markdown 序列化、导出或大纲数据
- 不拥有角标的主题化 CSS 外观（在 editor-document.css 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** `gsap`, `prosemirror-state`, `prosemirror-view`

**Change this when:**
- 修改角标触发条件、定位逻辑或 GSAP 显隐动画
- 修改窄窗口安全留白规则

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/plugins/paragraphSelectionGutter.ts`

**Kind:** plugin / view overlay

**Owns:** 正文外侧的单个复用选择入口、顶层普通段落与标题的悬停命中、真实文本选区、缩放定位和监听器／Observer／帧任务释放。

**Does not own:** 正文布局、Markdown 内容、选区背景绘制、列表／表格／代码块选择、触摸或正文拖选。

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** `prosemirror-state`, `prosemirror-view`, `activeEditRegistry.ts`, `inlineSourceEditing.ts`, `src/app/i18n.ts`；外观位于 `src/app/styles/editor-document.css`。

**Change this when:** 修改整段选择入口的触发范围、选择语义、定位或生命周期。

**Related tests:** `src/lib/editor-core/plugins/paragraphSelectionGutter.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/plugins/codeHighlightDecorationPlugin.ts`

**Kind:** plugin

**Owns:**
- 行内代码语法高亮 Decoration 插件
- 轻量 token 分类器（关键字/布尔值/数字/字符串/运算符）
- 全量扫描策略，每次 state 变化重新计算

**Does not own:**
- 不拥有代码块高亮（在 codeHighlight.ts 和 shikiCodeTokenizer.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** —

**Change this when:**
- 修改行内代码高亮规则
- 修改 token 分类逻辑

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/activeEditRegistry.ts`

**Kind:** utility

**Owns:**
- 跨 NodeView 编辑态协调注册表，每篇文档独立维护活动编辑实例
- 同一文档内切换编辑对象时请求结束上一个编辑态，并保留尚未结束的组字实例
- 保存时等待全部节点组字完成；运行时切只读取消活动编辑
- 通过可选原位提交回调，让代码块在后台保存和快照时保留编辑态；模式切换可显式结束编辑态

**Does not own:**
- 不拥有具体 NodeView 的编辑态实现

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`，代码、公式、图表、图片和注释 NodeView

**Depends on:** —

**Change this when:**
- 修改编辑态互斥策略

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/CommentBlockNodeView.ts`

**Kind:** nodeView

**Owns:**
- 块级 Markdown 注释 NodeView：卡片展示、点击编辑、textarea 编辑态

**Does not own:**
- 不拥有注释解析（在 markdown.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（NodeView 注册）

**Depends on:** `src/lib/editor-core/nodeViews/activeEditRegistry.ts`, `src/app/i18n.ts`

**Change this when:**
- 修改注释块展示/编辑行为

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/CommentInlineNodeView.ts`

**Kind:** nodeView

**Owns:**
- 行内 Markdown 注释 NodeView：正文内注释标签展示、支持换行的原位 textarea 编辑、只读与输入法组字协调

**Does not own:**
- 不拥有注释解析（在 markdown.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（NodeView 注册）

**Depends on:** `src/lib/editor-core/nodeViews/activeEditRegistry.ts`, `src/app/i18n.ts`

**Change this when:**
- 修改行内注释展示/编辑行为

**Related tests:** `src/lib/editor-core/nodeViews/CommentInlineNodeView.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/FootnoteDefNodeView.ts`

**Kind:** nodeView

**Owns:**
- 底部脚注定义 NodeView：定义标记、返回正文入口、内容区原生编辑

**Does not own:**
- 不拥有脚注解析（在 markdown.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（NodeView 注册）

**Depends on:** `src/app/i18n.ts`

**Change this when:**
- 修改脚注定义展示/交互

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/FootnoteRefNodeView.ts`

**Kind:** nodeView

**Owns:**
- 正文脚注引用 NodeView：跳转到底部定义、hover/focus 只读预览卡片与文档更新后已开预览刷新

**Does not own:**
- 不拥有脚注解析（在 markdown.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（NodeView 注册及文档更新后的预览刷新）

**Depends on:** `src/app/i18n.ts`, `src/lib/editor-core/InlineSourceCodec.ts`

**Change this when:**
- 修改脚注引用展示/预览行为

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/nodeViews/HorizontalRuleNodeView.ts`

**Kind:** nodeView

**Owns:**
- 水平分割线 NodeView：渲染 `<hr>`、点击选中（NodeSelection）

**Does not own:**
- 不拥有分割线解析（在 markdown.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（NodeView 注册）

**Depends on:** —

**Change this when:**
- 修改分割线渲染或选中行为

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/callout/calloutTypes.ts`

**Kind:** model

**Owns:**
- Callout 类型定义：5 种固定类型（note/tip/important/warning/caution）
- 类型配置表（图标、默认标题、颜色后缀）
- 多语言标签映射（zh-CN/zh-TW/en-US）

**Does not own:**
- 不拥有 Callout 解析/序列化（在 calloutParser.ts/calloutSerializer.ts 中）

**Called by:** `src/lib/editor-core/callout/calloutSchema.ts`, `src/lib/editor-core/nodeViews/CalloutNodeView.ts`

**Depends on:** —

**Change this when:**
- 新增 Callout 类型
- 修改类型配置或标签

**Related tests:** —

**Confidence:** high

---

### `src/lib/editor-core/callout/calloutPlugin.ts`

**Kind:** plugin

**Owns:**
- Callout ProseMirror 插件（当前仅保留插件位，键盘行为由 calloutCommands 处理）

**Does not own:**
- 不拥有 Callout 命令（在 calloutCommands.ts 中）

**Called by:** `src/lib/editor-core/ProseMirrorEditorCore.ts`（插件注册）

**Depends on:** —

**Change this when:**
- 需要为 Callout 添加插件级别的键盘/事务行为

**Related tests:** —

**Confidence:** high

---

### `src/lib/services/storage.ts`

**Kind:** model

**Owns:**
- 文件存储接口定义：`FileStorage`（open/save/saveAs）、`DocumentRepository`（rememberRecentFile/listRecentFiles/createSnapshot）
- `OpenDocumentResult`、`SaveDocumentInput`、`DocumentSnapshotRecord` 类型
- Markdown 源编码类型、默认值和兼容归一化规则

**Does not own:**
- 不拥有具体实现（在 tauriStorage.ts 和 Rust 后端中）

**Called by:** `src/lib/desktop/tauriStorage.ts`

**Depends on:** —

**Change this when:**
- 修改存储接口契约

**Related tests:** —

**Confidence:** high

---

### `src/lib/services/render.ts`

**Kind:** model

**Owns:**
- 渲染服务类型接口：`ImageLoader`、`CodeTokenizer`、`MathRenderer`、`DiagramRenderer`
- 图片处理设置类型：`ImageHandlingSettings`、`ImageInsertStrategy`、`ImageUploadProvider`
- `ImageContext`、`ImageResolveResult`、`ImageImportResult` 等

**Does not own:**
- 不拥有具体渲染实现（在 shikiCodeTokenizer.ts、katexMathRenderer.ts、mermaidDiagramRenderer.ts 中）

**Called by:** `src/lib/services/shikiCodeTokenizer.ts`, `src/lib/services/katexMathRenderer.ts`, `src/lib/services/mermaidDiagramRenderer.ts`, `src/app/services/desktopImageLoader.ts`

**Depends on:** —

**Change this when:**
- 新增渲染服务类型
- 修改图片处理设置结构

**Related tests:** —

**Confidence:** high

---

### `src/lib/services/logger.ts`

**Kind:** service

**Owns:**
- 前端全局日志工具：debug/info/warn/error 级别
- DevTools 输出和 Tauri 原生日志转发
- 日志缓冲区（最大 500 条）
- 性能计时器（`createPerfTimer`、`perf`、`perfAsync`）
- `window.NomoLogger` 控制台控制接口

**Does not own:**
- 不拥有后端日志落盘（在 app_logger.rs 中）

**Called by:** 前端各模块（通过 `logInfo`/`logError` 等）

**Depends on:** `@tauri-apps/api/core`（可选）

**Change this when:**
- 修改日志输出策略
- 修改性能计时逻辑

**Related tests:** —

**Confidence:** high

---

### `src/lib/markdown/MarkdownBridge.ts`

**Kind:** service

**Owns:**
- Markdown 桥接：将 Markdown 文本拆分为 front matter + 正文，以及反向合并
- `MarkdownDocument` 接口

**Does not own:**
- 不拥有 front matter 解析细节（在 frontMatter.ts 中）
- 不拥有 ProseMirror 文档转换（在 markdown.ts 中）

**Called by:** 编辑器/文档流程

**Depends on:** `src/lib/markdown/frontMatter.ts`

**Change this when:**
- 修改 front matter 与正文的分离/合并策略

**Related tests:** `src/lib/markdown/MarkdownBridge.test.ts`

**Confidence:** high

---

### `src-tauri/src/export.rs`

**Kind:** service

**Owns:**
- HTML 导出 IPC：写入 HTML 文件
- Base64 文件读取
- 临时 HTML 文件写入/清理（供 PDF 导出使用）

**Does not own:**
- 不拥有 PDF 生成（在 export_windows.rs 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）, `src-tauri/src/export_windows.rs`

**Depends on:** `src-tauri/src/models.rs`, `src-tauri/src/app_logger.rs`

**Change this when:**
- 修改 HTML 导出逻辑
- 修改临时文件策略

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/export_windows.rs`

**Kind:** service

**Owns:**
- Windows PDF 导出：通过隐藏 WebView2 的 PrintToPdf 生成 PDF
- 字体、图片、目标版心宽高与精细排版完成门禁

**Does not own:**
- 不拥有 HTML 临时文件写入（在 export.rs 中）

**Called by:** `src-tauri/src/export.rs`（在原生 PDF 任务中调用）

**Depends on:** `src-tauri/src/export.rs`, `src-tauri/src/models.rs`

**Change this when:**
- 修改 PDF 生成方式
- 修改打印纸张、边距或就绪检查

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/app_logger.rs`

**Kind:** service

**Owns:**
- 后端日志系统：文件落盘（`./logs/`）、5MB 轮转、终端输出
- 日志开关 IPC（`set_logger_enabled`、`get_logger_enabled`、`log_message`）
- 前端日志转发接收

**Does not own:**
- 不拥有前端日志生成（在 logger.ts 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）, 后端各模块（通过 `info`/`debug`/`error` 等）

**Depends on:** `chrono`, `src-tauri/src/config/mod.rs`

**Change this when:**
- 修改日志文件策略
- 修改日志轮转规则

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/config/commands.rs`

**Kind:** service

**Owns:**
- 数据操作 IPC 命令（原 database 层迁移至此）：
  - 最近打开：`remember_recent_entry`、`list_recent_entries`、`clear_recent_entries`
  - 文档快照索引与正文文件：`create_document_snapshot`、`list_document_snapshots`
  - 工作区草稿正文文件：`write_workspace_draft`、`read_workspace_draft`、`delete_workspace_draft`
  - 应用设置：`update_app_setting`、`update_app_settings`、`list_app_settings`
- 内部工具：`query_recent_entries`、`get_setting_value`、正文文件读写、设置路由到 AppConfig section

**Does not own:**
- 不拥有配置存储/结构定义（在 config/mod.rs 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）

**Depends on:** `src-tauri/src/config/mod.rs`, `src-tauri/src/models.rs`, `src-tauri/src/file_system.rs`

**Change this when:**
- 新增数据操作 IPC 命令
- 修改最近打开/快照/草稿/设置的查询逻辑

**Related tests:** `src-tauri/src/config/commands.rs` 模块内测试

**Confidence:** high

---

### `src-tauri/src/window/commands.rs`

**Kind:** service

**Owns:**
- 窗口相关 IPC 命令：常规窗口状态更新、设置窗口、菜单、授权关闭，以及 Markdown 小窗进入/返回/置顶
- 隐藏文档窗口完成无边框样式与几何初始化后的显示时机

**Does not own:**
- 不拥有窗口状态持久化逻辑（在 window/state.rs 中）
- 不拥有菜单构建（在 window/menu.rs 中）

**Called by:** `src-tauri/src/lib.rs`（注册为 IPC）

**Depends on:** `src-tauri/src/window/state.rs`, `src-tauri/src/window/menu.rs`, `src-tauri/src/window/os/mod.rs`, `src-tauri/src/window/tray.rs`, `src-tauri/src/config/commands.rs`

**Change this when:**
- 新增窗口相关 IPC 命令

**Related tests:** —

**Confidence:** high

---

### `src/app/services/markdownMiniTransition.ts`

**Kind:** transition controller

**Owns:** 小窗进入/返回共用的目标视口稳定等待、超时/销毁清理和精细排版暂停生命周期；切换期间保持窗口与正文可见。

**Does not own:** 原生窗口 API、编辑器模式或文档状态、阅读锚点计算。

**Called by:** `src/app/App.svelte`

**Depends on:** `src/lib/typography/interaction.ts`

**Confidence:** high

---

### `src-tauri/src/window/state.rs`

**Kind:** window state service

**Owns:**
- 普通窗口与 Markdown 小窗的位置、尺寸和最大化状态持久化
- 进入小窗前的运行时窗口快照，以及返回普通模式时的几何恢复
- 小窗最小尺寸、置顶、任务栏可见性与目标屏幕边界约束
- 进入/返回时保持原生窗口可见，合并 Windows 同 DPI 下的位置与尺寸更新，并在跨 DPI 时先完成屏幕适配

**Does not own:**
- 不拥有前端小窗内容和标题栏 UI
- 不拥有 IPC 命令注册

**Called by:** `src-tauri/src/lib.rs`, `src-tauri/src/window/commands.rs`

**Depends on:** Tauri window and monitor APIs, `src-tauri/src/window/os/windows.rs`, `src-tauri/src/models.rs`, `src-tauri/src/config/mod.rs`

**Change this when:**
- 修改窗口几何保存/恢复、小窗定位、置顶或模式切换的原生行为

**Related tests:** —

**Confidence:** high

---

### `src-tauri/src/window/os/windows.rs`

**Kind:** Windows platform adapter

**Owns:**
- 校验 Windows 窗口是否已按平台配置创建为无装饰窗口，并在配置漂移时兜底关闭装饰、保留系统阴影。
- 读取 Windows 应用主题，以及外部打开时将窗口带到前台。
- 通过启动插件为主窗口、新窗口和设置窗口禁用浏览器默认快捷键，原生按键事件即时兜底首次导航，保留前端与编辑器按键分发。
- 小窗切换时用一次 SetWindowPos 更新普通窗口的位置和客户区尺寸，保留系统阴影边距、焦点与 Z 序。

**Does not own:**
- 不拥有前端自绘按钮、标题栏布局或关闭业务。
- 不复制 Tao 已负责的缩放命中、最大化工作区和普通 Snap 行为。

**Called by:** `src-tauri/src/window/os/mod.rs`, `src-tauri/src/lib.rs`（注册 Windows 浏览器快捷键插件）

**Depends on:** `src-tauri/tauri.windows.conf.json`、Tauri window/plugin API、WebView2 COM、Windows Registry 与 WindowsAndMessaging API

**Change this when:**
- 修改 Windows 无边框、阴影、系统主题、窗口前置或浏览器默认快捷键行为。

**Related tests:** `src/app/App.layout.test.ts`（源码接线断言）；真实行为需 Windows 实窗验证

**Confidence:** high

---

### `src-tauri/src/window/mod.rs`

**Kind:** entry

**Owns:**
- 窗口子模块声明：commands、external_open、file_association、menu、open_targets、os、state、tray、workspace_lifecycle、tab_transfer、tab_drag

**Does not own:**
- 不拥有具体业务逻辑

**Called by:** `src-tauri/src/lib.rs`

**Depends on:** —

**Change this when:**
- 新增窗口子模块

**Related tests:** —

**Confidence:** high

---

### `src/app/services/desktopWindow.ts`

**Kind:** service

**Owns:**
- 桌面窗口关闭、退出与设置窗口打开操作
- 新窗口创建时的 chrome 选项（macOS overlay / Windows 无装饰）及 Windows 隐藏初始化
- 打开目标联合类型、窗口目标同步/准备 IPC 与预留窗口创建失败清理
- 当前文档窗口自身的激活 IPC 适配（`activateDocumentWindow`）
- Markdown 小窗进入、返回和置顶 IPC 的前端适配

**Does not own:**
- 不拥有窗口状态持久化（在 Rust window/state.rs 中）
- 不拥有关闭到托盘逻辑（在 Rust window/tray.rs 中）

**Called by:** `src/app/App.svelte`, `src/app/components/AppShell.svelte`

**Depends on:** `@tauri-apps/api/window`, `@tauri-apps/api/dpi`, `@tauri-apps/api/core`, `src/app/services/platform.ts`

**Change this when:**
- 修改窗口操作行为
- 修改新窗口 chrome 选项
- 修改 Markdown 小窗的前端原生窗口调用

**Related tests:** —

**Confidence:** high

---

### `src/app/services/editorSettingsController.ts`

**Kind:** controller

**Owns:**
- 编辑器排版设置应用控制器：将字体、行高和内容宽度同步到 EditorCore

**Does not own:**
- 不拥有设置模型定义（在 settings.ts 中）
- 不拥有配色主题或文档样式应用（在 themeManager.ts 中）
- 不拥有 EditorCore 内部实现

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/settings.ts`, `src/lib/editor-core/types.ts`

**Change this when:**
- 修改编辑器设置同步逻辑
- 新增编辑器设置项

**Related tests:** —

**Confidence:** high

---

### `src/app/services/desktopImageLoader.ts`

**Kind:** service

**Owns:**
- 桌面图片加载器实现：resolve/import/remove
- 通过 Tauri IPC 调用后端图片资源处理

**Does not own:**
- 不拥有图片后端处理逻辑（在 Rust file_system/image_assets.rs 中）
- 不拥有图片 NodeView 展示（在 ImageNodeView.ts 中）

**Called by:** `src/app/App.svelte`（注册到 renderers.ts）

**Depends on:** `src/lib/desktop/tauriStorage.ts`, `src/lib/services/render.ts`

**Change this when:**
- 修改图片 resolve/import/remove 前端调用
- 修改图片 context 构建逻辑

**Related tests:** —

**Confidence:** high

---

### `src/app/services/outlineInteractionController.ts`

**Kind:** controller

**Owns:**
- 大纲交互控制：点击定位、全部展开/折叠、章节移动和状态恢复
- 源码模式原生可撤销文本替换，并在不支持时保持文档不变

**Does not own:**
- 不拥有大纲数据计算（在 outlineService.ts 中）
- 不拥有滚动定位实现（在 outlineNavigation.ts 中）

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/services/outlineNavigation.ts`, `src/app/services/outlineState.ts`, `src/lib/outline/outlineReorder.ts`

**Change this when:**
- 修改大纲点击、章节移动或源码撤销接入行为

**Related tests:** `src/app/services/outlineInteractionController.test.ts`

**Confidence:** high

---

### `src/app/services/outlineNavigation.ts`

**Kind:** service

**Owns:**
- 大纲滚动定位：按大纲锚点恢复编辑区视觉焦点
- 源码与语义视图之间基于统一语义锚点滚动同步
- 源码滚动坐标与 textarea 内容偏移换算
- 阅读位置恢复专用滚动：同源恢复可使用像素锚点，跨模式恢复使用大纲段落进度 / 文档进度

**Does not own:**
- 不拥有大纲数据计算（在 outlineService.ts 中）
- 不拥有阅读位置持久化（在 readingPosition.ts 中）

**Called by:** `src/app/services/outlineInteractionController.ts`, `src/app/services/editorInteractionController.ts`

**Depends on:** `src/lib/editor-core/types.ts`

**Change this when:**
- 修改滚动定位算法
- 修改模式切换时的滚动同步

**Related tests:** `src/app/services/outlineNavigation.test.ts`

**Confidence:** high

---

### `src/app/components/SegmentedTextEditorWorkspace.svelte`

**Kind:** workspace controller / Svelte component

**Owns:**
- TXT/JSON 分段编辑器的生命周期、滚动事件与窗口切换
- TXT/JSON 全文搜索分页、计数、单次替换和后台全部替换任务协调
- 基于文件字节进度的固定全文滚动跑道；索引进度不得改变已校准的全局高度
- 快速拖动时的小预览窗口、只读门禁、空闲后扩展到正式窗口
- 后端首窗口不足正式窗口时的前端补读与原子切换

**Does not own:**
- 不拥有 CodeMirror 局部文档状态（在 `SegmentedTextEditorCore.ts`）
- 不拥有字节与滚动位置的纯映射算法（在 `virtualScroll.ts`）
- 不拥有 Rust 文件读取和索引任务

**Called by:** `src/app/App.svelte`

**Depends on:** `src/app/components/SearchReplacePanel.svelte`, `src/lib/text-editor/SegmentedTextEditorCore.ts`, `src/lib/text-editor/viewportController.ts`, `src/lib/text-editor/virtualScroll.ts`

**Change this when:**
- 修改 TXT/JSON 大文件的滚动、快速定位、加载状态或窗口切换体验

**Related tests:** `src/app/components/SegmentedTextEditorWorkspace.test.ts`

**Confidence:** high

---

### `src/lib/text-editor/virtualScroll.ts`

**Kind:** utility / model

**Owns:**
- 正式窗口、快速预览、拖动节流和空闲扩展的统一参数
- 首次校准后冻结的像素/字节比例及全文滚动高度
- 字节偏移与全局滚动位置的双向映射
- 快速定位预览窗口大小与居中起点计算

**Does not own:**
- 不发起文件读取
- 不控制 CodeMirror 或 Svelte DOM

**Called by:** `src/app/components/SegmentedTextEditorWorkspace.svelte`

**Depends on:** —

**Change this when:**
- 修改全文滚动高度模型、窗口大小或快速定位参数

**Related tests:** `src/lib/text-editor/virtualScroll.test.ts`

**Confidence:** high

---

### `src/lib/text-editor/SegmentedTextEditorCore.ts`

**Kind:** editor core

**Owns:**
- CodeMirror 局部窗口内容、全局字节锚点和选区映射
- 当前可见全局字节范围的单次替换，并复用普通编辑批次与撤销链路
- 加载/快速定位期间的交互只读门禁
- 在不改变选区的前提下，将目标字节位置滚动进当前局部视口
- 编辑批次、撤销重做和 JSON 增量高亮接线

**Does not own:**
- 不拥有全文滚动跑道和快速拖动状态机
- 不直接读取后端窗口

**Called by:** `src/app/components/SegmentedTextEditorWorkspace.svelte`

**Depends on:** CodeMirror 6, `positionMapping.ts`, `editBatch.ts`, `jsonLexer.ts`

**Change this when:**
- 修改局部编辑、选区/锚点映射、只读门禁或窗口内容替换

**Related tests:** `src/lib/text-editor/SegmentedTextEditorCore.test.ts`

**Confidence:** high

---

### `src/lib/text-editor/viewportController.ts`

**Kind:** controller

**Owns:**
- 分段窗口读取、LRU 缓存和过期响应保护
- 预览窗口与正式窗口的区分；正式读取不得错误复用过短预览
- 正式窗口加载后的前向优先、后向补充预取
- 正式窗口覆盖预览后清理临时缓存

**Does not own:**
- 不拥有滚动几何
- 不拥有后端实际文件读取实现

**Called by:** `src/app/components/SegmentedTextEditorWorkspace.svelte`

**Depends on:** `chunkCache.ts`, `protocol.ts`

**Change this when:**
- 修改窗口读取策略、缓存、乱序保护、预览扩展或预取顺序

**Related tests:** `src/lib/text-editor/viewportController.test.ts`

**Confidence:** high

---

### `src/app/styles/editor-segmented.css`

**Kind:** stylesheet

**Owns:**
- 分段编辑器的固定像素视口、粘性定位和全文滚动跑道布局
- 隐藏 CodeMirror 内层滚动条，只保留外层全文滚动条
- 窗口切换时的轻量加载遮罩

**Does not own:**
- 不拥有滚动状态或文件读取逻辑

**Called by:** `src/app/App.svelte`

**Depends on:** `SegmentedTextEditorWorkspace.svelte`

**Change this when:**
- 修改分段编辑器布局、滚动条或加载提示样式

**Related tests:** `src/app/App.layout.test.ts`, `src/app/components/SegmentedTextEditorWorkspace.test.ts`

**Confidence:** high

---


### `src/lib/outline/writingStats.ts`

**Kind:** pure statistics / index

**Owns:** 共享 tokenizer 的源码正文位置映射、ProseMirror 正文投影、中英文词边界和 Unicode 可见字符边界、累计范围查询。

**Does not own:** DOM、选区修改、文件 IO 或 Worker 生命周期。

**Related code:** `sourceTokenPositions.ts`, `writingStatsEngine.ts`, `writingStatsProtocol.ts`；后两者管理 Worker 内的单文档缓存与消息契约。

**Related tests:** `src/lib/outline/writingStats.test.ts`, `src/lib/outline/writingStatsEngine.test.ts`

**Confidence:** high

---

### `src/app/services/writingStatsController.ts`

**Kind:** controller

**Owns:** 单个统计 Worker、120 ms 最新选区防抖、内容修订快照、跨模式索引复用、请求合并、文档/模式/选区失效门禁、取消与错误状态。

**Called by:** `src/app/App.svelte`；重计算入口为 `src/lib/outline/writingStats.worker.ts`。

**Related tests:** `src/app/services/writingStatsController.test.ts`

**Confidence:** high

---

### `src/lib/editor-core/markdownTokenizer.ts` / `markdownSerialization.ts`

**Kind:** pure Markdown syntax / serialization

**Owns:** 可复用的 markdown-it 规则工厂、表格/Callout/目录 token 归一化、无 DOM 的 Markdown 与选区/剪贴板纯文本序列化；普通 GFM 表格及不能保真的合并格、无表头表格 HTML 保存。

**Called by:** `markdown.ts`、正文统计与统计 Worker；原模块继续重导出序列化 API。

**Statistics path:** 语义选区的字符数和行数复用序列化规则流式计量，避免生成完整选区字符串；剪贴板和保存仍使用原有字符串序列化入口。

**Related tests:** `src/lib/editor-core/markdown.test.ts`, `src/lib/editor-core/createEditorCore.test.ts`

**Confidence:** high

---

## 文件与目录速查

| 目录 | 用途 |
|---|---|
| `src/app/` | 前端 UI 层：应用壳、组件、控制器、服务、样式 |
| `src/app/components/` | Svelte UI 组件 |
| `src/app/services/` | 前端业务逻辑控制器和服务 |
| `src/app/actions/` | Svelte actions（clickOutside、motion） |
| `src/app/styles/` | CSS 样式文件（主题、布局、编辑器、表格等） |
| `src/lib/editor-core/` | ProseMirror 编辑器核心 |
| `src/lib/emoji/` | Emoji 双语索引、共享建议浮层、源码输入适配与有界围栏上下文 |
| `src/lib/editor-core/callout/` | Callout 提示块（schema、parser、serializer、命令、插件） |
| `src/lib/editor-core/html/` | HTML 块安全策略与转换 |
| `src/lib/editor-core/nodeViews/` | 各类 NodeView 实现 |
| `src/lib/editor-core/plugins/` | ProseMirror 插件 |
| `src/lib/markdown/` | Markdown 桥接、front matter、保存归一化 |
| `src/lib/markdown-document/` | Markdown 统一会话、真实窗口宿主、窗口投影、渐进结构索引、目录派生及完整快照任务 |
| `src/lib/outline/` | 大纲服务 |
| `src/lib/toc/` | TOC 服务 |
| `src/lib/services/` | 渲染服务接口与实现（Shiki、KaTeX、Mermaid） |
| `src/lib/desktop/` | Tauri IPC 适配（tauriStorage、tauriUpdater） |
| `src/lib/text-editor/` | TXT/JSON 大文件分段编辑、窗口缓存与全文虚拟滚动 |
| `src/quicklook/` | macOS Quick Look 预览 |
| `src/paraglide/` | Inlang/Paraglide 生成的本地化代码（**不要手改**） |
| `src-tauri/src/` | Rust 后端 |
| `src-tauri/src/config/` | JSON 配置管理（ConfigManager、commands）：设置、最近打开、快照索引、工作区状态/草稿正文仓库、窗口状态 |
| `src-tauri/src/file_system/` | 文件系统与图片资源 |
| `src-tauri/src/text_document/` | TXT/JSON 分段文档，以及 Markdown 原文会话、编码映射、磁盘历史与快照保存 |
| `src-tauri/src/window/` | 窗口、菜单、托盘、外部打开、文件关联 |
| `project.inlang/` | Inlang 本地化文案源文件（**修改这里而非 paraglide/**） |

---

## 置信度与漂移标记

- **high**: 基于代码扫描确认的职责划分。
- `src/app/App.svelte` 和 `src/app/components/SettingsWindow.svelte` 体积偏大，未来拆分后可能需要更新相关条目。
- `src/lib/editor-core/markdown.ts` 和 `editorCommands.ts` 体积极大，未来拆分为子模块后需要更新条目。
- `src-tauri/src/file_system.rs` 未来拆分后需要更新条目。
- `src-tauri/src/config/mod.rs` 承担了原 SQLite 数据库的全部职责（最近打开、快照、设置），未来若拆分需更新条目。
- `src/app/i18n.ts` 和 `src/app/i18n.ja.ts` 未单独建条目（已在 Feature Index 本地化节覆盖）。
