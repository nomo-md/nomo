//! 有界命令表；旧字符串保留，平台覆盖值独立保存。仅在提交偏好时重建键表。
use super::{workspace as action, toolbar_i18n::{ToolbarLocale, UiMessage}};
use crate::{editor, nomo_mark::view as mark, settings::Settings};
use gpui::{Action, App, Global, KeyBinding, KeyBindingContextPredicate, KeyContext, Keystroke};
use serde_json::{Value, json};

const MACOS: bool = cfg!(target_os = "macos");
pub(super) const PLATFORM_KEY: &str = if MACOS { "__macos" } else if cfg!(windows) { "__windows" } else { "__linux" };
pub(super) const MAX_BINDINGS: usize = 3;

#[derive(Clone, Copy)]
pub(crate) enum Scope { Application, Workspace, Document, Editor, History }
impl Scope {
    fn predicate(self) -> &'static str {
        match self {
            Self::Application => "!ShortcutRecorder",
            Self::Workspace => "Workspace && !ShortcutRecorder",
            Self::Document => "(Workspace || NomoMark) && !TextField && !ShortcutRecorder",
            Self::Editor => "NomoMark && !TextField && !ShortcutRecorder",
            Self::History => "(NomoMark || Outline) && !TextField && !ShortcutRecorder",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Application => "shortcuts.scope.application",
            Self::Workspace => "shortcuts.scope.workspace",
            Self::Document => "shortcuts.scope.document",
            Self::Editor => "shortcuts.scope.editor",
            Self::History => "shortcuts.scope.history",
        }
    }
}

