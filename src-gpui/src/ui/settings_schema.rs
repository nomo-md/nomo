//! 设置页的声明式描述：分区、分组与设置项。
//!
//! 键名和取值保留既有配置语义。support 决定当前能力：完全未接通的项隐藏，
//! 部分接通的项显示范围说明；配置文件中的旧值仍保留。

use crate::ui::components::IconName;

pub enum Control {
    Toggle,
    /// 选项（取值, 显示文案）。三项及以下用分段控件，更多用下拉选择。
    Choice(&'static [(&'static str, &'static str)]),
    /// 数值滑杆；`step` 小于 1 时按小数保存，否则按整数保存。
    Range { min: f64, max: f64, step: f64, unit: &'static str },
    Text { placeholder: &'static str },
    /// 只读说明行（不对应可编辑的键）。
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
    pub items: &'static [Item],
}

pub enum Page {
    Groups(&'static [Group]),
    Shortcuts,
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

pub const CATEGORIES: &[Category] = &[
    Category {
        title: "常规",
        icon: IconName::Settings2,
        page: Page::Groups(&[Group {
            title: "基础行为",
            items: &[
                item("editorMode", "启动默认编辑模式", "用于下次打开文档时的默认编辑形态。", Control::Choice(&[("semantic", "语义编辑"), ("source", "源码模式"), ("split", "双栏模式")]), Support::Full),
                item("splitViewLayout", "双栏排列", "选择双栏模式中语义编辑区与 Markdown 源码的位置。", Control::Choice(&[("semantic-source", "语义在左"), ("source-semantic", "源码在左")]), Support::Full),
                item("autoSaveEnabled", "自动保存", "停止输入后自动保存已有路径的文档；未命名文档不会自动弹出保存面板。", Control::Toggle, Support::Full),
                item("autoSaveDelayMs", "自动保存延迟", "输入停止后等待多久再保存。", Control::Range { min: 500.0, max: 5000.0, step: 100.0, unit: "ms" }, Support::Full),
                item("createSnapshotBeforeSave", "保存前创建快照", "覆盖文件前把旧版本保存到本地快照目录，每个文件保留最近 20 份。", Control::Toggle, Support::Full),
                item("interfaceLanguage", "界面语言", "选择界面显示语言。", Control::Choice(&[("system", "跟随系统"), ("zh-CN", "简体中文"), ("zh-TW", "繁體中文"), ("en-US", "English"), ("ja-JP", "日本語")]), Support::Partial("工具栏支持四种语言，其余界面目前为简体中文")),
            ],
        }]),
    },
    Category {
        title: "编辑器",
        icon: IconName::BookOpenText,
        page: Page::Groups(&[
            Group {
                title: "段落排版",
                items: &[item("typographyEnabled", "精细排版", "使用 Knuth–Plass 算法两端对齐，并处理中文避头尾。", Control::Toggle, Support::Full)],
            },
            Group {
                title: "编辑器选项",
                items: &[
                    item("fontSize", "字号", "影响语义编辑区和源码模式的正文大小。", Control::Range { min: 14.0, max: 22.0, step: 1.0, unit: "px" }, Support::Full),
                    item("lineHeight", "行高", "提升长文档阅读和编辑的舒适度。", Control::Range { min: 1.4, max: 2.1, step: 0.05, unit: "" }, Support::Full),
                    item("contentWidthPercent", "内容宽度", "控制编辑正文在窗口中的最大占比。", Control::Range { min: 45.0, max: 90.0, step: 1.0, unit: "%" }, Support::Full),
                    item("codeBlockLineNumbersVisible", "代码块行号", "在语义模式的代码块左侧显示行号。", Control::Toggle, Support::Full),
                    item("codeBlockIndent", "默认缩进", "代码块编辑态按 Tab 时使用。", Control::Choice(&[("spaces-2", "2 个空格"), ("spaces-4", "4 个空格"), ("tab", "Tab")]), Support::Full),
                    item("inlineCodeRenderingEnabled", "渲染行内代码", "关闭后行内代码按源码显示，保留反引号。", Control::Toggle, Support::Full),
                    item("copyMarkdownSyntaxEnabled", "复制时保留 Markdown 语法", "关闭后从语义模式复制得到纯文本。", Control::Toggle, Support::Full),
                ],
            },
            Group {
                title: "快捷插入",
                items: &[
                    item("quickInsertEnabled", "快捷插入", "输入触发字符后弹出插入菜单。", Control::Toggle, Support::None("斜杠命令尚未实现（Roadmap P0）")),
                    item("quickInsertTrigger", "触发字符", "打开快捷插入菜单的字符。", Control::Text { placeholder: "/" }, Support::None("斜杠命令尚未实现（Roadmap P0）")),
                ],
            },
            Group {
                title: "Markdown 检查",
                items: &[
                    item("markdownLintEnabled", "Markdown 检查", "在编辑时提示格式问题。", Control::Toggle, Support::None("尚未接入检查器")),
                    item("markdownLintRuleSet", "检查规则", "宽松规则减少写作干扰；默认规则启用全部内置检查。", Control::Choice(&[("relaxed", "宽松"), ("default", "默认")]), Support::None("尚未接入检查器")),
                ],
            },
        ]),
    },
    Category {
        title: "外观",
        icon: IconName::Palette,
        page: Page::Groups(&[
            Group {
                title: "主题",
                items: &[
                    item("themeMode", "明暗模式", "跟随系统时随 macOS 外观自动切换。", Control::Choice(&[("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")]), Support::Full),
                    item("colorThemeId", "配色主题", "选择编辑器配色。", Control::Choice(&[("nomo-default", "Nomo 默认")]), Support::Partial("暂只有默认配色")),
                ],
            },
            Group {
                title: "字体",
                items: &[
                    item("uiFontFamily", "界面字体", "留空跟随系统。", Control::Text { placeholder: "跟随系统" }, Support::Full),
                    item("documentFontFamily", "正文字体", "留空使用当前平台默认正文字体。填写已安装字体的字族名。", Control::Text { placeholder: "系统默认" }, Support::Full),
                    item("monoFontFamily", "代码字体", "留空使用当前平台默认等宽字体；汉字自动使用正文字体。", Control::Text { placeholder: "系统默认" }, Support::Full),
                ],
            },
            Group {
                title: "视觉缩放",
                items: &[
                    item("zoomPercent", "缩放级别", "调整正文编辑区和源码模式的整体显示比例。", Control::Range { min: 80.0, max: 160.0, step: 5.0, unit: "%" }, Support::Full),
                    item("ctrlWheelZoomEnabled", "⌘ / Ctrl + 滚轮缩放", "按住 ⌘ 或 Ctrl 滚动滚轮调整缩放。", Control::Toggle, Support::Full),
                ],
            },
        ]),
    },
    Category {
        title: "文件",
        icon: IconName::FolderOpen,
        page: Page::Groups(&[Group {
            title: "文件与窗口",
            items: &[
                item("openDefaultBehavior", "打开默认行为", "用于菜单、最近记录和系统文件管理器；目标已打开或当前为空窗口时会直接复用。", Control::Choice(&[("ask-every-time", "每次询问"), ("current-window", "当前窗口"), ("new-window", "新窗口")]), Support::None("目前为单窗口，始终在当前窗口打开")),
                item("closeWindowBehavior", "关闭窗口行为", "设置点击关闭按钮时的主窗口行为。", Control::Choice(&[("ask-every-time", "每次询问"), ("close-window", "直接关闭"), ("close-to-tray", "最小化到托盘")]), Support::None("暂无托盘")),
                item("externalFileChangeBehavior", "文件变更默认行为", "检测到外部修改时默认执行的操作。文件被删除或移动时仍会提示。", Control::Choice(&[("reload-external", "载入外部版本"), ("overwrite-external", "保留当前编辑"), ("ignore", "忽略")]), Support::None("尚未监听外部修改")),
                item("filePreviewEnabled", "文件预览标签", "单击侧栏文件时以预览标签打开，编辑后转为常驻标签。", Control::Toggle, Support::Full),
            ],
        }]),
    },
    Category {
        title: "图片",
        icon: IconName::FileImage,
        page: Page::Groups(&[Group {
            title: "图片导入",
            items: &[item("imageHandlingSettings", "图片处理方式与上传", "粘贴、拖放和上传图片时使用的策略（复制到资源目录、PicGo 上传等），以及默认宽度与对齐。", Control::Info, Support::None("粘贴与上传图片尚未实现，暂不提供相关设置"))],
        }]),
    },
    Category {
        title: "统计与导航",
        icon: IconName::ChartColumn,
        page: Page::Groups(&[Group {
            title: "统计与导航",
            items: &[
                item("writingStatsVisible", "显示写作统计", "在状态栏显示统计数字。", Control::Toggle, Support::Full),
                item("writingStatsMetric", "默认统计类型", "决定状态栏默认展示行数、词数、字数或字符数。", Control::Choice(&[("lines", "行数"), ("words", "词数"), ("visibleChars", "字数（不含空白）"), ("chars", "字符数")]), Support::Full),
                item("readingTimeVisible", "显示阅读时间", "按每分钟约 400 字估算。", Control::Toggle, Support::Full),
                item("outlineVisible", "显示大纲", "在编辑区右上角显示文档大纲。", Control::Toggle, Support::Full),
                item("outlineDefaultExpandLevel", "大纲默认展开层级", "默认展开到指定标题层级，更深的标题可手动展开。", Control::Range { min: 1.0, max: 6.0, step: 1.0, unit: " 级" }, Support::Full),
            ],
        }]),
    },
    Category {
        title: "高级",
        icon: IconName::SlidersHorizontal,
        page: Page::Groups(&[
            Group {
                title: "默认插入行为",
                items: &[
                    item("defaultCodeBlockLanguage", "默认代码块语言", "菜单或快捷键插入代码块时使用。", Control::Text { placeholder: "ts" }, Support::Full),
                    item("defaultDiagramType", "默认图表类型", "插入 Mermaid 图表时使用的模板。", Control::Choice(&[("flowchart", "流程图"), ("sequenceDiagram", "时序图"), ("classDiagram", "类图"), ("stateDiagram", "状态图"), ("erDiagram", "ER 图"), ("gantt", "甘特图"), ("pie", "饼图")]), Support::None("图表插入与预览尚未实现")),
                ],
            },
            Group {
                title: "开发者选项",
                items: &[
                    item("developerMode", "开发者模式", "显示调试信息与诊断入口。", Control::Toggle, Support::None("暂无调试面板")),
                    item("softwareUpdateAutoCheckEnabled", "自动检查更新", "启动时检查新版本。", Control::Toggle, Support::None("原型构建不检查更新")),
                ],
            },
        ]),
    },
    Category { title: "快捷键", icon: IconName::Keyboard, page: Page::Shortcuts },
    Category { title: "关于", icon: IconName::Info, page: Page::About },
];

/// 快捷键一览（与 `editor.rs`、`workspace.rs` 中的绑定一致）。GPUI 版暂不支持自定义。
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    ("文件", &[("新建", "⌘N"), ("打开文件", "⌘O"), ("打开文件夹", "⌘⇧O"), ("保存", "⌘S"), ("另存为", "⌘⇧S"), ("关闭标签", "⌘W"), ("切换标签", "⌃Tab / ⌃⇧Tab"), ("设置", "⌘,")]),
    ("视图", &[("切换源码模式", "⌘E"), ("文档大纲", "⌘⌥O"), ("侧栏", "⌘⌥S"), ("工具栏", "⌘⇧B"), ("切换深浅色", "⌘⇧L"), ("放大 / 缩小", "Ctrl+= / Ctrl+-")]),
    ("查找", &[("查找", "⌘F"), ("查找并替换", "⌘⌥F"), ("下一个 / 上一个", "⌘G / ⌘⇧G")]),
    ("格式", &[("加粗", "⌘B"), ("斜体", "⌘I"), ("下划线", "⌘U"), ("删除线", "Alt+Shift+5"), ("分隔线", "Ctrl+Shift+H"), ("行内代码", "⌘⇧C"), ("链接", "⌘K"), ("标题 1–6 / 正文", "Ctrl+1–6 / Ctrl+0"), ("清除样式", r"Ctrl+\"), ("代码块", "⌘⇧K"), ("表格", "⌘⇧T"), ("公式块", "⌘⇧M")]),
    ("编辑", &[("撤销 / 重做", "⌘Z / ⌘⇧Z"), ("列表缩进 / 反缩进", "Tab / ⇧Tab"), ("按词移动", "⌥← / ⌥→"), ("行首 / 行尾", "⌘← / ⌘→"), ("文首 / 文末", "⌘↑ / ⌘↓"), ("表情面板", "⌃⌘Space")]),
];
