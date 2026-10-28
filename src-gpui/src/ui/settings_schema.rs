//! 偏好设置的原版分区、顺序和文案。未接通项保留位置并禁用，不写入配置。
use crate::ui::components::IconName;

pub enum Control {
    Toggle,
    /// 二、三项横排，四项按原版排成两行；更多选项使用下拉。
    Choice(&'static [(&'static str, &'static str)]),
    Range { min: f64, max: f64, step: f64, unit: &'static str },
    Text { placeholder: &'static str, compact: bool },
    FontFamily,
    Number { unit: &'static str },
    ThemePicker,
    Shortcuts,
    Action(&'static str),
    Info,
}

pub enum Support {
    Full,
    Partial(&'static str),
    None(&'static str),
}

pub struct Item {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub control: Control,
    pub support: Support,
}

pub struct Group {
    pub title: &'static str,
    pub badge: &'static str,
    pub items: &'static [Item],
}

pub enum Page {
    Groups(&'static [Group]),
    About,
}

pub struct Category {
    pub title: &'static str,
    pub icon: IconName,
    pub page: Page,
}

const fn item(key: &'static str, label: &'static str, description: &'static str, control: Control, support: Support) -> Item {
    Item { key, label, description, control, support }
}

const fn group(title: &'static str, items: &'static [Item]) -> Group {
    Group { title, badge: "", items }
}

pub const CATEGORIES: &[Category] = &[
    Category {
        title: "通用", icon: IconName::Settings2,
        page: Page::Groups(&[group("基础行为", &[
            item("editorMode", "启动默认编辑模式", "用于下次打开文档时的默认编辑形态。", Control::Choice(&[("semantic", "语义编辑"), ("source", "源码模式"), ("split", "双栏模式")]), Support::Full),
            item("splitViewLayout", "双栏排列", "选择双栏模式中语义编辑区与 Markdown 源码的位置。", Control::Choice(&[("semantic-source", "语义在左"), ("source-semantic", "源码在左")]), Support::Full),
            item("autoSaveEnabled", "自动保存", "编辑后自动写入当前本地文件。", Control::Toggle, Support::Full),
            item("autoSaveDelayMs", "自动保存延迟", "输入停止后等待多久再保存。", Control::Range { min: 500.0, max: 5000.0, step: 100.0, unit: "ms" }, Support::Full),
            item("createSnapshotBeforeSave", "保存前创建快照", "保存本地文件前记录一份 Markdown 快照，便于恢复误覆盖。", Control::Toggle, Support::Full),
            item("interfaceLanguage", "界面语言", "控制应用界面、菜单和提示文案，不会改写当前 Markdown 文档。", Control::Choice(&[("system", "跟随系统"), ("zh-CN", "简体中文"), ("zh-TW", "繁體中文"), ("en-US", "English"), ("ja-JP", "日本語")]), Support::Partial("工具栏支持四种语言，其余界面目前为简体中文")),
        ])]),
    },
    Category {
        title: "编辑器", icon: IconName::BookOpenText,
        page: Page::Groups(&[
            group("段落排版", &[
                item("typographyEnabled", "精细排版", "采用 Knuth–Plass（KP）算法优化段落断行。", Control::Toggle, Support::Full),
            ]),
            group("编辑器选项", &[
                item("fontSize", "字号", "影响语义编辑区和源码模式的正文大小。", Control::Range { min: 14.0, max: 22.0, step: 1.0, unit: "px" }, Support::Full),
                item("lineHeight", "行高", "提升长文档阅读和编辑的舒适度。", Control::Range { min: 1.4, max: 2.1, step: 0.05, unit: "" }, Support::Full),
                item("contentWidthPercent", "内容宽度", "控制编辑正文在窗口中的最大占比。", Control::Range { min: 45.0, max: 90.0, step: 1.0, unit: "%" }, Support::Full),
                item("documentStyleId", "Callout 样式", "控制提示块和引用块的视觉密度。", Control::Choice(&[("nomo-classic", "经典"), ("nomo-modern", "现代")]), Support::None("原版文档样式切换尚未接入")),
                item("largeDocumentLimit", "大文件阈值", "超过阈值后使用只读源码模式，避免语义解析阻塞窗口。", Control::Number { unit: "字符 / 字节" }, Support::None("当前编辑器按窗口加载文档，不使用旧版阈值切换策略")),
                item("codeBlockIndent", "默认缩进", "代码块编辑态按 Tab 时使用。", Control::Choice(&[("spaces-2", "2 空格"), ("spaces-4", "4 空格"), ("tab", "Tab")]), Support::Full),
                item("codeBlockLineNumbersVisible", "代码块行号", "控制语义编辑区代码块是否显示行号。", Control::Toggle, Support::Full),
                item("inlineCodeRenderingEnabled", "渲染行内代码", "开启后显示行内代码样式；关闭后显示原始 Markdown 反引号文本。", Control::Toggle, Support::Full),
                item("copyMarkdownSyntaxEnabled", "复制时保留 Markdown 语法", "开启后，在语义编辑模式中复制或剪切选区时，纯文本剪贴板使用语义等价的 Markdown；富文本格式保持不变。", Control::Toggle, Support::Full),
            ]),
            group("快捷插入", &[
                item("quickInsertEnabled", "启用快捷插入", "在行首或空格后输入触发符号，选择要插入的内容。↑↓ 选类别，←→ 切换样式。", Control::Toggle, Support::Full),
                item("quickInsertTrigger", "触发符号", "默认 /，可以改为一个标点或符号。", Control::Text { placeholder: "/", compact: true }, Support::Full),
            ]),
            Group { title: "Markdown 格式检查", badge: "Beta", items: &[
                item("markdownLintEnabled", "检查 Markdown 格式", "仅报告格式问题，不会修改文档内容。", Control::Toggle, Support::None("Markdown 格式检查器尚未接入")),
                item("markdownLintRuleSet", "检查规则", "宽松规则减少写作干扰；默认规则启用全部内置检查。", Control::Choice(&[("relaxed", "宽松规则"), ("default", "默认规则")]), Support::None("Markdown 格式检查器尚未接入")),
            ] },
        ]),
    },
    Category {
        title: "外观", icon: IconName::Palette,
        page: Page::Groups(&[
            group("主题", &[
                item("themeMode", "明暗模式", "", Control::Choice(&[("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")]), Support::Full),
                item("colorThemeId", "配色主题", "", Control::ThemePicker, Support::Partial("当前原生版仅启用 Nomo 默认配色，其他主题保留原版预览")),
            ]),
            group("字体", &[
                item("uiFontFamily", "界面字体", "用于菜单、侧栏、工具栏和设置窗口。", Control::FontFamily, Support::Full),
                item("documentFontFamily", "正文字体", "用于编辑区正文，以及导出的 HTML 和 PDF。", Control::FontFamily, Support::Full),
                item("monoFontFamily", "代码字体", "用于代码块、行内代码和源码模式。", Control::FontFamily, Support::Full),
            ]),
            group("视觉缩放", &[
                item("zoomPercent", "缩放级别", "调整正文编辑区和源码模式的整体显示比例。", Control::Range { min: 80.0, max: 160.0, step: 5.0, unit: "%" }, Support::Full),
                item("ctrlWheelZoomEnabled", "Ctrl 滚轮缩放", "按住 Ctrl 并滚动鼠标滚轮调整缩放。", Control::Toggle, Support::Full),
            ]),
        ]),
    },
    Category {
        title: "文件与窗口", icon: IconName::FolderOpen,
        page: Page::Groups(&[group("文件与窗口", &[
            item("openDefaultBehavior", "打开默认行为", "用于菜单、最近记录和系统文件管理器；目标已打开或当前为空窗口时会直接复用。", Control::Choice(&[("ask-every-time", "每次询问"), ("current-window", "当前窗口"), ("new-window", "新窗口")]), Support::Full),
            item("filePreviewEnabled", "文件预览标签", "单击新文件显示为预览标签；打开另一文件、编辑或双击后自动固定。", Control::Toggle, Support::Full),
            item("sidebarHidden", "启动时隐藏资源管理器侧边栏", "下次打开后保持侧边栏收起。", Control::Toggle, Support::Full),
            item("closeWindowBehavior", "关闭窗口行为", "设置点击关闭按钮时的主窗口行为。", Control::Choice(&[("ask-every-time", "每次选择"), ("close-window", "关闭窗口"), ("close-to-tray", "关闭到托盘")]), Support::Partial("关闭到托盘仅在 Windows 可用")),
            item("externalFileChangeBehavior", "文件变更默认行为", "检测到外部修改时默认执行的操作。文件被删除或移动时仍会提示。", Control::Choice(&[("reload-external", "重新载入"), ("overwrite-external", "覆盖外部"), ("ignore", "忽略")]), Support::None("当前使用外部变更确认弹窗；自动处理偏好尚未接入")),
            item("markdownAssociation", "绑定 .md 默认打开方式", "将 Nomo 注册到 Windows 默认应用候选列表，并在系统设置中完成确认。", Control::Action("绑定 .md"), Support::None("当前构建尚未接入系统文件关联设置")),
            item("contextMenuRegistration", "注册 .md 与文件夹右键菜单", "在 .md / .markdown 文件和文件夹右键菜单中加入 Nomo。", Control::Action("注册右键菜单"), Support::None("当前构建尚未接入系统右键菜单注册")),
        ])]),
    },
    Category {
        title: "图片", icon: IconName::FileImage,
        page: Page::Groups(&[group("图片导入", &[
            item("imageHandlingSettings.imageInsertStrategy", "图片处理方式", "粘贴、拖放和上传图片时使用的默认策略。", Control::Choice(&[("copy-current-folder", "当前文件夹"), ("copy-assets", "assets 文件夹"), ("copy-document-assets", "文档.assets"), ("upload", "上传")]), Support::Partial("仅支持本地复制；旧上传配置保留，需切换本地策略后导入")),
            item("imageHandlingSettings.autoDeleteUnusedLocalImages", "自动清理本地图片", "图片引用完全移除后，同步删除对应本地文件。", Control::Toggle, Support::None("自动清理图片尚未接入，不会删除任何图片")),
            item("imageHandlingSettings.uploadProvider", "上传方式", "PicGo 适合常驻服务，PicGo-Core 适合命令行工作流。", Control::Choice(&[("picgo", "PicGo"), ("picgo-core", "PicGo-Core")]), Support::None("图片上传尚未接入")),
            item("imageHandlingSettings.picgoServerUrl", "PicGo Server 地址", "用于调用本机 PicGo 服务上传图片。", Control::Text { placeholder: "", compact: false }, Support::None("图片上传尚未接入")),
            item("imageHandlingSettings.picgoCoreCommand", "PicGo-Core 命令", "例如 picgo、npx picgo 或完整可执行文件路径。", Control::Text { placeholder: "", compact: false }, Support::None("图片上传尚未接入，不会执行此命令")),
            item("imageHandlingSettings.picgoCoreConfigPath", "PicGo-Core 配置文件路径", "可留空，使用 PicGo-Core 默认配置。", Control::Text { placeholder: "留空", compact: false }, Support::None("图片上传尚未接入")),
            item("imageConnectionTest", "连接测试", "检查当前 PicGo 配置能否被 Nomo 调用。", Control::Action("测试连接"), Support::None("图片上传尚未接入，不会发起网络请求")),
            item("imageHandlingSettings.defaultImageWidth", "图片默认宽度", "插入图片时自动写入宽度属性，可填 640px、80% 或留空。", Control::Text { placeholder: "留空", compact: true }, Support::Full),
            item("imageHandlingSettings.defaultImageAlign", "图片默认对齐", "插入图片时自动写入对齐属性。", Control::Choice(&[("none", "跟随正文"), ("left", "左对齐"), ("center", "居中"), ("right", "右对齐")]), Support::Full),
        ])]),
    },
    Category {
        title: "统计与大纲", icon: IconName::ChartColumn,
        page: Page::Groups(&[group("统计与导航", &[
            item("outlineVisible", "显示文档大纲", "在编辑区右侧显示当前文档标题导航。", Control::Toggle, Support::Full),
            item("writingStatsVisible", "显示文档统计", "在正文右下角显示当前文档的轻量统计。", Control::Toggle, Support::Full),
            item("writingStatsMetric", "默认统计类型", "决定状态栏默认展示行数、词数、字数或字符数。", Control::Choice(&[("lines", "行数"), ("words", "词数"), ("visibleChars", "字数"), ("chars", "字符")]), Support::Full),
            item("readingTimeVisible", "阅读时间", "在统计弹层中显示由当前 Markdown 派生的阅读时长。", Control::Toggle, Support::Full),
            item("outlineDefaultExpandLevel", "大纲默认展开层级", "打开或切换文档时默认展示到指定标题层级。", Control::Range { min: 1.0, max: 6.0, step: 1.0, unit: "" }, Support::Full),
        ])]),
    },
    Category {
        title: "高级", icon: IconName::SlidersHorizontal,
        page: Page::Groups(&[
            group("渲染", &[
                item("renderMode", "渲染模式", "选择使用硬件 GPU 加速或软件渲染。", Control::Choice(&[("hardware", "硬件"), ("software", "软件")]), Support::None("当前原生渲染后端尚未提供此切换")),
            ]),
            group("默认插入行为", &[
                item("defaultCodeBlockLanguage", "代码块默认语言", "菜单或快捷键插入代码块时使用。", Control::Text { placeholder: "ts", compact: true }, Support::Full),
                item("defaultDiagramType", "Mermaid 默认图表类型", "菜单插入图表时使用。", Control::Choice(&[("flowchart", "流程图"), ("sequenceDiagram", "时序图"), ("classDiagram", "类图"), ("stateDiagram", "状态图"), ("erDiagram", "ER 图"), ("gantt", "甘特图"), ("pie", "饼图")]), Support::None("图表插入已接通，默认模板偏好尚未接入")),
            ]),
            group("自定义快捷键", &[
                item("shortcutPreferences", "", "", Control::Shortcuts, Support::None("当前显示实际绑定，尚不支持修改快捷键")),
            ]),
            group("开发者选项", &[
                item("developerMode", "开发者模式", "开启后输出运行日志，便于排查问题。", Control::Toggle, Support::None("开发者模式偏好尚未接入")),
                item("exportSettings", "导出设置", "完整导出管线不在当前阶段。", Control::Info, Support::None("后续版本支持")),
            ]),
        ]),
    },
    Category { title: "关于", icon: IconName::Info, page: Page::About },
];

pub const ABOUT_ITEMS: &[Item] = &[
    item("softwareUpdateAutoCheckEnabled", "启动时自动检查更新", "每次冷启动后在后台检查一次，检查失败时不会打扰编辑。", Control::Toggle, Support::Full),
    item("updateCheck", "更新检查", "检查 GitHub Release 中的 Nomo 稳定版更新。", Control::Action("检查更新"), Support::Full),
];

/// 顺序和文案沿用原版高级页；未接通的快捷键只读展示，不注册绑定。
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("新建 Markdown", "N"),
    ("打开文件", "O"),
    ("保存", "S"),
    ("切换源码模式", "E"),
    ("主动切换浅 / 深色", "Shift+L"),
    ("显示 / 隐藏资源管理器", "Shift+F"),
    ("显示 / 隐藏工具栏", "Shift+B"),
    ("打开 / 返回 Markdown 小窗", "Alt+M"),
    ("插入代码块", "Shift+K"),
    ("插入表格", "Shift+T"),
    ("插入公式块", "Shift+M"),
    ("编辑链接", "K"),
    ("清除样式", "\\"),
];