pub(crate) struct Shortcut {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub scope: Scope,
    windows: &'static [&'static str],
    macos: &'static [&'static str],
    action: fn() -> Box<dyn Action>,
}
macro_rules! shortcut {
    ($id:literal, $label:literal, $group:literal, $scope:ident, $action:path, $win:expr, $mac:expr) => {
        Shortcut { id: $id, label: $label, group: $group, scope: Scope::$scope, windows: $win, macos: $mac, action: || Box::new($action) }
    };
}
pub(crate) const SHORTCUTS: &[Shortcut] = &[
    shortcut!("new-file", "shortcuts.newFile", "files", Workspace, action::NewFile, &["Ctrl+N"], &["Command+N"]),
    shortcut!("new-window", "settings.openDefaultBehavior.choice.new-window", "files", Workspace, action::NewWindow, &["Ctrl+Shift+N"], &["Command+Shift+N"]),
    shortcut!("open-file", "shortcuts.openFile", "files", Workspace, action::OpenFile, &["Ctrl+O"], &["Command+O"]),
    shortcut!("open-folder", "menu.openFolder", "files", Workspace, action::OpenFolder, &["Ctrl+Shift+O"], &["Command+Shift+O"]),
    shortcut!("save-file", "shortcuts.save", "files", Workspace, action::Save, &["Ctrl+S"], &["Command+S"]),
    shortcut!("save-as", "menu.saveAs", "files", Workspace, action::SaveAs, &["Ctrl+Shift+S"], &["Command+Shift+S"]),
    shortcut!("close-tab", "menu.closeTab", "files", Workspace, action::CloseTab, &["Ctrl+W"], &["Command+W"]),
    shortcut!("next-tab", "shortcuts.nextTab", "files", Workspace, action::NextTab, &["Ctrl+Tab"], &["Ctrl+Tab"]),
    shortcut!("previous-tab", "shortcuts.previousTab", "files", Workspace, action::PrevTab, &["Ctrl+Shift+Tab"], &["Ctrl+Shift+Tab"]),
    shortcut!("export-html", "shortcuts.exportHtml", "files", Workspace, action::ExportHtml, &["Ctrl+Shift+E"], &["Command+Shift+E"]),
    shortcut!("export-pdf", "shortcuts.exportPdf", "files", Workspace, action::ExportPdf, &["Ctrl+Shift+P"], &["Command+Shift+P"]),
    shortcut!("export-word", "menu.exportWord", "files", Workspace, action::ExportWord, &[], &[]),
    shortcut!("print", "menu.print", "files", Workspace, action::PrintDocument, &["Ctrl+P"], &["Command+P"]),
    shortcut!("undo", "menu.undo", "editing", History, editor::Undo, &["Ctrl+Z"], &["Command+Z"]),
    shortcut!("redo", "menu.redo", "editing", History, editor::Redo, &["Ctrl+Y", "Ctrl+Shift+Z"], &["Command+Shift+Z"]),
    shortcut!("cut", "menu.cut", "editing", Editor, editor::Cut, &["Ctrl+X"], &["Command+X"]),
    shortcut!("copy", "menu.copy", "editing", Editor, editor::Copy, &["Ctrl+C"], &["Command+C"]),
    shortcut!("paste", "menu.paste", "editing", Editor, editor::Paste, &["Ctrl+V"], &["Command+V"]),
    shortcut!("paste-plain", "shortcuts.pastePlain", "editing", Editor, mark::PastePlain, &["Ctrl+Shift+V"], &["Command+Shift+V", "Command+Alt+Shift+V"]),
    shortcut!("select-all", "menu.selectAll", "editing", Editor, editor::SelectAll, &["Ctrl+A"], &["Command+A"]),
    shortcut!("find", "menu.find", "search", Workspace, action::ToggleFind, &["Ctrl+F"], &["Command+F"]),
    shortcut!("replace", "menu.findReplace", "search", Workspace, action::ToggleReplace, &["Ctrl+H", "Ctrl+Alt+F"], &["Command+Alt+F"]),
    shortcut!("find-next", "menu.findNext", "search", Workspace, action::FindNext, &["Ctrl+G", "F3"], &["Command+G"]),
    shortcut!("find-previous", "menu.findPrev", "search", Workspace, action::FindPrev, &["Ctrl+Shift+G", "Shift+F3"], &["Command+Shift+G"]),
    shortcut!("heading-1", "menu.heading1", "format", Document, action::Heading1, &["Ctrl+1", "Ctrl+Alt+1"], &["Command+1", "Command+Alt+1"]),
    shortcut!("heading-2", "menu.heading2", "format", Document, action::Heading2, &["Ctrl+2", "Ctrl+Alt+2"], &["Command+2", "Command+Alt+2"]),
    shortcut!("heading-3", "menu.heading3", "format", Document, action::Heading3, &["Ctrl+3", "Ctrl+Alt+3"], &["Command+3", "Command+Alt+3"]),
    shortcut!("heading-4", "chrome.heading4", "format", Document, action::Heading4, &["Ctrl+4"], &["Command+4"]),
    shortcut!("heading-5", "chrome.heading5", "format", Document, action::Heading5, &["Ctrl+5"], &["Command+5"]),
    shortcut!("heading-6", "chrome.heading6", "format", Document, action::Heading6, &["Ctrl+6"], &["Command+6"]),
    shortcut!("paragraph", "menu.paragraph", "format", Document, action::Paragraph, &["Ctrl+0", "Ctrl+Alt+0"], &["Command+0", "Command+Alt+0"]),
    shortcut!("promote-heading", "chrome.liftHeading", "format", Document, action::PromoteHeading, &["Ctrl+="], &["Command+="]),
    shortcut!("demote-heading", "chrome.sinkHeading", "format", Document, action::DemoteHeading, &["Ctrl+-"], &["Command+-"]),
    shortcut!("bold", "menu.bold", "format", Document, action::Bold, &["Ctrl+B"], &["Command+B"]),
    shortcut!("italic", "menu.italic", "format", Document, action::Italic, &["Ctrl+I"], &["Command+I"]),
    shortcut!("underline", "chrome.underline", "format", Document, action::Underline, &["Ctrl+U"], &["Command+U"]),
    shortcut!("strike", "menu.strike", "format", Document, action::Strike, &["Alt+Shift+5"], &["Alt+Shift+5"]),
    shortcut!("inline-code", "menu.inlineCode", "format", Document, action::InlineCode, &["Ctrl+`", "Ctrl+Shift+C"], &["Command+Shift+C"]),
    shortcut!("highlight", "chrome.highlight", "format", Document, action::Highlight, &[], &[]),
    shortcut!("quote", "menu.quote", "format", Document, action::Quote, &["Ctrl+Shift+Q"], &["Command+Alt+Q"]),
    shortcut!("bullet-list", "menu.bulletList", "format", Document, action::BulletList, &["Ctrl+Shift+]"], &["Command+Shift+]"]),
    shortcut!("ordered-list", "menu.orderedList", "format", Document, action::OrderedList, &["Ctrl+Shift+["], &["Command+Shift+["]),
    shortcut!("task-list", "menu.taskList", "format", Document, action::TaskList, &["Ctrl+Shift+X"], &["Command+Shift+X"]),
    shortcut!("menu-clear-format", "shortcuts.clear", "format", Document, action::ClearFormatting, &["Ctrl+\\"], &["Command+\\"]),
    shortcut!("menu-link", "shortcuts.link", "insert", Document, action::Link, &["Ctrl+K"], &["Command+K"]),
    shortcut!("insert-image", "chrome.imageMenu", "insert", Document, action::InsertImage, &[], &[]),
    shortcut!("insert-code-block", "shortcuts.code", "insert", Document, action::CodeBlock, &["Ctrl+Shift+K"], &["Command+Shift+K"]),
    shortcut!("insert-table", "shortcuts.table", "insert", Document, action::Table, &["Ctrl+Shift+T"], &["Command+Shift+T"]),
    shortcut!("insert-math-block", "shortcuts.math", "insert", Document, action::MathBlock, &["Ctrl+Shift+M"], &["Command+Shift+M"]),
    shortcut!("insert-rule", "menu.rule", "insert", Document, action::Rule, &["Ctrl+Shift+H"], &["Command+Shift+H"]),
    shortcut!("insert-footnote", "chrome.footnote", "insert", Document, action::InsertFootnote, &[], &[]),
    shortcut!("insert-toc", "chrome.toc", "insert", Document, action::InsertToc, &[], &[]),
    shortcut!("insert-callout", "chrome.quickInsertCallout", "insert", Document, action::InsertCallout, &[], &[]),
    shortcut!("insert-details", "chrome.details", "insert", Document, action::InsertDetails, &[], &[]),
    shortcut!("block-comment", "menu.blockComment", "insert", Document, action::BlockComment, &[], &[]),
    shortcut!("front-matter", "menu.editFrontMatter", "insert", Document, action::EditFrontMatter, &[], &[]),
    shortcut!("paragraph-before", "chrome.paragraphBefore", "insert", Editor, mark::ParagraphBefore, &["Ctrl+Shift+Enter"], &["Command+Shift+Enter"]),
    shortcut!("paragraph-after", "chrome.paragraphAfter", "insert", Editor, mark::ParagraphAfter, &["Ctrl+Enter"], &["Command+Enter"]),
    shortcut!("toggle-source", "shortcuts.toggleSource", "view", Workspace, action::ToggleSource, &["Ctrl+E"], &["Command+E"]),
    shortcut!("mode-semantic", "shortcuts.semantic", "view", Workspace, action::ModeSemantic, &[], &[]),
    shortcut!("mode-source", "menu.modeSource", "view", Workspace, action::ModeSource, &[], &[]),
    shortcut!("mode-split", "shortcuts.split", "view", Workspace, action::ModeSplit, &[], &[]),
    shortcut!("toggle-edit-lock", "menu.toggleEditLock", "view", Workspace, action::ToggleEditLock, &["Ctrl+Shift+R"], &["Command+Shift+R"]),
    shortcut!("toggle-theme", "shortcuts.toggleTheme", "view", Workspace, action::ToggleTheme, &["Ctrl+Shift+L"], &["Command+Shift+L"]),
    shortcut!("toggle-focus", "shortcuts.toggleSidebar", "view", Workspace, action::ToggleSidebar, &["Ctrl+Shift+F", "Ctrl+Alt+S"], &["Command+Shift+F", "Command+Alt+S"]),
    shortcut!("toggle-toolbar", "shortcuts.toggleToolbar", "view", Workspace, action::ToggleToolbar, &["Ctrl+Shift+B"], &["Command+Shift+B"]),
    shortcut!("toggle-outline", "shortcuts.outline", "view", Workspace, action::ToggleOutline, &["Ctrl+Alt+O"], &["Command+Alt+O"]),
    shortcut!("toggle-markdown-mini", "shortcuts.toggleMini", "view", Workspace, action::ToggleMini, &["Ctrl+Alt+M"], &["Command+Alt+M"]),
    shortcut!("zoom-in", "shortcuts.zoomIn", "view", Workspace, action::ZoomIn, &["Ctrl+Plus"], &["Command+Plus"]),
    shortcut!("zoom-out", "shortcuts.zoomOut", "view", Workspace, action::ZoomOut, &["Ctrl+Alt+-"], &["Command+Alt+-"]),
    shortcut!("zoom-reset", "shortcuts.zoomReset", "view", Workspace, action::ZoomReset, &["Ctrl+Alt+Shift+0"], &["Command+Alt+Shift+0"]),
    shortcut!("show-home", "navigation.home", "view", Workspace, action::ShowHome, &["Ctrl+Alt+Shift+H"], &["Command+Alt+Shift+H"]),
    shortcut!("open-settings", "menu.openSettings", "view", Application, action::OpenSettings, &["Ctrl+,"], &["Command+,"]),
    shortcut!("quit", "menu.quit", "files", Application, action::Quit, &["Ctrl+Q"], &["Command+Q"]),
];

