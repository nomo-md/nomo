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

const WINDOWS_SHELL: Support = if cfg!(windows) { Support::Full } else { Support::None("settings.support.windowsShell") };

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
        title: "settings.category.general", icon: IconName::Settings2,
        page: Page::Groups(&[group("settings.group.basics", &[
            item("editorMode", "settings.editorMode.label", "settings.editorMode.description", Control::Choice(&[("semantic", "settings.editorMode.choice.semantic"), ("source", "settings.editorMode.choice.source"), ("split", "settings.editorMode.choice.split")]), Support::Full),
            item("splitViewLayout", "settings.splitViewLayout.label", "settings.splitViewLayout.description", Control::Choice(&[("semantic-source", "settings.splitViewLayout.choice.semantic-source"), ("source-semantic", "settings.splitViewLayout.choice.source-semantic")]), Support::Full),
            item("autoSaveEnabled", "settings.autoSaveEnabled.label", "settings.autoSaveEnabled.description", Control::Toggle, Support::Full),
            item("autoSaveDelayMs", "settings.autoSaveDelayMs.label", "settings.autoSaveDelayMs.description", Control::Range { min: 500.0, max: 5000.0, step: 100.0, unit: "ms" }, Support::Full),
            item("createSnapshotBeforeSave", "settings.createSnapshotBeforeSave.label", "settings.createSnapshotBeforeSave.description", Control::Toggle, Support::Full),
            item("interfaceLanguage", "settings.interfaceLanguage.label", "settings.interfaceLanguage.description", Control::Choice(&[("system", "settings.interfaceLanguage.choice.system"), ("zh-CN", "settings.interfaceLanguage.choice.zh-CN"), ("zh-TW", "settings.interfaceLanguage.choice.zh-TW"), ("en-US", "settings.interfaceLanguage.choice.en-US"), ("ja-JP", "settings.interfaceLanguage.choice.ja-JP")]), Support::Full),
        ])]),
    },
    Category {
        title: "settings.category.editor", icon: IconName::BookOpenText,
        page: Page::Groups(&[
            group("settings.group.typography", &[
                item("typographyEnabled", "settings.typographyEnabled.label", "settings.typographyEnabled.description", Control::Toggle, Support::Full),
            ]),
            group("settings.group.editor", &[
                item("fontSize", "settings.fontSize.label", "settings.fontSize.description", Control::Range { min: 14.0, max: 22.0, step: 1.0, unit: "px" }, Support::Full),
                item("lineHeight", "settings.lineHeight.label", "settings.lineHeight.description", Control::Range { min: 1.4, max: 2.1, step: 0.05, unit: "" }, Support::Full),
                item("contentWidthPercent", "settings.contentWidthPercent.label", "settings.contentWidthPercent.description", Control::Range { min: 45.0, max: 90.0, step: 1.0, unit: "%" }, Support::Full),
                item("largeDocumentLimit", "settings.largeDocumentLimit.label", "settings.largeDocumentLimit.description", Control::Number { unit: "settings.unit.size" }, Support::None("settings.largeDocumentLimit.support")),
                item("codeBlockIndent", "settings.codeBlockIndent.label", "settings.codeBlockIndent.description", Control::Choice(&[("spaces-2", "settings.codeBlockIndent.choice.spaces-2"), ("spaces-4", "settings.codeBlockIndent.choice.spaces-4"), ("tab", "settings.codeBlockIndent.choice.tab")]), Support::Full),
                item("codeBlockLineNumbersVisible", "settings.codeBlockLineNumbersVisible.label", "settings.codeBlockLineNumbersVisible.description", Control::Toggle, Support::Full),
                item("inlineCodeRenderingEnabled", "settings.inlineCodeRenderingEnabled.label", "settings.inlineCodeRenderingEnabled.description", Control::Toggle, Support::Full),
                item("copyMarkdownSyntaxEnabled", "settings.copyMarkdownSyntaxEnabled.label", "settings.copyMarkdownSyntaxEnabled.description", Control::Toggle, Support::Full),
            ]),
            group("settings.group.quickInsert", &[
                item("quickInsertEnabled", "settings.quickInsertEnabled.label", "settings.quickInsertEnabled.description", Control::Toggle, Support::Full),
                item("quickInsertTrigger", "settings.quickInsertTrigger.label", "settings.quickInsertTrigger.description", Control::Text { placeholder: "/", compact: true }, Support::Full),
            ]),
            Group { title: "settings.group.lint", badge: "Beta", items: &[
                item("markdownLintEnabled", "settings.markdownLintEnabled.label", "settings.markdownLintEnabled.description", Control::Toggle, Support::Full),
                item("markdownLintRuleSet", "settings.markdownLintRuleSet.label", "settings.markdownLintRuleSet.description", Control::Choice(&[("relaxed", "settings.markdownLintRuleSet.choice.relaxed"), ("default", "settings.markdownLintRuleSet.choice.default")]), Support::Full),
            ] },
        ]),
    },
    Category {
        title: "settings.category.appearance", icon: IconName::Palette,
        page: Page::Groups(&[
            group("settings.group.theme", &[
                item("themeMode", "settings.themeMode.label", "", Control::Choice(&[("system", "settings.themeMode.choice.system"), ("light", "settings.themeMode.choice.light"), ("dark", "settings.themeMode.choice.dark")]), Support::Full),
                item("colorThemeId", "settings.colorThemeId.label", "", Control::ThemePicker, Support::Full),
            ]),
            group("settings.group.fonts", &[
                item("uiFontFamily", "settings.uiFontFamily.label", "settings.uiFontFamily.description", Control::FontFamily, Support::Full),
                item("documentFontFamily", "settings.documentFontFamily.label", "settings.documentFontFamily.description", Control::FontFamily, Support::Full),
                item("monoFontFamily", "settings.monoFontFamily.label", "settings.monoFontFamily.description", Control::FontFamily, Support::Full),
            ]),
            group("settings.group.zoom", &[
                item("zoomPercent", "settings.zoomPercent.label", "settings.zoomPercent.description", Control::Range { min: 80.0, max: 160.0, step: 5.0, unit: "%" }, Support::Full),
                item("ctrlWheelZoomEnabled", "settings.ctrlWheelZoomEnabled.label", "settings.ctrlWheelZoomEnabled.description", Control::Toggle, Support::Full),
            ]),
        ]),
    },
    Category {
        title: "settings.category.files", icon: IconName::FolderOpen,
        page: Page::Groups(&[group("settings.group.files", &[
            item("openDefaultBehavior", "settings.openDefaultBehavior.label", "settings.openDefaultBehavior.description", Control::Choice(&[("ask-every-time", "settings.openDefaultBehavior.choice.ask-every-time"), ("current-window", "settings.openDefaultBehavior.choice.current-window"), ("new-window", "settings.openDefaultBehavior.choice.new-window")]), Support::Full),
            item("filePreviewEnabled", "settings.filePreviewEnabled.label", "settings.filePreviewEnabled.description", Control::Toggle, Support::Full),
            item("sidebarHidden", "settings.sidebarHidden.label", "settings.sidebarHidden.description", Control::Toggle, Support::Full),
            item("closeWindowBehavior", "settings.closeWindowBehavior.label", "settings.closeWindowBehavior.description", Control::Choice(&[("ask-every-time", "settings.closeWindowBehavior.choice.ask-every-time"), ("close-window", "settings.closeWindowBehavior.choice.close-window"), ("close-to-tray", "settings.closeWindowBehavior.choice.close-to-tray")]), Support::Partial("settings.closeWindowBehavior.support")),
            item("externalFileChangeBehavior", "settings.externalFileChangeBehavior.label", "settings.externalFileChangeBehavior.description", Control::Choice(&[("reload-external", "settings.externalFileChangeBehavior.choice.reload-external"), ("overwrite-external", "settings.externalFileChangeBehavior.choice.overwrite-external"), ("ignore", "settings.externalFileChangeBehavior.choice.ignore")]), Support::Full),
            item("markdownAssociation", "settings.markdownAssociation.label", "settings.markdownAssociation.description", Control::Action("settings.markdownAssociation.action"), WINDOWS_SHELL),
            item("contextMenuRegistration", "settings.contextMenuRegistration.label", "settings.contextMenuRegistration.description", Control::Action("settings.contextMenuRegistration.action"), WINDOWS_SHELL),
        ])]),
    },
    Category {
        title: "settings.category.images", icon: IconName::FileImage,
        page: Page::Groups(&[group("settings.group.images", &[
            item("imageHandlingSettings.imageInsertStrategy", "settings.imageHandlingSettings.imageInsertStrategy.label", "settings.imageHandlingSettings.imageInsertStrategy.description", Control::Choice(&[("copy-current-folder", "settings.imageHandlingSettings.imageInsertStrategy.choice.copy-current-folder"), ("copy-assets", "settings.imageHandlingSettings.imageInsertStrategy.choice.copy-assets"), ("copy-document-assets", "settings.imageHandlingSettings.imageInsertStrategy.choice.copy-document-assets"), ("upload", "settings.imageHandlingSettings.imageInsertStrategy.choice.upload")]), Support::Full),
            item("imageHandlingSettings.uploadProvider", "settings.imageHandlingSettings.uploadProvider.label", "settings.imageHandlingSettings.uploadProvider.description", Control::Choice(&[("picgo", "settings.imageHandlingSettings.uploadProvider.choice.picgo"), ("picgo-core", "settings.imageHandlingSettings.uploadProvider.choice.picgo-core")]), Support::Full),
            item("imageHandlingSettings.picgoServerUrl", "settings.imageHandlingSettings.picgoServerUrl.label", "settings.imageHandlingSettings.picgoServerUrl.description", Control::Text { placeholder: "", compact: false }, Support::Full),
            item("imageHandlingSettings.picgoCoreCommand", "settings.imageHandlingSettings.picgoCoreCommand.label", "settings.imageHandlingSettings.picgoCoreCommand.description", Control::Text { placeholder: "", compact: false }, Support::Full),
            item("imageHandlingSettings.picgoCoreConfigPath", "settings.imageHandlingSettings.picgoCoreConfigPath.label", "settings.imageHandlingSettings.picgoCoreConfigPath.description", Control::Text { placeholder: "settings.placeholder.empty", compact: false }, Support::Full),
            item("imageConnectionTest", "settings.imageConnectionTest.label", "settings.imageConnectionTest.description", Control::Action("settings.imageConnectionTest.action"), Support::Full),
            item("imageHandlingSettings.defaultImageWidth", "settings.imageHandlingSettings.defaultImageWidth.label", "settings.imageHandlingSettings.defaultImageWidth.description", Control::Text { placeholder: "settings.placeholder.empty", compact: true }, Support::Full),
            item("imageHandlingSettings.defaultImageAlign", "settings.imageHandlingSettings.defaultImageAlign.label", "settings.imageHandlingSettings.defaultImageAlign.description", Control::Choice(&[("none", "settings.imageHandlingSettings.defaultImageAlign.choice.none"), ("left", "settings.imageHandlingSettings.defaultImageAlign.choice.left"), ("center", "settings.imageHandlingSettings.defaultImageAlign.choice.center"), ("right", "settings.imageHandlingSettings.defaultImageAlign.choice.right")]), Support::Full),
        ])]),
    },
    Category {
        title: "settings.category.statistics", icon: IconName::ChartColumn,
        page: Page::Groups(&[group("settings.group.statistics", &[
            item("outlineVisible", "settings.outlineVisible.label", "settings.outlineVisible.description", Control::Toggle, Support::Full),
            item("writingStatsVisible", "settings.writingStatsVisible.label", "settings.writingStatsVisible.description", Control::Toggle, Support::Full),
            item("writingStatsMetric", "settings.writingStatsMetric.label", "settings.writingStatsMetric.description", Control::Choice(&[("lines", "settings.writingStatsMetric.choice.lines"), ("words", "settings.writingStatsMetric.choice.words"), ("visibleChars", "settings.writingStatsMetric.choice.visibleChars"), ("chars", "settings.writingStatsMetric.choice.chars")]), Support::Full),
            item("readingTimeVisible", "settings.readingTimeVisible.label", "settings.readingTimeVisible.description", Control::Toggle, Support::Full),
            item("outlineDefaultExpandLevel", "settings.outlineDefaultExpandLevel.label", "settings.outlineDefaultExpandLevel.description", Control::Range { min: 1.0, max: 6.0, step: 1.0, unit: "" }, Support::Full),
        ])]),
    },
    Category {
        title: "settings.category.advanced", icon: IconName::SlidersHorizontal,
        page: Page::Groups(&[
            group("settings.group.insertion", &[
                item("defaultCodeBlockLanguage", "settings.defaultCodeBlockLanguage.label", "settings.defaultCodeBlockLanguage.description", Control::Text { placeholder: "ts", compact: true }, Support::Full),
                item("defaultDiagramType", "settings.defaultDiagramType.label", "settings.defaultDiagramType.description", Control::Choice(&[("flowchart", "settings.defaultDiagramType.choice.flowchart"), ("sequenceDiagram", "settings.defaultDiagramType.choice.sequenceDiagram"), ("classDiagram", "settings.defaultDiagramType.choice.classDiagram"), ("stateDiagram", "settings.defaultDiagramType.choice.stateDiagram"), ("erDiagram", "settings.defaultDiagramType.choice.erDiagram"), ("gantt", "settings.defaultDiagramType.choice.gantt"), ("pie", "settings.defaultDiagramType.choice.pie")]), Support::Full),
            ]),
            group("settings.group.shortcuts", &[
                item("shortcutPreferences", "", "", Control::Shortcuts, Support::Full),
            ]),
            group("settings.group.developer", &[
                item("developerMode", "settings.developerMode.label", "settings.developerMode.description", Control::Toggle, Support::None("settings.developerMode.support")),
                item("exportSettings", "settings.exportSettings.label", "settings.exportSettings.description", Control::Info, Support::Full),
            ]),
        ]),
    },
    Category { title: "settings.category.about", icon: IconName::Info, page: Page::About },
];

pub const ABOUT_ITEMS: &[Item] = &[
    item("softwareUpdateAutoCheckEnabled", "settings.softwareUpdateAutoCheckEnabled.label", "settings.softwareUpdateAutoCheckEnabled.description", Control::Toggle, Support::Full),
    item("updateCheck", "settings.updateCheck.label", "settings.updateCheck.description", Control::Action("settings.updateCheck.action"), Support::Full),
];
