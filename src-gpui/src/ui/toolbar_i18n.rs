//! 应用界面共用四种语言；静态词典按需加载，正文、文件名和路径不参与翻译。
use crate::settings::Settings;
use gpui::App;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolbarLocale(&'static str);

impl ToolbarLocale {
    pub fn code(self) -> &'static str { self.0 }
    // 仅用于原生回调的原子状态，不写入配置文件。
    pub fn id(self) -> u8 { match self.0 { "zh-TW" => 1, "en-US" => 2, "ja-JP" => 3, _ => 0 } }
    pub fn from_id(id: u8) -> Self { Self(match id { 1 => "zh-TW", 2 => "en-US", 3 => "ja-JP", _ => "zh-CN" }) }
    pub fn fallback() -> Self { Self("zh-CN") }
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

    pub fn app_text(self, key: &'static str) -> &'static str {
        static APPLICATION: OnceLock<serde_json::Value> = OnceLock::new();
        let values = APPLICATION.get_or_init(|| serde_json::from_str(include_str!("../../resources/app-locales.json"))
            .expect("bundled application translations"));
        values.get(self.0).and_then(|language| language.get(key)).and_then(|value| value.as_str())
            .or_else(|| values.get("zh-CN").and_then(|language| language.get(key)).and_then(|value| value.as_str()))
            .unwrap_or(key)
    }

    /// 单次扫描模板；参数中的花括号、路径和用户文本不会再次作为占位符解释。
    pub fn app_format(self, key: &'static str, arguments: &[(&str, &str)]) -> String {
        Self::format_template(self.app_text(key), arguments)
    }

    fn format_template(template: &str, arguments: &[(&str, &str)]) -> String {
        let mut rest = template;
        let mut output = String::with_capacity(rest.len());
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            rest = &rest[start..];
            let Some(end) = rest.find('}') else { break; };
            let name = &rest[1..end];
            if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) { output.push_str(value); }
            else { output.push_str(&rest[..=end]); }
            rest = &rest[end + 1..];
        }
        output.push_str(rest);
        output
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

/// 后台结果和已缓存提示只保存稳定键与参数，显示时才读取语言；不持有 App、视图或文档。
#[derive(Clone, Debug, Hash)]
pub(crate) struct UiMessage {
    key: &'static str,
    toolbar: bool,
    arguments: Vec<(&'static str, String)>,
    text_arguments: Vec<(&'static str, &'static str)>,
    source: Option<Box<UiMessage>>,
}

impl UiMessage {
    pub fn new(key: &'static str) -> Self { Self { key, toolbar: false, arguments: Vec::new(), text_arguments: Vec::new(), source: None } }
    pub fn toolbar(key: &'static str) -> Self { Self { toolbar: true, ..Self::new(key) } }
    pub fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.arguments.push((name, value.to_string())); self
    }
    pub fn raw(value: impl ToString) -> Self { Self::new("app.raw").arg("value", value) }
    pub fn text_arg(mut self, name: &'static str, key: &'static str) -> Self {
        self.text_arguments.push((name, key)); self
    }
    pub fn context(self, key: &'static str) -> Self {
        Self { source: Some(Box::new(self)), ..Self::new(key) }
    }
    pub fn render(&self, locale: ToolbarLocale) -> String {
        let source = self.source.as_ref().map(|source| source.render(locale));
        let mut arguments: Vec<_> = self.arguments.iter().map(|(key, value)| (*key, value.as_str())).collect();
        arguments.extend(self.text_arguments.iter().map(|(name, key)| (*name, locale.app_text(key))));
        if let Some(source) = &source { arguments.push(("error", source.as_str())); }
        let template = if self.toolbar { locale.text(self.key) } else { locale.app_text(self.key) };
        ToolbarLocale::format_template(template, &arguments)
    }
}

// 保留尚未接入界面的 String 错误入口和日志语义；界面应显式使用当前语言 render。
impl std::fmt::Display for UiMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.render(ToolbarLocale::fallback())) }
}
impl std::error::Error for UiMessage {}
impl From<String> for UiMessage { fn from(value: String) -> Self { Self::raw(value) } }
impl From<&str> for UiMessage { fn from(value: &str) -> Self { Self::raw(value) } }
impl From<UiMessage> for String { fn from(value: UiMessage) -> Self { value.to_string() } }

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