impl Shortcut {
    pub fn defaults(&self) -> Vec<String> {
        (if MACOS { self.macos } else { self.windows }).iter().map(|key| normalize(key).expect("内置键位").0).collect()
    }
}
struct Bindings { values: Vec<Vec<String>>, errors: Vec<UiMessage> }
impl Global for Bindings {}

// 默认值不物化到旧版字段，避免平台默认值被误认为用户自定义。
pub(crate) fn defaults() -> Value { json!({}) }
pub(crate) fn current_keys(id: &str, cx: &App) -> Vec<String> {
    let Some(index) = SHORTCUTS.iter().position(|entry| entry.id == id) else { return Vec::new(); };
    cx.try_global::<Bindings>().map_or_else(|| SHORTCUTS[index].defaults(), |b| b.values[index].clone())
}
pub(crate) fn current(id: &str, cx: &App) -> String {
    current_keys(id, cx).iter().map(|key| display(key)).collect::<Vec<_>>().join(" / ")
}
pub(crate) fn display(value: &str) -> String {
    if MACOS {
        value.replace("Ctrl+", "⌃").replace("Alt+", "⌥").replace("Shift+", "⇧").replace("Command+", "⌘").replace("Plus", "+")
    } else { value.replace("Plus", "+") }
}
pub(crate) fn action_label(action: &dyn Action, cx: &App) -> Option<String> {
    SHORTCUTS.iter().find(|entry| (entry.action)().name() == action.name()).map(|entry| current(entry.id, cx))
}
/// 工具栏只展示对应命令的真实键位；下拉选择器不借用其中某项的快捷键。
pub(crate) fn toolbar_label(id: &str, cx: &App) -> String {
    let command = match id {
        "h1" => "heading-1", "link" => "menu-link", "ul" => "bullet-list", "task" => "task-list",
        "codeblock" => "insert-code-block", "table" => "insert-table", "footnote" => "insert-footnote",
        "toc" => "insert-toc", "callout" => "insert-callout", "details" => "insert-details",
        "search" => "find", "outline" => "toggle-outline", "hide-toolbar" => "toggle-toolbar",
        other => other,
    };
    current(command, cx)
}
pub(crate) fn warnings(cx: &App) -> Option<String> {
    cx.try_global::<Bindings>().filter(|b| !b.errors.is_empty()).map(|b| b.errors.iter().map(|e| e.render(ToolbarLocale::current(cx))).collect::<Vec<_>>().join("\n"))
}

