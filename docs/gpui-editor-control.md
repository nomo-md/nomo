# GPUI 编辑入口与当前边界

默认 `nomo [文件或目录 ...]` 已使用 `Workspace → DocumentHost → EditorControl`。新建、打开均建立原生文档会话，不创建完整 Buffer 镜像；尚未完成主分支页面和全部行为等价验收。

```sh
cargo build --locked --release --bin nomo
cargo run --locked --bin nomo -- /path/to/document.md
```

独立开发外壳仍保留：

```sh
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control-demo
cargo run --locked --features editor-control-demo --bin nomo -- --editor-control /path/to/document.md
```

feature 仅控制开发入口、内置样本和调试界面；正式与开发入口共用窗口、事务、历史和资源代码，文档会话依赖现属于默认构建。

## 数据与生命周期

- 应用共享一个 MarkdownDocumentSessionManager；每个文档有独立 owner，避免关闭后台标签取消另一文档的查询。完整正文、编码、历史、全局修订与保存快照归会话；控件只消费局部 DocumentWindow。
- 沿用原有连续窗口、视口调度、预取、ResidentLease 字节预算和取消机制。普通新建为零字节窗口，打开按结构加载窗口，不把文件全文传给控件。
- Workspace 负责标签、窗口关闭确认、保存对话框、自动保存和保存前快照；DocumentHost 负责窗口输入、确认、导航、错误重试与挂载生命周期。
- 隐藏标签暂停窗口和非必要查询，保留选区/阅读位置；等待保存的目录检查可继续。关闭时显式关闭会话、detach 挂载控件并取消请求。异常释放走保留恢复资料的 detach。
- 搜索使用会话分页查询，每页最多 128 个结果；全局替换调用会话 prepare/commit。浮动大纲独立查询结构索引的折叠投影，页面最多缓存 8 页、合计 4 MiB，超限和不完整状态会显示提示；隐藏或切换文档时释放视图。统计调用既有后台统计服务，不阻塞大纲。
- 大纲可按标题层级折叠并导航，支持拖动浮层和章节；显隐、折叠和悬停使用短时动效，折叠过渡只暂存有限的可见标题并遵循系统减少动态效果偏好。章节移动通过文档会话复用正文片段并写入一次历史记录，无法安全识别的边界或标题会拒绝修改。界面、双栏交互和性能尚未实机验收。
- 语义/源码和双栏共用同一个会话及历史。双栏按需增加一个 Pane，各自持有有界窗口、滚动/高度树；任一时刻只开放一个输入控件。事务确认绑定提交 Entity，窗口加载核对 Pane 身份、代数与修订；新用户意图取消旧滚动跟随。离开双栏释放另一栏，隐藏标签同时暂停两栏。
- 模式切换及双栏同步使用完整文档字节锚点；单栏内容宽度使用配置，双栏按比例自适应。上述新行为尚未通过实机验收。超大单结构/native_region 的既有加载限制仍待补齐，不得改成全文加载来绕过。
- 标题栏独立 Chrome Entity 只接收活动文档状态摘要，菜单动作回到 Workspace/控件；未接通的导出、最近记录等能力继续在 Roadmap 登记，禁用项不代表功能已经完成。
- 主工具栏以本地 `origin/master` 的 `339fb93` 为外观和按钮顺序参考；额外的原生命令仍可从菜单、快捷键或设置访问。支持格式状态、折叠/恢复、紧凑宽度、公式和图表选择、表格、Emoji 及双栏对齐线。图标、滚动、焦点、动画和不同 DPI 的一致性尚需实机对照。
- 代码块与公式块保留选区方向；语义模式的普通段落会将所选可见文字转换为块正文，并拆开选区两侧的行内格式。跨窗口普通段落可分组处理格式和块转换；列表、引用及 Markdown 表格可分组处理行内格式，块转换仍会提示并保留原文。源码按 64 KiB 分片扫描；语义跨窗口选区和补丁分别受 4 MiB 预算约束，单个生成块受现有 32 KiB 语义结构预算约束。部分行内代码、HTML 等未支持的结构及超预算选区会提示并保留原文。
- 脚注和目录插入通过后台分页结构索引执行。标题修改后，目录补丁在后台合并准备，通过编辑器原有确认队列作为派生事务提交；保存先等待目录检查，再捕获该修订的快照。候选目录扫描及补丁各有 4 MiB 上限；选区位于目录内或结构不完整时保留原文，保存继续并给出提示。Emoji 目录按需加载并用虚拟列表显示；搜索面板提供全字、反向和循环选项。
- 工具栏及公式、图表、链接、表格、Emoji 和搜索浮层支持简中、繁中、英文、日文，并可读取系统语言；其余界面仍为简中，部分底层错误详情保持原语言。界面语言设置可即时切换，系统语言在进程启动后缓存。
- Windows 主窗口和设置窗口共用原生窗口按钮及拖动区域；主窗口关闭仍经过未保存确认。Markdown Mini 入口仅在有文档时显示，功能尚未接入；窗口交互和旧版视觉对照仍待实机验收。

## 当前验证入口

```sh
cargo check --locked --bin nomo
cargo check --locked --features editor-control-demo --all-targets
cargo build --locked --release --bin nomo
cargo build --locked --release --features editor-control-demo --bin nomo
cargo test --locked --release --features editor-control-demo --bin nomo editor_control
cargo test --locked --release --features editor-control-demo --bin nomo text_document::markdown_session
```

完整本轮证据、阶段状态和后续差距统一记录于 [Roadmap](../src-gpui/ROADMAP.md)。不得用构建通过替代桌面、视觉或性能验收。大型语料与远程元数据 ignored 检查未执行，不计通过。

## 隔离实机验收

设置本次进程 `NOMO_DATA_DIR`，只使用任务目录中的测试文档。参考主分支固定为 `339fb932db064508af4e471a0fda8f44468d0157`；旧安装包和旧报告不能替代该版本的运行证据。

目前尚需覆盖真实中文输入法、跨窗口复杂编辑、长文档资源约束、完整模式切换、深浅主题/缩放、主分支视觉对照和三轮性能基准。输入 Unicode 文本不等于中文输入法验收。
