//! 应用设置（GPUI 全局状态）与持久化。
//!
//! 设置与 Web 桌面版、MyGo 原生版共用同一份配置文件
//! `~/Library/Application Support/com.nomo.desktop/config.json` 的 `app.settings` 段：
//! 每项为 `{ key, value_json, updated_at }`，键名与取值格式与 Web 版 `AppPreferences` 一致，
//! 因此在任一版本里修改的偏好对其他版本同样可见。
//!
//! 写入只更新被修改的键：每次都重新读取文件再合并，保留其他版本写入的未知字段，
//! 并先写临时文件再原子替换，避免与 Web 版并发写入时损坏文件。

use crate::layout::{self, Wrap};
use crate::theme::{self, Metrics, Theme};
use gpui::{App, Global};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditorMode {
    Semantic,
    Source,
    Split,
}

/// 主题模式：默认跟随系统外观（与 Web 版 `themeMode: 'system'` 一致）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatsMetric {
    Lines,
    Words,
    VisibleChars,
    Chars,
}

/// Web 版 `DEFAULT_APP_PREFERENCES` 中 GPUI 版会读写的键及其默认值。
pub fn default_value(key: &str) -> Value {
    match key {
        "themeMode" => json!("system"),
        "colorThemeId" => json!("nomo-default"),
        "uiFontFamily" | "documentFontFamily" | "monoFontFamily" => json!(""),
        "interfaceLanguage" => json!("system"),
        "editorMode" => json!("semantic"),
        "splitViewLayout" => json!("semantic-source"),
        "autoSaveEnabled" => json!(false),
        "autoSaveDelayMs" => json!(1000),
        "createSnapshotBeforeSave" => json!(true),
        "fontSize" => json!(16),
        "lineHeight" => json!(1.75),
        "typographyEnabled" => json!(false),
        "contentWidthPercent" => json!(60),
        "openDefaultBehavior" => json!("ask-every-time"),
        "filePreviewEnabled" => json!(true),
        "closeWindowBehavior" => json!("ask-every-time"),
        "externalFileChangeBehavior" => json!("reload-external"),
        "sidebarHidden" => json!(false),
        "toolbarHidden" => json!(false),
        "outlineVisible" => json!(true),
        "writingStatsVisible" => json!(true),
        "writingStatsMetric" => json!("words"),
        "readingTimeVisible" => json!(false),
        "defaultCodeBlockLanguage" => json!("ts"),
        "defaultDiagramType" => json!("flowchart"),
        "zoomPercent" => json!(100),
        "ctrlWheelZoomEnabled" => json!(true),
        "outlineDefaultExpandLevel" => json!(2),
        "codeBlockLineNumbersVisible" => json!(true),
        "codeBlockIndent" => json!("spaces-2"),
        "inlineCodeRenderingEnabled" => json!(true),
        "copyMarkdownSyntaxEnabled" => json!(true),
        "quickInsertEnabled" => json!(true),
        "quickInsertTrigger" => json!("/"),
        "markdownLintEnabled" => json!(false),
        "markdownLintRuleSet" => json!("relaxed"),
        "developerMode" => json!(false),
        "softwareUpdateAutoCheckEnabled" => json!(true),
        _ => Value::Null,
    }
}

#[derive(Clone)]
pub struct Settings {
    /// 原始取值（键为 Web 版偏好名）；只含从文件读到或在本版本修改过的键，缺省时取 `default_value`。
    values: BTreeMap<String, Value>,
    // 以下为由 `values` 派生的类型化字段，在每次修改后统一刷新，热路径直接读取。
    pub theme_mode: ThemeMode,
    pub zoom: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub typography: bool,
    /// 内容宽度占比，0.45–0.9。
    pub content_width_percent: f32,
    pub default_mode: EditorMode,
    /// 分栏时源码在左（`source-semantic`）。
    pub split_source_first: bool,
    pub show_outline: bool,
    pub show_sidebar: bool,
    pub show_toolbar: bool,
    pub autosave: Option<Duration>,
    pub snapshot_before_save: bool,
    pub ctrl_wheel_zoom: bool,
    pub stats_visible: bool,
    pub stats_metric: StatsMetric,
    pub reading_time: bool,
    pub outline_level: u8,
    pub code_line_numbers: bool,
    /// 代码块中 Tab 插入的缩进文本。
    pub code_indent: &'static str,
    pub inline_code_rendering: bool,
    pub copy_markdown_syntax: bool,
    pub default_code_language: String,
}