fn normalize(text: &str) -> Result<(String, String), UiMessage> {
    if text.len() > 40 { return Err(UiMessage::new("shortcuts.maxLength")); }
    let text = text.trim().replace('⌘', "Command+").replace('⌃', "Ctrl+").replace('⌥', "Alt+").replace('⇧', "Shift+");
    let text = if text.ends_with("++") { format!("{}Plus", &text[..text.len()-1]) } else { text };
    let parts: Vec<_> = text.split('+').map(str::trim).collect();
    let (mut ctrl, mut cmd, mut shift, mut alt, mut function_modifier) = (false, false, false, false, false);
    for modifier in &parts[..parts.len()-1] {
        let flag = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => &mut ctrl, "command" | "cmd" | "meta" if MACOS => &mut cmd,
            "alt" | "option" => &mut alt, "shift" => &mut shift,
            "fn" => &mut function_modifier,
            _ => return Err(UiMessage::new("shortcuts.invalidCombination")),
        };
        if *flag { return Err(UiMessage::new("shortcuts.duplicateModifiers")); } *flag = true;
    }
    let key = parts.last().unwrap().to_ascii_lowercase();
    let function = key.strip_prefix('f').and_then(|v| v.parse::<u8>().ok()).is_some_and(|v| (1..=12).contains(&v) && key == format!("f{v}"));
    let named = matches!(key.as_str(), "tab" | "enter" | "space" | "backspace" | "delete" | "home" | "end" | "pageup" | "pagedown" | "left" | "right" | "up" | "down" | "plus");
    let literal = key.chars().count() == 1 && !key.chars().any(char::is_whitespace);
    if !(function || named || literal) || (!ctrl && !cmd && !alt && !function) { return Err(UiMessage::new("shortcuts.invalidCombination")); }
    let mut canonical = String::new(); let mut binding = String::new();
    for (enabled, label, token) in [(ctrl, "Ctrl+", "ctrl-"), (alt, "Alt+", "alt-"), (shift, "Shift+", "shift-"), (cmd, "Command+", "cmd-"), (function_modifier, "Fn+", "fn-")] {
        if enabled { canonical.push_str(label); binding.push_str(token); }
    }
    let label = match key.as_str() { "plus" => "Plus".into(), _ if literal || function => key.to_ascii_uppercase(), _ => format!("{}{}", key[..1].to_uppercase(), &key[1..]) };
    canonical.push_str(&label); binding.push_str(if key == "plus" { "+" } else { &key });
    Ok((canonical, binding))
}

