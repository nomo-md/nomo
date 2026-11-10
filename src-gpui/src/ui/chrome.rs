//! 正式窗口标题栏，页面基准为 master 339fb932 的 AppTitleBar。
//!
//! 只保存菜单、焦点及宿主提供的轻量状态；不持有文档、正文或撤销历史。
//! 宿主必须先连接事件，再声明相应 capability。未连接的主分支功能保留可见的禁用项。

use super::toolbar_i18n::ToolbarLocale;
use super::components::{IconName, icon, tooltip};
use super::{window_chrome, workspace as action};
use super::popup::{popup, reduced_motion, PopupAnchor, PopupPlacement};
use crate::{editor, settings::EditorMode, theme::Theme};
use gpui::{
    Action, App, Context, Div, EventEmitter, FocusHandle,
    Focusable, FontWeight, IntoElement, KeyDownEvent, MouseButton, Pixels, Point, Render,
    ScrollHandle, SharedString, Stateful, Window, anchored, deferred, div, point, prelude::*,
    px,
};
use std::path::PathBuf;

pub const TITLEBAR_HEIGHT: f32 = 42.0;
const MENU_WIDTH: f32 = 278.0;

/// 只有宿主已经实现并订阅的业务能力才能启用菜单项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeCapability {
    NewWindow, Recent, ExportHtml, ExportPdf, ExportWord, TablePicker, LinkPicker,
    HeadingLevels, HeadingAdjustment, MathInline, Callout, Details, BlockComment,
    InlineComment, Diagrams, Footnote, Toc, FrontMatter, ClearFormatting,
    Mini, SoftwareUpdate, UnavailableNotice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramKind { Default, Blank, Flowchart, Sequence, Class, State, Pie, Gantt, EntityRelationship }

pub(crate) use crate::document_export::Format as ExportFormat;