impl Global for Settings {}

impl Default for Settings {
    fn default() -> Self {
        let mut settings = Self {
            values: BTreeMap::new(),
            theme_mode: ThemeMode::System,
            zoom: 1.0,
            font_size: 16.0,
            line_height: 1.75,
            typography: false,
            content_width_percent: 0.6,
            default_mode: EditorMode::Semantic,
            split_source_first: false,
            show_outline: true,
            show_sidebar: true,
            show_toolbar: true,
            autosave: None,
            snapshot_before_save: true,
            ctrl_wheel_zoom: true,
            stats_visible: true,
            stats_metric: StatsMetric::Words,
            reading_time: false,
            outline_level: 2,
            code_line_numbers: true,
            code_indent: "  ",
            inline_code_rendering: true,
            copy_markdown_syntax: true,
            default_code_language: "ts".into(),
        };
        settings.derive();
        settings
    }
}

impl Settings {
    /// 从共享配置文件加载；文件不存在或损坏时使用默认值（不覆盖损坏的文件）。
    pub fn load() -> Self {
        let mut settings = Self::default();
        if let Some(Value::Object(stored)) = read_config().and_then(|config| config.pointer("/app/settings").cloned()) {
            for (key, record) in stored {
                if default_value(&key).is_null() {
                    continue;
                }
                if let Some(value) = record.get("value_json").and_then(Value::as_str).and_then(|raw| serde_json::from_str(raw).ok()) {
                    settings.values.insert(key, value);
                }
            }
        }
        settings.derive();
        settings
    }

    pub fn value(&self, key: &str) -> Value {
        self.values.get(key).cloned().unwrap_or_else(|| default_value(key))
    }

    pub fn bool(&self, key: &str) -> bool {
        self.value(key).as_bool().unwrap_or(false)
    }

    pub fn number(&self, key: &str) -> f64 {
        self.value(key).as_f64().or_else(|| default_value(key).as_f64()).unwrap_or(0.0)
    }

    pub fn string(&self, key: &str) -> String {
        self.value(key).as_str().map(str::to_string).unwrap_or_default()
    }

    pub fn metrics(&self) -> Metrics {
        Metrics { zoom: self.zoom, font_size: self.font_size, line_height: self.line_height }
    }

    pub fn wrap(&self) -> Wrap {
        if self.typography { Wrap::KnuthPlass } else { Wrap::Greedy }
    }

    /// 修改一项设置：更新内存、刷新派生字段、写回配置文件并重绘所有窗口。
    pub fn set(cx: &mut App, key: &str, value: Value) {
        Self::set_with(cx, key, value, true);
    }

    /// `persist` 为假时只更新内存（滑杆拖动过程中），拖动结束再以 `persist = true` 写入文件。
    pub fn set_with(cx: &mut App, key: &str, value: Value, persist: bool) {
        let settings = cx.global_mut::<Settings>();
        let changed = settings.value(key) != value;
        if !changed && !persist {
            return;
        }
        settings.values.insert(key.to_string(), value.clone());
        settings.derive();
        if persist {
            if let Err(error) = write_setting(key, &value) {
                eprintln!("保存设置 {key} 失败：{error}");
            }
        }
        if !changed {
            return;
        }
        // 影响排版结果的设置需要让文本与块布局缓存失效。
        if matches!(key, "documentFontFamily" | "monoFontFamily" | "codeBlockLineNumbersVisible" | "inlineCodeRenderingEnabled") {
            layout::bump_generation();
        }
        if key == "themeMode" {
            let appearance = cx.windows().first().and_then(|window| window.update(cx, |_, window, _| window.appearance()).ok());
            if let Some(appearance) = appearance {
                Theme::apply(appearance, cx);
            }
        }
        cx.refresh_windows();
    }