pub(super) fn record(key: &Keystroke) -> Result<String, UiMessage> {
    if key.modifiers.platform && !MACOS { return Err(UiMessage::new("shortcuts.systemReserved")); }
    let m = key.modifiers;
    if (!MACOS && m.alt && !m.control && key.key == "f4") || (!MACOS && m.control && m.alt && key.key == "delete")
        || (MACOS && m.platform && (matches!(key.key.as_str(), "space" | "tab" | "`")
            || (!m.shift && !m.control && key.key == "h") || (m.shift && key.key == "q"))) {
        return Err(UiMessage::new("shortcuts.systemReserved"));
    }
    let mut text = String::new();
    for (enabled, name) in [(key.modifiers.control, "Ctrl+"), (key.modifiers.alt, "Alt+"), (key.modifiers.shift, "Shift+"), (key.modifiers.platform, "Command+"), (key.modifiers.function, "Fn+")] {
        if enabled { text.push_str(name); }
    }
    text.push_str(if key.key == "+" { "Plus" } else { &key.key });
    normalize(&text).map(|v| v.0)
}

fn binding(entry: &Shortcut, value: &str, cx: &App) -> Result<KeyBinding, UiMessage> {
    let (_, keys) = normalize(value)?;
    KeyBinding::load(&keys, (entry.action)(), Some(KeyBindingContextPredicate::parse(entry.scope.predicate()).expect("快捷键上下文").into()), false, None, cx)
        .map_err(|e| UiMessage::new("shortcuts.parseFailed").arg("error", e))
}

