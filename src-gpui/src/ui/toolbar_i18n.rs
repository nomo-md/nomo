//! 工具栏及其浮层使用旧版四种界面语言；正文内容不参与翻译。
use crate::settings::Settings;
use gpui::App;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(crate) struct ToolbarLocale(&'static str);

impl ToolbarLocale {
    pub fn code(self) -> &'static str { self.0 }
    pub fn is_chinese(self) -> bool { self.0 == "zh-CN" || self.0 == "zh-TW" }
    pub fn current(cx: &App) -> Self {
        let preference = cx.global::<Settings>().string("interfaceLanguage");
        let language = match preference.as_str() {
            "zh-CN" => "zh-CN",
            "zh-TW" => "zh-TW",
            "en-US" => "en-US",
            "ja-JP" => "ja-JP",
            _ => *SYSTEM_LOCALE.get_or_init(system_locale),
        };
        Self(language)
    }

    pub fn text(self, key: &'static str) -> &'static str {
        let values = TRANSLATIONS.get_or_init(|| serde_json::from_str(include_str!("../../resources/toolbar-locales.json"))
            .expect("bundled toolbar translations"));
        values.get(self.0).and_then(|language| language.get(key)).and_then(|value| value.as_str())
            .or_else(|| values.get("zh-CN").and_then(|language| language.get(key)).and_then(|value| value.as_str()))
            .unwrap_or(key)
    }

    pub fn tooltip(self, key: &'static str, shortcut: &str) -> String {
        let suffix = shortcut.find('⌘').or_else(|| shortcut.find("Alt+")).or_else(|| shortcut.find("Ctrl+"))
            .map(|at| &shortcut[at..]);
        match suffix {
            None => self.text(key).to_owned(),
            Some(suffix) => {
                let suffix = if cfg!(target_os = "macos") { suffix.to_owned() } else {
                    suffix.replace("⌘⌥", "Ctrl+Alt+").replace("⌘⇧", "Ctrl+Shift+").replace('⌘', "Ctrl+").replace('⇧', "Shift+")
                };
                format!("{} {suffix}", self.text(key))
            }
        }
    }

    pub fn context_text(self, key: &'static str) -> &'static str {
        static MENUS: OnceLock<serde_json::Value> = OnceLock::new();
        let values = MENUS.get_or_init(|| serde_json::from_str(include_str!("../../resources/context-menu-locales.json"))
            .expect("bundled context menu translations"));
        values.get(self.0).and_then(|language| language.get(key)).and_then(|value| value.as_str())
            .or_else(|| values.get("zh-CN").and_then(|language| language.get(key)).and_then(|value| value.as_str())).unwrap_or(key)
    }

    pub fn error(self, error: &crate::nomo_mark::protocol::ControlError) -> String {
        use crate::nomo_mark::protocol::ControlError;
        match error {
            ControlError::InvalidRange => self.text("error_range").into(),
            ControlError::InvalidUtf8Boundary => self.text("error_utf8").into(),
            ControlError::Stale => self.text("error_stale").into(),
            ControlError::ReadOnly => self.text("error_readonly").into(),
            ControlError::Busy => self.text("error_busy").into(),
            ControlError::Detached => self.text("error_detached").into(),
            ControlError::NeedsContext(reason) => format!("{}: {reason}", self.text("error_context")),
            ControlError::HostFailure(reason) => format!("{}: {reason}", self.text("error_host")),
            ControlError::Unsupported(reason) => format!("{}: {reason}", self.text("error_unsupported")),
        }
    }
}

static TRANSLATIONS: OnceLock<serde_json::Value> = OnceLock::new();
static SYSTEM_LOCALE: OnceLock<&'static str> = OnceLock::new();

fn system_locale() -> &'static str {
    for locale in sys_locale::get_locales() {
        let locale = locale.trim().to_ascii_lowercase().replace('_', "-");
        if locale.starts_with("zh-hant") || locale.starts_with("zh-tw") || locale.starts_with("zh-hk") || locale.starts_with("zh-mo") { return "zh-TW"; }
        if locale == "zh" || locale.starts_with("zh-") { return "zh-CN"; }
        if locale == "ja" || locale.starts_with("ja-") { return "ja-JP"; }
        if locale == "en" || locale.starts_with("en-") { return "en-US"; }
    }
    "en-US"
}