/// 这四项在固定 master 本身调用 comingSoon，保留相同业务语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableFeature { ParagraphBefore, ParagraphAfter, InlineMath, Image }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChromeCommand {
    Heading(u8), AdjustHeading { increase: bool }, MathInline, Callout, Details,
    BlockComment, InlineComment, Diagram(DiagramKind), Footnote, Toc, ClearFormatting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChromeEvent {
    NewWindow,
    OpenRecent { path: PathBuf, folder: bool },
    RemoveRecent(PathBuf),
    ClearRecent,
    Export(ExportFormat),
    TablePicker,
    LinkPicker,
    FrontMatter,
    Command(ChromeCommand),
    ToggleSidebar,
    ToggleMini,
    SoftwareUpdate,
    Unavailable(UnavailableFeature),
    /// 必须由 Workspace 执行未保存确认，不能在标题栏直接移除窗口。
    CloseWindow,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChromeRecentEntry {
    pub path: PathBuf,
    pub label: SharedString,
    pub folder: bool,
    pub missing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeUpdateState { Available, Downloading(u8), Downloaded }

#[derive(Clone, Debug, PartialEq)]
pub struct ChromeState {
    pub(crate) view_menu: ViewMenuState,
    pub can_undo: bool,
    pub can_redo: bool,
    pub word_export_busy: bool,
    pub recent: Vec<ChromeRecentEntry>,
    /// 单文档隐藏标签栏时在标题栏居中显示的文档名与未保存标记；显示标签栏或在主页时为 None。
    pub document_title: Option<(SharedString, bool)>,
    pub capabilities: Vec<ChromeCapability>,
    pub update: Option<ChromeUpdateState>,
}

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            view_menu: ViewMenuState::default(), can_undo: false, can_redo: false, word_export_busy: false,
            recent: Vec::new(), document_title: None, capabilities: Vec::new(), update: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu { File, Edit, Paragraph, Format, View, ViewText, ViewCorner, ViewAppearance, Recent, Export, Heading, Diagram, Window }

impl Menu {
    fn label(self, locale: ToolbarLocale) -> &'static str {
        match self {
            Self::File => locale.app_text("menu.file"), Self::Edit => locale.app_text("menu.edit"), Self::Paragraph => locale.app_text("chrome.paragraph"),
            Self::Format => locale.app_text("menu.format"), Self::View => locale.app_text("menu.view"), Self::Recent => locale.app_text("chrome.openRecent"),
            Self::ViewText => locale.app_text("menu.viewText"), Self::ViewCorner => locale.app_text("menu.viewCorner"), Self::ViewAppearance => locale.app_text("menu.viewAppearance"),
            Self::Export => locale.app_text("chrome.export"), Self::Heading => locale.app_text("chrome.quickInsertHeading"), Self::Diagram => locale.app_text("chrome.diagram"),
            Self::Window => locale.app_text("chrome.window"),
        }
    }
    fn index(self) -> usize {
        match self { Self::File => 0, Self::Edit => 1, Self::Paragraph => 2, Self::Format => 3, Self::View => 4, _ => 0 }
    }
}

#[derive(Clone)]
enum MenuAction {
    Dispatch(fn() -> Box<dyn Action>), Emit(ChromeEvent), Submenu(Menu),
    Minimize, Maximize, None,
}

#[derive(Clone)]
struct MenuItem {
    label: SharedString,
    shortcut: SharedString,
    action: MenuAction,
    enabled: bool,
    checked: Option<bool>,
    separator: bool,
    icon: Option<IconName>,
    remove_recent: Option<PathBuf>,
}

impl MenuItem {
    fn new(label: impl Into<SharedString>, shortcut: impl Into<SharedString>, action: MenuAction, enabled: bool) -> Self {
        Self { label: label.into(), shortcut: shortcut.into(), action, enabled, checked: None, separator: false, icon: None, remove_recent: None }
    }
    fn separator() -> Self {
        Self { separator: true, ..Self::new("", "", MenuAction::None, false) }
    }
    fn submenu(menu: Menu, locale: ToolbarLocale) -> Self { Self::new(menu.label(locale), "", MenuAction::Submenu(menu), true) }
    fn checked(mut self, checked: bool) -> Self { self.checked = Some(checked); self }
}

mod view_menu;
pub(crate) use view_menu::{ViewMenuState, native_edit_lock_item, native_view_menu};

pub struct Chrome {
    focus: FocusHandle,
    #[cfg(target_os = "windows")]
    caption_focus: window_chrome::CaptionFocus,
    restore_focus: Option<FocusHandle>,
    state: ChromeState,
    menu: Option<Menu>,
    submenu: Option<Menu>,
    selected: Option<usize>,
    subselected: Option<usize>,
    menu_scroll: ScrollHandle,
    submenu_scroll: ScrollHandle,
    context_position: Option<Point<Pixels>>,
}

impl EventEmitter<ChromeEvent> for Chrome {}
impl Focusable for Chrome {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl Chrome {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(), restore_focus: None, state: ChromeState::default(),
            #[cfg(target_os = "windows")]
            caption_focus: window_chrome::CaptionFocus::new(cx, true),
            menu: None, submenu: None, selected: None, subselected: None,
            menu_scroll: ScrollHandle::new(), submenu_scroll: ScrollHandle::new(), context_position: None,
        }
    }

    /// 宿主仅传当前标签摘要；最近项上限与主分支一致，不保留文档 Entity。
    pub fn set_state(&mut self, mut state: ChromeState, cx: &mut Context<Self>) {
        state.recent.truncate(10);
        if self.state != state { self.state = state; cx.notify(); }
    }

    fn capable(&self, capability: ChromeCapability) -> bool { self.state.capabilities.contains(&capability) }

    fn event_item(&self, label: &'static str, shortcut: &'static str, event: ChromeEvent, capability: ChromeCapability, needs_document: bool) -> MenuItem {
        MenuItem::new(label, shortcut, MenuAction::Emit(event), self.capable(capability) && (!needs_document || self.state.view_menu.mode.is_some()))
    }

    fn items(&self, menu: Menu, maximized: bool, cx: &App) -> Vec<MenuItem> {
        use ChromeCapability as C;
        use ChromeCommand as Cmd;
        use ChromeEvent as E;
        use MenuAction as A;
        let locale = ToolbarLocale::current(cx);
        let doc = self.state.view_menu.mode.is_some();
        let separator = MenuItem::separator;
        let action = |label: &'static str, shortcut: &'static str, action: fn() -> Box<dyn Action>, enabled: bool| MenuItem::new(label, shortcut, A::Dispatch(action), enabled);
        let event = |label, shortcut, value, capability, document| self.event_item(label, shortcut, value, capability, document);
        let mut entries = match menu {
            Menu::File => vec![
                action(locale.app_text("shortcuts.newFile"), "Ctrl + N", || Box::new(action::NewFile), true),
                event(locale.app_text("settings.openDefaultBehavior.choice.new-window"), "Ctrl + Shift + N", E::NewWindow, C::NewWindow, false), separator(),
                action(locale.app_text("chrome.openFile"), "Ctrl + O", || Box::new(action::OpenFile), true),
                action(locale.app_text("chrome.openFolder"), "Ctrl + Shift + O", || Box::new(action::OpenFolder), true),
                MenuItem::submenu(Menu::Recent, locale),
                action(locale.app_text("chrome.recover"), "", || Box::new(action::RecoverDocuments), true), separator(),
                action(locale.app_text("shortcuts.save"), "Ctrl + S", || Box::new(action::Save), doc),
                action(locale.app_text("menu.saveAs"), "Ctrl + Shift + S", || Box::new(action::SaveAs), doc), separator(),
                MenuItem::submenu(Menu::Export, locale), separator(),
                action(locale.app_text("chrome.closeFile"), "Ctrl + W", || Box::new(action::CloseTab), doc),
                MenuItem::new(locale.app_text("settings.closeWindowBehavior.choice.close-window"), "Alt + F4", A::Emit(E::CloseWindow), true), separator(),
                MenuItem::new(locale.app_text("chrome.quit"), "", A::Emit(E::Quit), true),
            ],
            Menu::Edit => vec![
                action(locale.app_text("menu.undo"), "Ctrl + Z", || Box::new(editor::Undo), doc && self.state.can_undo),
                action(locale.app_text("menu.redo"), "Ctrl + Y", || Box::new(editor::Redo), doc && self.state.can_redo),
                separator(),
                self.state.view_menu.edit_lock_item(locale),
            ],
            Menu::Paragraph => vec![
                MenuItem::submenu(Menu::Heading, locale),
                action(locale.app_text("chrome.paragraph"), "Ctrl + 0", || Box::new(action::Paragraph), doc),
                event(locale.app_text("chrome.liftHeading"), "Ctrl + =", E::Command(Cmd::AdjustHeading { increase: true }), C::HeadingAdjustment, true),
                event(locale.app_text("chrome.sinkHeading"), "Ctrl + -", E::Command(Cmd::AdjustHeading { increase: false }), C::HeadingAdjustment, true), separator(),
                event(locale.app_text("menu.table"), "Ctrl + Shift + T", E::TablePicker, C::TablePicker, true),
                action(locale.app_text("menu.codeBlock"), "Ctrl + Shift + K", || Box::new(action::CodeBlock), doc),
                action(locale.app_text("chrome.mathBlock"), "Ctrl + Shift + M", || Box::new(action::MathBlock), doc),
                MenuItem::new(locale.app_text("chrome.mathInline"), "", A::Emit(E::Command(Cmd::MathInline)), doc && self.capable(C::MathInline) && self.state.view_menu.mode != Some(EditorMode::Source)), separator(),
                action(locale.app_text("menu.quote"), "Ctrl + Shift + Q", || Box::new(action::Quote), doc),
                event(locale.app_text("chrome.quickInsertCallout"), "Ctrl + Shift + A", E::Command(Cmd::Callout), C::Callout, true),
                event(locale.app_text("chrome.details"), "", E::Command(Cmd::Details), C::Details, true),
                event(locale.app_text("menu.blockComment"), "", E::Command(Cmd::BlockComment), C::BlockComment, true),
                action(locale.app_text("menu.orderedList"), "Ctrl + Shift + [", || Box::new(action::OrderedList), doc),
                action(locale.app_text("menu.bulletList"), "Ctrl + Shift + ]", || Box::new(action::BulletList), doc),
                action(locale.app_text("menu.taskList"), "Ctrl + Shift + X", || Box::new(action::TaskList), doc), separator(),
                action(locale.app_text("chrome.paragraphBefore"), "Ctrl + Shift + Enter", || Box::new(crate::nomo_mark::view::ParagraphBefore), doc),
                action(locale.app_text("chrome.paragraphAfter"), "Ctrl + Enter", || Box::new(crate::nomo_mark::view::ParagraphAfter), doc), separator(),
                MenuItem::submenu(Menu::Diagram, locale),
                event(locale.app_text("chrome.footnote"), "", E::Command(Cmd::Footnote), C::Footnote, true),
                action(locale.app_text("chrome.horizontalRule"), "Ctrl + Shift + H", || Box::new(action::Rule), doc),
                event(locale.app_text("chrome.toc"), "", E::Command(Cmd::Toc), C::Toc, true),
                event(locale.app_text("menu.editFrontMatter"), "", E::FrontMatter, C::FrontMatter, true),
            ],
            Menu::Format => vec![
                action(locale.app_text("menu.bold"), "Ctrl + B", || Box::new(action::Bold), doc),
                action(locale.app_text("menu.italic"), "Ctrl + I", || Box::new(action::Italic), doc),
                action(locale.app_text("chrome.underline"), "Ctrl + U", || Box::new(action::Underline), doc),
                action(locale.app_text("chrome.inlineCode"), "Ctrl + `", || Box::new(action::InlineCode), doc),
                event(locale.app_text("chrome.inlineMath"), "", E::Unavailable(UnavailableFeature::InlineMath), C::UnavailableNotice, true), separator(),
                action(locale.app_text("menu.strike"), "Alt + Shift + 5", || Box::new(action::Strike), doc),
                action(locale.app_text("chrome.highlight"), "", || Box::new(action::Highlight), doc),
                event(locale.app_text("chrome.inlineComment"), "", E::Command(Cmd::InlineComment), C::InlineComment, true), separator(),
                event(locale.app_text("menu.link"), "Ctrl + K", E::LinkPicker, C::LinkPicker, true),
                event(locale.app_text("chrome.imageMenu"), "", E::Unavailable(UnavailableFeature::Image), C::UnavailableNotice, true), separator(),
                event(locale.app_text("shortcuts.clear"), r"Ctrl + \", E::Command(Cmd::ClearFormatting), C::ClearFormatting, true),
            ],
            Menu::View | Menu::ViewText | Menu::ViewCorner | Menu::ViewAppearance => self.state.view_menu.items(menu, locale),
            Menu::Heading => vec![
                action(locale.app_text("menu.heading1"), "Ctrl + 1", || Box::new(action::Heading1), doc),
                action(locale.app_text("menu.heading2"), "Ctrl + 2", || Box::new(action::Heading2), doc),
                action(locale.app_text("menu.heading3"), "Ctrl + 3", || Box::new(action::Heading3), doc),
                event(locale.app_text("chrome.heading4"), "Ctrl + 4", E::Command(Cmd::Heading(4)), C::HeadingLevels, true),
                event(locale.app_text("chrome.heading5"), "Ctrl + 5", E::Command(Cmd::Heading(5)), C::HeadingLevels, true),
                event(locale.app_text("chrome.heading6"), "Ctrl + 6", E::Command(Cmd::Heading(6)), C::HeadingLevels, true),
            ],
            Menu::Export => vec![
                event(locale.app_text("shortcuts.exportHtml"), "", E::Export(ExportFormat::Html), C::ExportHtml, true),
                event(locale.app_text("shortcuts.exportPdf"), "", E::Export(ExportFormat::Pdf), C::ExportPdf, true),
                MenuItem::new(locale.app_text("chrome.exportWord"), "", A::Emit(E::Export(ExportFormat::Word)), doc && self.capable(C::ExportWord) && !self.state.word_export_busy),
            ],
            Menu::Diagram => {
                let mut entries = vec![event(locale.app_text("chrome.defaultDiagram"), "", E::Command(Cmd::Diagram(DiagramKind::Default)), C::Diagrams, true), event(locale.app_text("chrome.blankDiagram"), "mermaid", E::Command(Cmd::Diagram(DiagramKind::Blank)), C::Diagrams, true), separator()];
                for (label, shortcut, kind) in [
                    (locale.app_text("settings.defaultDiagramType.choice.flowchart"), "flowchart", DiagramKind::Flowchart), (locale.app_text("settings.defaultDiagramType.choice.sequenceDiagram"), "sequenceDiagram", DiagramKind::Sequence),
                    (locale.app_text("settings.defaultDiagramType.choice.classDiagram"), "classDiagram", DiagramKind::Class), (locale.app_text("settings.defaultDiagramType.choice.stateDiagram"), "stateDiagram", DiagramKind::State),
                    (locale.app_text("settings.defaultDiagramType.choice.pie"), "pie", DiagramKind::Pie), (locale.app_text("settings.defaultDiagramType.choice.gantt"), "gantt", DiagramKind::Gantt),
                    (locale.app_text("chrome.erDiagram"), "erDiagram", DiagramKind::EntityRelationship),
                ] { entries.push(event(label, shortcut, E::Command(Cmd::Diagram(kind)), C::Diagrams, true)); }
                entries
            }
            Menu::Recent => {
                let mut entries: Vec<_> = self.state.recent.iter().take(10).map(|recent| {
                    let mut entry = MenuItem::new(recent.label.clone(), if recent.missing { locale.app_text("chrome.unavailable") } else { "" }, A::Emit(E::OpenRecent { path: recent.path.clone(), folder: recent.folder }), self.capable(C::Recent) && !recent.missing);
                    entry.icon = Some(if recent.folder { IconName::Folder } else { IconName::FileText });
                    entry.remove_recent = Some(recent.path.clone());
                    entry
                }).collect();
                if entries.is_empty() { entries.push(MenuItem::new(locale.app_text("chrome.noRecentFiles"), "", A::None, false)); }
                else { entries.push(separator()); entries.push(event(locale.app_text("chrome.clearRecentFiles"), "", E::ClearRecent, C::Recent, false)); }
                entries
            }
            Menu::Window => vec![
                MenuItem::new(locale.app_text("window.minimize"), "", A::Minimize, true),
                MenuItem::new(if maximized { locale.app_text("window.restore") } else { locale.app_text("window.maximize") }, "", A::Maximize, true), separator(),
                MenuItem::new(locale.app_text("settings.closeWindowBehavior.choice.close-window"), "Alt + F4", A::Emit(E::CloseWindow), true),
            ],
        };
        for entry in &mut entries {
            if self.state.view_menu.editing_locked && matches!(menu, Menu::Paragraph | Menu::Format | Menu::Heading | Menu::Diagram) {
                entry.enabled = false;
            }
            let command: Option<Box<dyn Action>> = match &entry.action {
                A::Dispatch(action) => {
                    let command = action();
                    Some(if command.name() == action::ModeSource.name() { Box::new(action::ToggleSource) } else { command })
                }
                A::Emit(E::Export(ExportFormat::Html)) => Some(Box::new(action::ExportHtml)),
                A::Emit(E::Export(ExportFormat::Pdf)) => Some(Box::new(action::ExportPdf)),
                A::Emit(E::TablePicker) => Some(Box::new(action::Table)),
                A::Emit(E::LinkPicker) => Some(Box::new(action::Link)),
                A::Emit(E::Command(Cmd::ClearFormatting)) => Some(Box::new(action::ClearFormatting)),
                _ => None,
            };
            if let Some(label) = command.as_ref().and_then(|command| super::shortcuts::action_label(command.as_ref(), cx)) { entry.shortcut = label.into(); }
        }
        entries
    }

    fn open(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_none() { self.restore_focus = window.focused(cx); window.focus(&self.focus, cx); }
        self.menu = Some(menu); self.submenu = None; self.selected = None; self.subselected = None;
        self.menu_scroll.set_offset(point(px(0.0), px(0.0))); self.context_position = None;
        cx.notify();
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None; self.submenu = None; self.selected = None; self.subselected = None;
        self.context_position = None;
        if let Some(focus) = self.restore_focus.take() { window.focus(&focus, cx); }
        cx.notify();
    }

    fn run(&mut self, item: &MenuItem, window: &mut Window, cx: &mut Context<Self>) {
        if !item.enabled { return; }
        if let MenuAction::Submenu(menu) = item.action {
            self.submenu = Some(menu); self.subselected = None;
            self.submenu_scroll.set_offset(point(px(0.0), px(0.0))); cx.notify(); return;
        }
        self.dismiss(window, cx);
        match &item.action {
            MenuAction::Dispatch(action) => window.dispatch_action(action(), cx),
            MenuAction::Emit(event) => cx.emit(event.clone()),
            MenuAction::Minimize => window.minimize_window(),
            MenuAction::Maximize => window_chrome::toggle_maximize(window),
            MenuAction::Submenu(_) | MenuAction::None => {},
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu else { return; };
        let active = self.submenu.unwrap_or(menu);
        let entries = self.items(active, window.is_maximized(), cx);
        let current = if self.submenu.is_some() { self.subselected } else { self.selected };
        match event.keystroke.key.as_str() {
            "escape" => {
                if self.submenu.take().is_some() { self.subselected = None; cx.notify(); }
                else { self.dismiss(window, cx); }
            }
            "up" | "down" | "home" | "end" => {
                let candidates: Vec<_> = entries.iter().enumerate().filter_map(|(ix, item)| (item.enabled && !item.separator).then_some(ix)).collect();
                if !candidates.is_empty() {
                    let pos = current.and_then(|value| candidates.iter().position(|ix| *ix == value));
                    let next = match event.keystroke.key.as_str() {
                        "home" => 0, "end" => candidates.len() - 1,
                        "up" => pos.map_or(candidates.len() - 1, |pos| (pos + candidates.len() - 1) % candidates.len()),
                        _ => pos.map_or(0, |pos| (pos + 1) % candidates.len()),
                    };
                    if self.submenu.is_some() { self.subselected = Some(candidates[next]); self.submenu_scroll.scroll_to_item(candidates[next]); }
                    else { self.selected = Some(candidates[next]); self.menu_scroll.scroll_to_item(candidates[next]); }
                    cx.notify();
                }
            }
            "enter" | "space" => { if let Some(item) = current.and_then(|index| entries.get(index)) { self.run(item, window, cx); } }
            "right" => {
                if let Some(item) = current.and_then(|index| entries.get(index)).filter(|item| matches!(item.action, MenuAction::Submenu(_))) {
                    self.run(item, window, cx);
                } else if self.submenu.is_none() && menu != Menu::Window {
                    self.open([Menu::File, Menu::Edit, Menu::Paragraph, Menu::Format, Menu::View][(menu.index() + 1) % 5], window, cx);
                }
            }
            "left" => {
                if self.submenu.take().is_some() { self.subselected = None; cx.notify(); }
                else if menu != Menu::Window { self.open([Menu::File, Menu::Edit, Menu::Paragraph, Menu::Format, Menu::View][(menu.index() + 4) % 5], window, cx); }
            }
            "tab" => self.dismiss(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn menu_panel(&self, menu: Menu, nested: bool, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let locale = ToolbarLocale::current(cx);
        let theme = Theme::ui(cx);
        let compact = window.viewport_size().height <= px(700.0);
        let selected = if nested { self.subselected } else { self.selected };
        let entries = self.items(menu, window.is_maximized(), cx);
        let check_gutter = entries.iter().any(|entry| entry.checked.is_some());
        let mut rows = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.separator {
                rows.push(div().h(px(1.0)).my(px(4.0)).bg(theme.border).flex_none().into_any_element()); continue;
            }
            let submenu = match entry.action { MenuAction::Submenu(menu) => Some(menu), _ => None };
            let highlighted = selected == Some(index);
            let enabled = entry.enabled;
            let clicked = entry.clone();
            let color = if !enabled { theme.muted } else if highlighted { theme.on_accent } else { theme.fg };
            let mut row = div().id(("chrome-menu-item", index)).relative().flex().items_center().gap(px(8.0))
                .min_h(px(if compact { 27.0 } else { 31.0 })).px(px(16.0)).py(px(if compact { 4.0 } else { 6.0 }))
                .flex_none().text_size(px(13.0)).line_height(px(19.0)).text_color(color)
                .when(highlighted && enabled, |el| el.bg(theme.accent_fill))
                .when(enabled, |el| el.cursor_pointer())
                .when(!enabled, |el| el.opacity(0.55).tooltip(tooltip(locale.app_text("chrome.currentlyUnavailable"))))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.run(&clicked, window, cx); }))
                .on_hover(cx.listener(move |this, hover, _, cx| {
                    if *hover {
                        if nested { this.subselected = Some(index); }
                        else {
                            this.selected = Some(index);
                            if this.submenu != submenu { this.submenu = submenu; this.subselected = None; this.submenu_scroll.set_offset(point(px(0.0), px(0.0))); }
                        }
                        cx.notify();
                    }
                }));
            if let Some(name) = entry.icon { row = row.child(icon(name, 14.0, color)); }
            if check_gutter {
                row = row.child(div().size(px(12.0)).flex_none()
                    .children((entry.checked == Some(true)).then(|| icon(IconName::Check, 12.0, color))));
            }
            row = row.child(div().flex_1().min_w(px(0.0)).child(entry.label))
                .when(!entry.shortcut.is_empty(), |el| el.child(div().flex_none().ml(px(16.0)).text_size(px(12.0)).text_color(if highlighted && enabled { theme.on_accent } else { theme.muted }).child(entry.shortcut)));
            if submenu.is_some() { row = row.child(icon(IconName::ChevronRight, 10.0, color)); }
            if let Some(path) = entry.remove_recent {
                let removable = self.capable(ChromeCapability::Recent);
                row = row.child(div().id(("chrome-remove-recent", index)).size(px(18.0)).flex().items_center().justify_center()
                    .when(removable, |el| el.cursor_pointer()).child(icon(IconName::Close, 12.0, color))
                    .tooltip(tooltip(locale.app_text("chrome.removeRecent")))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); if removable { cx.emit(ChromeEvent::RemoveRecent(path.clone())); } })));
            }
            if let Some(submenu) = submenu.filter(|value| self.submenu == Some(*value)) {
                let panel = self.menu_panel(submenu, true, window, cx);
                row = row.child(deferred(popup(("chrome-submenu-enter", submenu as usize),
                    PopupAnchor::Parent, PopupPlacement::Right, panel, reduced_motion(cx))).with_priority(3));
            }
            rows.push(row.into_any_element());
        }
        div().id(if nested { "chrome-submenu" } else { "chrome-menu" }).w(px(if locale.is_chinese() { MENU_WIDTH } else { 360.0 }))
            .max_h((window.viewport_size().height - px(16.0)).max(px(0.0))).py(px(4.0))
            .flex().flex_col().rounded(px(theme.radius_md)).bg(theme.dropdown_bg).border_1().border_color(theme.dropdown_border)
            .shadow(vec![theme.floating_shadow.clone()]).occlude().overflow_y_scroll()
            .track_scroll(if nested { &self.submenu_scroll } else { &self.menu_scroll })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(rows)
    }

    fn basic_button(&self, id: &'static str, theme: &Theme) -> Stateful<Div> {
        div().id(id).size(px(32.0)).flex_none().flex().items_center().justify_center().rounded(px(4.0))
            .cursor_pointer().text_color(theme.fg).hover(|style| style.bg(gpui::rgba(0x80808026)))
            .when(cfg!(target_os = "windows"), |el| el.occlude())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
    }

    fn icon_button(&self, id: &'static str, name: IconName, theme: &Theme) -> Stateful<Div> {
        let size = match name { IconName::PanelLeft => 16.0, IconName::Moon | IconName::Sun => 14.0, _ => 15.0 };
        self.basic_button(id, theme).child(icon(name, size, theme.fg))
    }
}

