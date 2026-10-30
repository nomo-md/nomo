//! 与原版共用 shortcutPreferences。只在偏好提交后重建有限键表，不累积覆盖绑定。
use super::workspace as action;
use crate::settings::Settings;
use gpui::{Action, App, Global, KeyBinding, KeyBindingContextPredicate};
use serde_json::{Value, json};

pub(crate) struct Shortcut {
    pub id: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    action: fn() -> Box<dyn Action>,
}

pub(crate) const SHORTCUTS: &[Shortcut] = &[
    Shortcut { id: "new-file", label: "新建 Markdown", default: "Ctrl+N", action: || Box::new(action::NewFile) },
    Shortcut { id: "open-file", label: "打开文件", default: "Ctrl+O", action: || Box::new(action::OpenFile) },
    Shortcut { id: "export-html", label: "导出 HTML", default: "Ctrl+Shift+E", action: || Box::new(action::ExportHtml) },
    Shortcut { id: "export-pdf", label: "导出 PDF", default: "Ctrl+Shift+P", action: || Box::new(action::ExportPdf) },
    Shortcut { id: "save-file", label: "保存", default: "Ctrl+S", action: || Box::new(action::Save) },
    Shortcut { id: "toggle-source", label: "切换源码模式", default: "Ctrl+E", action: || Box::new(action::ToggleSource) },
    Shortcut { id: "toggle-theme", label: "主动切换浅 / 深色", default: "Ctrl+Shift+L", action: || Box::new(action::ToggleTheme) },
    Shortcut { id: "toggle-focus", label: "显示 / 隐藏资源管理器", default: "Ctrl+Shift+F", action: || Box::new(action::ToggleSidebar) },
    Shortcut { id: "toggle-toolbar", label: "显示 / 隐藏工具栏", default: "Ctrl+Shift+B", action: || Box::new(action::ToggleToolbar) },
    Shortcut { id: "toggle-markdown-mini", label: "打开 / 返回 Markdown 小窗", default: "Ctrl+Alt+M", action: || Box::new(action::ToggleMini) },
    Shortcut { id: "insert-code-block", label: "插入代码块", default: "Ctrl+Shift+K", action: || Box::new(action::CodeBlock) },
    Shortcut { id: "insert-table", label: "插入表格", default: "Ctrl+Shift+T", action: || Box::new(action::Table) },
    Shortcut { id: "insert-math-block", label: "插入公式块", default: "Ctrl+Shift+M", action: || Box::new(action::MathBlock) },
    Shortcut { id: "menu-link", label: "编辑链接", default: "Ctrl+K", action: || Box::new(action::Link) },
    Shortcut { id: "menu-clear-format", label: "清除样式", default: "Ctrl+\\", action: || Box::new(action::ClearFormatting) },
];

struct Bindings { values: Vec<String>, pub errors: Vec<String> }
impl Global for Bindings {}

pub(crate) fn defaults() -> Value {
    Value::Object(SHORTCUTS.iter().map(|entry| (entry.id.into(), json!(entry.default))).collect())
}

pub(crate) fn current(id: &str, cx: &App) -> String {
    let Some(index) = SHORTCUTS.iter().position(|entry| entry.id == id) else { return String::new(); };
    cx.try_global::<Bindings>().map(|bindings| bindings.values[index].clone()).unwrap_or_else(|| SHORTCUTS[index].default.into())
}

pub(crate) fn action_label(action: &dyn Action, cx: &App) -> Option<String> {
    SHORTCUTS.iter().find(|entry| (entry.action)().name() == action.name()).map(|entry| {
        let text = current(entry.id, cx);
        if cfg!(target_os = "macos") { text.replace("Ctrl+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥") } else { text }
    })
}

pub(crate) fn warnings(cx: &App) -> Option<String> {
    cx.try_global::<Bindings>().filter(|bindings| !bindings.errors.is_empty()).map(|bindings| bindings.errors.join("；"))
}