    pub fn toggle(cx: &mut App, key: &str) {
        let current = cx.global::<Settings>().bool(key);
        Settings::set(cx, key, json!(!current));
    }

    fn derive(&mut self) {
        self.theme_mode = match self.string("themeMode").as_str() {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        };
        self.zoom = (self.number("zoomPercent") as f32 / 100.0).clamp(0.6, 2.0);
        self.font_size = (self.number("fontSize") as f32).clamp(12.0, 28.0);
        self.line_height = (self.number("lineHeight") as f32).clamp(1.2, 2.4);
        self.typography = self.bool("typographyEnabled");
        self.content_width_percent = (self.number("contentWidthPercent") as f32 / 100.0).clamp(0.3, 1.0);
        self.default_mode = match self.string("editorMode").as_str() {
            "source" => EditorMode::Source,
            "split" => EditorMode::Split,
            _ => EditorMode::Semantic,
        };
        self.split_source_first = self.string("splitViewLayout") == "source-semantic";
        self.show_outline = self.bool("outlineVisible");
        self.show_sidebar = !self.bool("sidebarHidden");
        self.show_toolbar = !self.bool("toolbarHidden");
        self.autosave = self.bool("autoSaveEnabled").then(|| Duration::from_millis(self.number("autoSaveDelayMs").clamp(300.0, 60_000.0) as u64));
        self.snapshot_before_save = self.bool("createSnapshotBeforeSave");
        self.ctrl_wheel_zoom = self.bool("ctrlWheelZoomEnabled");
        self.stats_visible = self.bool("writingStatsVisible");
        self.stats_metric = match self.string("writingStatsMetric").as_str() {
            "lines" => StatsMetric::Lines,
            "visibleChars" => StatsMetric::VisibleChars,
            "chars" => StatsMetric::Chars,
            _ => StatsMetric::Words,
        };
        self.reading_time = self.bool("readingTimeVisible");
        self.outline_level = (self.number("outlineDefaultExpandLevel") as u8).clamp(1, 6);
        self.code_line_numbers = self.bool("codeBlockLineNumbersVisible");
        self.code_indent = match self.string("codeBlockIndent").as_str() {
            "spaces-4" => "    ",
            "tab" => "\t",
            _ => "  ",
        };
        self.inline_code_rendering = self.bool("inlineCodeRenderingEnabled");
        self.copy_markdown_syntax = self.bool("copyMarkdownSyntaxEnabled");
        self.default_code_language = self.string("defaultCodeBlockLanguage");
        theme::set_font_overrides(&self.string("documentFontFamily"), &self.string("monoFontFamily"));
    }
}

/// 共享配置文件路径（与 `src-tauri/src/config/mod.rs` 的 `resolve_app_data_dir` 一致）。
pub fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/com.nomo.desktop/config.json"))
}

/// 本地快照目录：保存前的旧版本按文件名存放于此。
pub fn snapshot_dir() -> Option<PathBuf> {
    config_path().and_then(|path| path.parent().map(|dir| dir.join("gpui-snapshots")))
}

fn read_config() -> Option<Value> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_setting(key: &str, value: &Value) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| std::io::Error::other("无法确定配置目录"))?;
    let mut config = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).map_err(|error| std::io::Error::other(format!("配置文件格式错误，未写入：{error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error),
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
    let record = json!({ "key": key, "value_json": value.to_string(), "updated_at": now });
    let root = config.as_object_mut().ok_or_else(|| std::io::Error::other("配置根节点不是对象"))?;
    let app = root.entry("app").or_insert_with(|| json!({ "settings": {} }));
    let settings = app
        .as_object_mut()
        .and_then(|app| Some(app.entry("settings").or_insert_with(|| json!({}))))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| std::io::Error::other("配置 app.settings 不是对象"))?;
    settings.insert(key.to_string(), record);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.gpui-tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&config).map_err(std::io::Error::other)?)?;
    std::fs::rename(&temporary, &path)
}