impl Render for Chrome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = ToolbarLocale::current(cx);
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let viewport = window.viewport_size();
        let mut root = div().id("nomo-chrome").track_focus(&self.focus).key_context("NomoChrome")
            .relative().children(animate_theme.then(crate::ui::motion::theme_frame))
            .h(px(TITLEBAR_HEIGHT)).w_full().flex_none().flex().items_center().gap(px(6.0)).pl(px(8.0))
            .bg(theme.titlebar_bg).text_color(theme.titlebar_fg).text_size(px(13.0)).occlude()
            .when(cfg!(target_os = "macos") && !window.is_fullscreen(), |el| el.pl(px(88.0)))
            .on_key_down(cx.listener(Self::on_key))
            .when(!cfg!(target_os = "windows"), |el| el.on_mouse_down(MouseButton::Left, cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                this.dismiss(window, cx);
                if event.click_count == 2 { window.titlebar_double_click(); } else { window.start_window_move(); }
            })))
            .when(!cfg!(target_os = "windows"), |el| el.on_mouse_down(MouseButton::Right, cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                this.open(Menu::Window, window, cx); this.context_position = Some(event.position); cx.stop_propagation();
            })));
        #[cfg(target_os = "windows")]
        {
            root = root.child(window_chrome::drag_surface("chrome-drag")
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.dismiss(window, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open(Menu::Window, window, cx); this.context_position = Some(event.position); cx.stop_propagation();
                })));
        }
        root = root.child(self.icon_button("chrome-sidebar", IconName::PanelLeft, &theme)
                .tooltip(tooltip(if self.state.view_menu.sidebar_visible { locale.app_text("chrome.hideSidebar") } else { locale.app_text("chrome.showSidebar") }))
                .on_click(cx.listener(|this, _, window, cx| { this.dismiss(window, cx); cx.emit(ChromeEvent::ToggleSidebar); })));
        if !cfg!(target_os = "macos") {
            root = root.child(div().text_size(px(14.0)).font_weight(FontWeight(650.0)).opacity(0.9).mr(px(6.0)).child("Nomo"));
            let mut nav = div().flex().items_center().gap(px(1.0)).flex_none();
            for menu in [Menu::File, Menu::Edit, Menu::Paragraph, Menu::Format, Menu::View] {
                let open = self.menu == Some(menu);
                let trigger = div().id(("chrome-menu-trigger", menu.index())).h(px(32.0)).px(px(8.0))
                    .flex().items_center().rounded(px(4.0)).cursor_pointer()
                    .when(cfg!(target_os = "windows"), |el| el.occlude())
                    .when(open, |el| el.bg(gpui::rgba(0x80808026))).hover(|style| style.bg(gpui::rgba(0x80808026))).child(menu.label(locale))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.menu == Some(menu) { this.dismiss(window, cx); } else { this.open(menu, window, cx); }
                    }))
                    .on_hover(cx.listener(move |this, hover, window, cx| {
                        if *hover && this.menu.is_some() && this.menu != Some(menu) { this.open(menu, window, cx); }
                    }));
                let mut item = div().relative().flex().flex_col().child(trigger);
                if open {
                    let panel = self.menu_panel(menu, false, window, cx);
                    item = item.child(deferred(popup(("chrome-menu-enter", menu.index()),
                        PopupAnchor::Parent, PopupPlacement::BelowLeft, panel, reduced_motion(cx))).with_priority(2));
                }
                nav = nav.child(item);
            }
            root = root.child(nav.child(div().id("chrome-settings").h(px(32.0)).px(px(8.0)).flex().items_center()
                .rounded(px(4.0)).cursor_pointer().hover(|style| style.bg(theme.hover)).child(locale.app_text("chrome.settings"))
                .when(cfg!(target_os = "windows"), |el| el.occlude())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| { this.dismiss(window, cx); window.dispatch_action(Box::new(action::OpenSettings), cx); }))));
        }
        root = root.child(div().flex_1().min_w(px(0.0)).h_full().flex().items_center().justify_center()
            .children(self.state.document_title.clone().map(|(title, dirty)| div().flex().items_center().gap(px(6.0)).min_w(px(0.0))
                .text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(theme.fg)
                .child(div().min_w(px(0.0)).truncate().child(title))
                .children(dirty.then(|| div().size(px(6.0)).flex_none().rounded_full().bg(theme.accent))))));
        let mut right = div().flex().items_center().h_full().flex_none();
        if let Some(update) = self.state.update {
            let enabled = self.capable(ChromeCapability::SoftwareUpdate);
            right = right.child(self.icon_button("chrome-update", IconName::ArrowDown, &theme).relative()
                .when(!enabled, |el| el.opacity(0.5)).tooltip(tooltip(locale.app_text("chrome.softwareUpdate")))
                .child(div().absolute().right_0().top_0().text_size(px(8.0)).text_color(theme.accent)
                    .child(match update { ChromeUpdateState::Available => "●".to_owned(), ChromeUpdateState::Downloading(percent) => percent.min(100).to_string(), ChromeUpdateState::Downloaded => "✓".to_owned() }))
                .on_click(cx.listener(move |this, _, window, cx| { if enabled { this.dismiss(window, cx); cx.emit(ChromeEvent::SoftwareUpdate); } })));
        }
        if self.state.view_menu.mode.is_some() {
            let mini_enabled = self.capable(ChromeCapability::Mini);
            right = right.child(self.basic_button("chrome-mini", &theme).child(icon(IconName::PictureInPicture2, 15.0, theme.fg))
                .when(!mini_enabled, |el| el.opacity(0.45)).tooltip(tooltip(if mini_enabled { "Markdown Mini (Ctrl+Alt+M)" } else { super::mini::unavailable_reason_key(window).map(|key| locale.app_text(key)).unwrap_or(locale.app_text("chrome.miniBusy")) }))
                .on_click(cx.listener(move |this, _, window, cx| { if mini_enabled { this.dismiss(window, cx); cx.emit(ChromeEvent::ToggleMini); } })));
        }
        right = right.child(self.icon_button("chrome-theme", if theme.dark { IconName::Sun } else { IconName::Moon }, &theme)
                .tooltip(tooltip(locale.app_text("chrome.switchTheme")))
                .on_click(cx.listener(|this, _, window, cx| { this.dismiss(window, cx); window.dispatch_action(Box::new(action::ToggleTheme), cx); })));
        root = root.child(right);
        #[cfg(target_os = "windows")]
        {
            use window_chrome::CaptionAction;
            let on_action = cx.listener(|this, action: &CaptionAction, window, cx| {
                this.dismiss(window, cx);
                match action {
                    CaptionAction::Minimize => window.minimize_window(),
                    CaptionAction::Maximize => window_chrome::toggle_maximize(window),
                    CaptionAction::Close => cx.emit(ChromeEvent::CloseWindow),
                }
            });
            root = root.child(window_chrome::caption_controls(&theme, window, &self.caption_focus, move |action, window, cx| on_action(&action, window, cx)));
        }
        if self.menu.is_some() {
            root = root.child(deferred(anchored().position(point(px(0.0), px(TITLEBAR_HEIGHT))).child(
                div().id("chrome-dismiss-overlay").w(viewport.width).h((viewport.height - px(TITLEBAR_HEIGHT)).max(px(0.0))).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| { cx.stop_propagation(); this.dismiss(window, cx); }))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| { cx.stop_propagation(); this.dismiss(window, cx); }))
            )).with_priority(1));
        }
        if let Some(position) = self.context_position.filter(|_| self.menu == Some(Menu::Window)) {
            let panel = self.menu_panel(Menu::Window, false, window, cx);
            root = root.child(deferred(popup("chrome-window-menu-enter",
                PopupAnchor::Bounds(gpui::Bounds::new(position, gpui::size(px(0.0), px(0.0)))),
                PopupPlacement::BelowLeft, panel, reduced_motion(cx))).with_priority(2));
        }
        #[cfg(target_os = "windows")]
        if !window.is_maximized() && !window.is_fullscreen() {
            root = root.child(window_chrome::top_resize_guard(window));
        }
        root
    }
}