fn normalize(text: &str) -> Result<(String, String), String> {
    if text.len() > 40 { return Err("快捷键不能超过 40 字符".into()); }
    let parts: Vec<_> = text.split('+').map(str::trim).collect();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) { return Err("请输入修饰键与一个按键，例如 Ctrl+Shift+K".into()); }
    let (mut primary, mut shift, mut alt) = (false, false, false);
    for modifier in &parts[..parts.len() - 1] {
        let flag = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" | "meta" => &mut primary,
            "shift" => &mut shift, "alt" | "option" => &mut alt,
            _ => return Err("只支持 Ctrl、Shift、Alt（macOS 的 Ctrl 对应 Command）".into()),
        };
        if *flag { return Err("修饰键不能重复".into()); }
        *flag = true;
    }
    if !primary && !alt { return Err("需要 Ctrl 或 Alt，避免拦截普通输入".into()); }
    let key = parts.last().unwrap().to_ascii_lowercase();
    let valid = (key.len() == 1 && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"\\-=[]`,./;'".contains(&byte)))
        || key.strip_prefix('f').and_then(|number| number.parse::<u8>().ok()).is_some_and(|number| (1..=12).contains(&number) && key == format!("f{number}"));
    if !valid { return Err("末尾需要一个字母、数字、标点或 F1–F12".into()); }
    // 系统直接消费的组合不能伪装为应用快捷键。
    if alt && !primary && key == "f4" { return Err("Alt+F4 是系统关闭窗口快捷键".into()); }
    let mut display = String::new();
    let mut binding = String::new();
    for (enabled, label, token) in [(primary, "Ctrl+", "secondary-"), (shift, "Shift+", "shift-"), (alt, "Alt+", "alt-")] {
        if enabled { display.push_str(label); binding.push_str(token); }
    }
    display.push_str(&key.to_ascii_uppercase()); binding.push_str(&key);
    Ok((display, binding))
}

fn binding(entry: &Shortcut, value: &str, cx: &App) -> Result<KeyBinding, String> {
    let (_, keys) = normalize(value)?;
    KeyBinding::load(&keys, (entry.action)(), Some(KeyBindingContextPredicate::parse("Workspace").expect("快捷键上下文").into()), false, None, cx)
        .map_err(|error| format!("无法识别快捷键：{error}"))
}

pub(crate) fn validate(id: &str, value: &str, cx: &App) -> Result<String, String> {
    let entry = SHORTCUTS.iter().find(|entry| entry.id == id).ok_or("未知快捷键")?;
    let candidate = binding(entry, value, cx)?;
    let keymap = cx.key_bindings();
    for existing in keymap.borrow().bindings() {
        if existing.action().name() != candidate.action().name() && existing.keystrokes() == candidate.keystrokes() {
            let label = SHORTCUTS.iter().find(|entry| (entry.action)().name() == existing.action().name()).map(|entry| entry.label).unwrap_or("已有编辑或窗口命令");
            return Err(format!("与“{label}”冲突，请换一个组合"));
        }
    }
    normalize(value).map(|(display, _)| display)
}

/// 由 reload_key_bindings 在所有固定快捷键之后调用。无效旧配置仅回退，不覆盖原文件。
pub(crate) fn bind_keys(cx: &mut App) {
    let preferences = cx.global::<Settings>().value("shortcutPreferences");
    let mut errors = Vec::new();
    let mut values: Vec<String> = SHORTCUTS.iter().map(|entry| {
        let Some(value) = preferences.get(entry.id).and_then(Value::as_str) else { return entry.default.into(); };
        match validate(entry.id, value, cx) { Ok(value) => value, Err(error) => { errors.push(format!("{}：{error}，已使用默认值", entry.label)); entry.default.into() } }
    }).collect();
    // 同时恢复旧配置时允许互换；真正重复的组合回退。回退引发的新冲突最多传播 13 次。
    for _ in 0..SHORTCUTS.len() {
        let mut duplicates = Vec::new();
        for index in 0..values.len() {
            if values.iter().enumerate().any(|(other, value)| index != other && *value == values[index]) && values[index] != SHORTCUTS[index].default { duplicates.push(index); }
        }
        if duplicates.is_empty() { break; }
        for index in duplicates { values[index] = SHORTCUTS[index].default.into(); errors.push(format!("{}：组合重复，已使用默认值", SHORTCUTS[index].label)); }
    }
    let bindings: Vec<_> = SHORTCUTS.iter().zip(&values).filter_map(|(entry, value)| binding(entry, value, cx).map_err(|error| errors.push(error)).ok()).collect();
    cx.bind_keys(bindings);
    cx.set_global(Bindings { values, errors });
}