// 检查实际可同时出现的焦点路径。TextField 的固定复制键与编辑器自定义可合法共存。
fn overlaps(left: &KeyBinding, right: &KeyBinding) -> bool {
    const PATHS: &[&[&str]] = &[
        &["Workspace"], &["Workspace", "NomoMark"], &["Workspace", "Outline"],
        &["Workspace", "TextField"], &["Workspace", "NomoMark", "TextField"],
        &["NomoMark"], &["Settings", "TextField"], &["Settings"],
    ];
    PATHS.iter().any(|path| {
        let contexts: Vec<_> = path.iter().map(|p| KeyContext::parse(p).expect("焦点路径")).collect();
        [left, right].iter().all(|b| b.predicate().is_none_or(|p| p.depth_of(&contexts).is_some()))
    })
}
fn conflicts(left: &KeyBinding, right: &KeyBinding) -> bool {
    left.action().name() != right.action().name() && left.keystrokes() == right.keystrokes() && overlaps(left, right)
}
pub(super) fn validate_keys(id: &str, values: &[String], cx: &App) -> Result<Vec<String>, UiMessage> {
    let entry = SHORTCUTS.iter().find(|e| e.id == id).ok_or_else(|| UiMessage::new("shortcuts.unknown"))?;
    if values.len() > MAX_BINDINGS { return Err(UiMessage::new("shortcuts.tooMany")); }
    let mut normalized = Vec::new();
    let keymap = cx.key_bindings(); let keymap = keymap.borrow();
    for value in values {
        let candidate = binding(entry, value, cx)?;
        for existing in keymap.bindings() {
            if conflicts(&candidate, existing) {
                let label = SHORTCUTS.iter().find(|e| (e.action)().name() == existing.action().name()).map_or("shortcuts.existing", |e| e.label);
                return Err(UiMessage::new("shortcuts.conflict").text_arg("label", label));
            }
        }
        let canonical = normalize(value)?.0;
        if normalized.contains(&canonical) { return Err(UiMessage::new("shortcuts.duplicateBinding")); }
        normalized.push(canonical);
    }
    Ok(normalized)
}
pub(super) fn save(id: &str, values: &[String], cx: &mut App) {
    // Settings 按平台和命令两级合并磁盘值，保留其它平台、旧字段和未知命令。
    Settings::set(cx, &format!("shortcutPreferences.{PLATFORM_KEY}.{id}"), json!(values));
}
pub(super) fn reset_all(cx: &mut App) {
    let values: serde_json::Map<String, Value> = SHORTCUTS.iter().map(|e| (e.id.into(), json!(e.defaults()))).collect();
    Settings::set(cx, &format!("shortcutPreferences.{PLATFORM_KEY}"), Value::Object(values));
}

pub(crate) fn bind_keys(cx: &mut App) {
    let preferences = cx.global::<Settings>().value("shortcutPreferences");
    let mut errors = Vec::new();
    let mut values = vec![Vec::new(); SHORTCUTS.len()];
    // 自定义先于默认值；后续新增默认键位不能抢占已有自定义。
    let mut pending = Vec::new();
    for (index, entry) in SHORTCUTS.iter().enumerate() {
        let platform = preferences.get(PLATFORM_KEY).and_then(|v| v.get(entry.id));
        let legacy = preferences.get(entry.id).and_then(Value::as_str);
        let keys = if let Some(value) = platform {
            value.as_array().filter(|a| a.len() <= MAX_BINDINGS).and_then(|a| a.iter().map(|v| v.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>())
        } else if let Some(value) = legacy {
            // 旧版 Ctrl 表示跨平台主修饰键，读取时转换，不回写原值。
            let value = value.replace('⌘', "Command+").replace('⇧', "Shift+").replace('⌥', "Alt+");
            let value = value.split('+').map(|part| match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" | "meta" => if MACOS { "Command" } else { "Ctrl" },
                _ => part.trim(),
            }).collect::<Vec<_>>().join("+");
            let mut keys = vec![value];
            if entry.id == "toggle-focus" { keys.extend(entry.defaults().into_iter().skip(1)); }
            Some(keys)
        } else { Some(entry.defaults()) };
        let keys = keys.and_then(|keys| keys.iter().map(|v| normalize(v).ok().map(|v| v.0)).collect::<Option<Vec<_>>>());
        let custom = platform.is_some() || legacy.is_some();
        let keys = keys.unwrap_or_else(|| { errors.push(UiMessage::new("shortcuts.invalidCombination").context("shortcuts.defaulted").text_arg("label", entry.label)); entry.defaults() });
        pending.push((!custom, index, keys));
    }
    pending.sort_by_key(|(default, index, _)| (*default, *index));
    let mut accepted: Vec<KeyBinding> = Vec::new();
    for (_, index, keys) in pending {
        let entry = &SHORTCUTS[index];
        for key in keys {
            let candidate = match binding(entry, &key, cx) { Ok(b) => b, Err(e) => { errors.push(e); continue; } };
            let fixed = cx.key_bindings(); let fixed = fixed.borrow();
            if accepted.iter().chain(fixed.bindings()).any(|b| conflicts(&candidate, b)) {
                errors.push(UiMessage::new("shortcuts.inactiveConflict").text_arg("label", entry.label).arg("shortcut", display(&key)));
                continue;
            }
            if !values[index].contains(&key) { values[index].push(key); accepted.push(candidate); }
        }
    }
    cx.bind_keys(accepted);
    cx.set_global(Bindings { values, errors });
}
