//! 正式窗口标题栏，页面基准为 master 339fb932 的 AppTitleBar。
//!
//! 只保存菜单、焦点及宿主提供的轻量状态；不持有文档、正文或撤销历史。
//! 宿主必须先连接事件，再声明相应 capability。未连接的主分支功能保留可见的禁用项。

use super::components::{IconName, icon, tooltip};
use super::{window_chrome, workspace as action};
use crate::{editor, settings::EditorMode, theme::Theme};
use gpui::{
    Action, AnchoredPositionMode, AnyElement, App, Context, Div, EventEmitter, FocusHandle,
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
    ResetOutlinePosition, Mini, SoftwareUpdate, UnavailableNotice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramKind { Blank, Flowchart, Sequence, Class, State, Pie, Gantt, EntityRelationship }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat { Html, Pdf, Word }

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
    ResetOutlinePosition,
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
    pub has_document: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub mode: EditorMode,
    pub sidebar_visible: bool,
    pub toolbar_visible: bool,
    pub outline_visible: bool,
    pub large_document: bool,
    pub split_available: bool,
    pub word_export_busy: bool,
    pub toolbar_shortcut: SharedString,
    pub recent: Vec<ChromeRecentEntry>,
    pub capabilities: Vec<ChromeCapability>,
    pub update: Option<ChromeUpdateState>,
}

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            has_document: false, can_undo: false, can_redo: false, mode: EditorMode::Semantic,
            sidebar_visible: true, toolbar_visible: true, outline_visible: false,
            large_document: false, split_available: true, word_export_busy: false,
            toolbar_shortcut: "".into(), recent: Vec::new(), capabilities: Vec::new(), update: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu { File, Edit, Paragraph, Format, View, Recent, Export, Heading, Diagram, Window }

impl Menu {
    fn label(self) -> &'static str {
        match self {
            Self::File => "文件", Self::Edit => "编辑", Self::Paragraph => "段落",
            Self::Format => "格式", Self::View => "查看", Self::Recent => "打开最近",
            Self::Export => "导出", Self::Heading => "标题", Self::Diagram => "图表",
            Self::Window => "窗口",
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
    checked: bool,
    separator: bool,
    icon: Option<IconName>,
    remove_recent: Option<PathBuf>,
}

impl MenuItem {
    fn new(label: impl Into<SharedString>, shortcut: impl Into<SharedString>, action: MenuAction, enabled: bool) -> Self {
        Self { label: label.into(), shortcut: shortcut.into(), action, enabled, checked: false, separator: false, icon: None, remove_recent: None }
    }
    fn separator() -> Self {
        Self { separator: true, ..Self::new("", "", MenuAction::None, false) }
    }
    fn submenu(menu: Menu) -> Self { Self::new(menu.label(), "", MenuAction::Submenu(menu), true) }
    fn checked(mut self, checked: bool) -> Self { self.checked = checked; self }
}

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
            caption_focus: window_chrome::CaptionFocus::new(cx),
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
        MenuItem::new(label, shortcut, MenuAction::Emit(event), self.capable(capability) && (!needs_document || self.state.has_document))
    }

    fn items(&self, menu: Menu, maximized: bool) -> Vec<MenuItem> {
        use ChromeCapability as C;
        use ChromeCommand as Cmd;
        use ChromeEvent as E;
        use MenuAction as A;
        let doc = self.state.has_document;
        let separator = MenuItem::separator;
        let action = |label: &'static str, shortcut: &'static str, action: fn() -> Box<dyn Action>, enabled: bool| MenuItem::new(label, shortcut, A::Dispatch(action), enabled);
        let event = |label, shortcut, value, capability, document| self.event_item(label, shortcut, value, capability, document);
        match menu {
            Menu::File => vec![
                action("新建 Markdown", "Ctrl + N", || Box::new(action::NewFile), true),
                event("新窗口", "Ctrl + Shift + N", E::NewWindow, C::NewWindow, false), separator(),
                action("打开文件...", "Ctrl + O", || Box::new(action::OpenFile), true),
                action("打开文件夹...", "Ctrl + Shift + O", || Box::new(action::OpenFolder), true),
                MenuItem::submenu(Menu::Recent), separator(),
                action("保存", "Ctrl + S", || Box::new(action::Save), doc),
                action("另存为...", "Ctrl + Shift + S", || Box::new(action::SaveAs), doc), separator(),
                MenuItem::submenu(Menu::Export), separator(),
                action("关闭当前文件", "Ctrl + W", || Box::new(action::CloseTab), doc),
                MenuItem::new("关闭窗口", "Alt + F4", A::Emit(E::CloseWindow), true), separator(),
                MenuItem::new("退出", "", A::Emit(E::Quit), true),
            ],
            Menu::Edit => vec![
                action("撤销", "Ctrl + Z", || Box::new(editor::Undo), doc && self.state.can_undo),
                action("重做", "Ctrl + Y", || Box::new(editor::Redo), doc && self.state.can_redo),
            ],
            Menu::Paragraph => vec![
                MenuItem::submenu(Menu::Heading),
                action("段落", "Ctrl + 0", || Box::new(action::Paragraph), doc),
                event("提升标题", "Ctrl + =", E::Command(Cmd::AdjustHeading { increase: true }), C::HeadingAdjustment, true),
                event("降低标题", "Ctrl + -", E::Command(Cmd::AdjustHeading { increase: false }), C::HeadingAdjustment, true), separator(),
                event("表格", "Ctrl + Shift + T", E::TablePicker, C::TablePicker, true),
                action("代码块", "Ctrl + Shift + K", || Box::new(action::CodeBlock), doc),
                action("行间公式", "Ctrl + Shift + M", || Box::new(action::MathBlock), doc),
                MenuItem::new("行内公式", "", A::Emit(E::Command(Cmd::MathInline)), doc && self.capable(C::MathInline) && self.state.mode != EditorMode::Source), separator(),
                action("引用", "Ctrl + Shift + Q", || Box::new(action::Quote), doc),
                event("提示块", "Ctrl + Shift + A", E::Command(Cmd::Callout), C::Callout, true),
                event("折叠块", "", E::Command(Cmd::Details), C::Details, true),
                event("行内注释", "", E::Command(Cmd::BlockComment), C::BlockComment, true),
                action("有序列表", "Ctrl + Shift + [", || Box::new(action::OrderedList), doc),
                action("无序列表", "Ctrl + Shift + ]", || Box::new(action::BulletList), doc),
                action("任务列表", "Ctrl + Shift + X", || Box::new(action::TaskList), doc), separator(),
                event("上插段落", "Ctrl + Shift + Enter", E::Unavailable(UnavailableFeature::ParagraphBefore), C::UnavailableNotice, true),
                event("下插段落", "Ctrl + Enter", E::Unavailable(UnavailableFeature::ParagraphAfter), C::UnavailableNotice, true), separator(),
                MenuItem::submenu(Menu::Diagram),
                event("脚注", "", E::Command(Cmd::Footnote), C::Footnote, true),
                action("水平分割线", "Ctrl + Shift + H", || Box::new(action::Rule), doc),
                event("目录", "", E::Command(Cmd::Toc), C::Toc, true),
                event("文档元数据", "", E::FrontMatter, C::FrontMatter, true),
            ],
            Menu::Format => vec![
                action("加粗", "Ctrl + B", || Box::new(action::Bold), doc),
                action("斜体", "Ctrl + I", || Box::new(action::Italic), doc),
                action("下划线", "Ctrl + U", || Box::new(action::Underline), doc),
                action("行代码", "Ctrl + `", || Box::new(action::InlineCode), doc),
                event("行公式", "", E::Unavailable(UnavailableFeature::InlineMath), C::UnavailableNotice, true), separator(),
                action("删除线", "Alt + Shift + 5", || Box::new(action::Strike), doc),
                action("高亮", "", || Box::new(action::Highlight), doc),
                event("行内注释", "", E::Command(Cmd::InlineComment), C::InlineComment, true), separator(),
                event("链接", "Ctrl + K", E::LinkPicker, C::LinkPicker, true),
                event("图像", "", E::Unavailable(UnavailableFeature::Image), C::UnavailableNotice, true), separator(),
                event("清除样式", r"Ctrl + \", E::Command(Cmd::ClearFormatting), C::ClearFormatting, true),
            ],
            Menu::View => vec![
                action("语义编辑", "", || Box::new(action::ModeSemantic), doc && !self.state.large_document).checked(self.state.mode == EditorMode::Semantic),
                action("源码模式", "Ctrl + E", || Box::new(action::ModeSource), doc).checked(self.state.mode == EditorMode::Source),
                action("双栏模式", "", || Box::new(action::ModeSplit), doc && !self.state.large_document && self.state.split_available).checked(self.state.mode == EditorMode::Split),
                action(if self.state.outline_visible { "隐藏文档大纲" } else { "显示文档大纲" }, "", || Box::new(action::ToggleOutline), doc),
                event("恢复大纲默认位置", "", E::ResetOutlinePosition, C::ResetOutlinePosition, false),
                MenuItem::new(if self.state.toolbar_visible { "隐藏工具栏" } else { "显示工具栏" }, self.state.toolbar_shortcut.clone(), A::Dispatch(|| Box::new(action::ToggleToolbar)), true),
                action("切换主题", "Ctrl + Shift + L", || Box::new(action::ToggleTheme), true),
                action("显示 / 隐藏资源管理器", "Ctrl + Shift + F", || Box::new(action::ToggleSidebar), true),
            ],
            Menu::Heading => vec![
                action("一级标题", "Ctrl + 1", || Box::new(action::Heading1), doc),
                action("二级标题", "Ctrl + 2", || Box::new(action::Heading2), doc),
                action("三级标题", "Ctrl + 3", || Box::new(action::Heading3), doc),
                event("四级标题", "Ctrl + 4", E::Command(Cmd::Heading(4)), C::HeadingLevels, true),
                event("五级标题", "Ctrl + 5", E::Command(Cmd::Heading(5)), C::HeadingLevels, true),
                event("六级标题", "Ctrl + 6", E::Command(Cmd::Heading(6)), C::HeadingLevels, true),
            ],
            Menu::Export => vec![
                event("导出 HTML", "", E::Export(ExportFormat::Html), C::ExportHtml, true),
                event("导出 PDF", "", E::Export(ExportFormat::Pdf), C::ExportPdf, true),
                MenuItem::new("导出 Word", "", A::Emit(E::Export(ExportFormat::Word)), doc && self.capable(C::ExportWord) && !self.state.word_export_busy),
            ],
            Menu::Diagram => {
                let mut entries = vec![event("空白图表", "mermaid", E::Command(Cmd::Diagram(DiagramKind::Blank)), C::Diagrams, true), separator()];
                for (label, shortcut, kind) in [
                    ("流程图", "flowchart", DiagramKind::Flowchart), ("时序图", "sequenceDiagram", DiagramKind::Sequence),
                    ("类图", "classDiagram", DiagramKind::Class), ("状态图", "stateDiagram", DiagramKind::State),
                    ("饼图", "pie", DiagramKind::Pie), ("甘特图", "gantt", DiagramKind::Gantt),
                    ("实体关系图", "erDiagram", DiagramKind::EntityRelationship),
                ] { entries.push(event(label, shortcut, E::Command(Cmd::Diagram(kind)), C::Diagrams, true)); }
                entries
            }
            Menu::Recent => {
                let mut entries: Vec<_> = self.state.recent.iter().take(10).map(|recent| {
                    let mut entry = MenuItem::new(recent.label.clone(), if recent.missing { "不可用" } else { "" }, A::Emit(E::OpenRecent { path: recent.path.clone(), folder: recent.folder }), self.capable(C::Recent) && !recent.missing);
                    entry.icon = Some(if recent.folder { IconName::Folder } else { IconName::FileText });
                    entry.remove_recent = Some(recent.path.clone());
                    entry
                }).collect();
                if entries.is_empty() { entries.push(MenuItem::new("无最近打开的记录", "", A::None, false)); }
                else { entries.push(separator()); entries.push(event("清除最近打开", "", E::ClearRecent, C::Recent, false)); }
                entries
            }
            Menu::Window => vec![
                MenuItem::new("最小化", "", A::Minimize, true),
                MenuItem::new(if maximized { "还原" } else { "最大化" }, "", A::Maximize, true), separator(),
                MenuItem::new("关闭窗口", "Alt + F4", A::Emit(E::CloseWindow), true),
            ],
        }
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
        let entries = self.items(active, window.is_maximized());
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

    fn menu_panel(&self, menu: Menu, nested: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::ui(cx);
        let compact = window.viewport_size().height <= px(700.0);
        let selected = if nested { self.subselected } else { self.selected };
        let entries = self.items(menu, window.is_maximized());
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
                .when(highlighted && enabled, |el| el.bg(theme.accent))
                .when(enabled, |el| el.cursor_pointer())
                .when(!enabled, |el| el.opacity(0.55).tooltip(tooltip("当前不可用")))
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
            if entry.checked { row = row.child(icon(IconName::Check, 12.0, color)); }
            row = row.child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().child(entry.label))
                .when(!entry.shortcut.is_empty(), |el| el.child(div().flex_none().ml(px(16.0)).text_size(px(12.0)).text_color(if highlighted && enabled { theme.on_accent } else { theme.muted }).child(entry.shortcut)));
            if submenu.is_some() { row = row.child(icon(IconName::ChevronRight, 10.0, color)); }
            if let Some(path) = entry.remove_recent {
                let removable = self.capable(ChromeCapability::Recent);
                row = row.child(div().id(("chrome-remove-recent", index)).size(px(18.0)).flex().items_center().justify_center()
                    .when(removable, |el| el.cursor_pointer()).child(icon(IconName::Close, 12.0, color))
                    .tooltip(tooltip("从最近记录移除"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); if removable { cx.emit(ChromeEvent::RemoveRecent(path.clone())); } })));
            }
            if let Some(submenu) = submenu.filter(|value| self.submenu == Some(*value)) {
                let parent_x = (101.0 + self.menu.unwrap_or(Menu::File).index() as f32 * 44.0).min(f32::from(window.viewport_size().width) - MENU_WIDTH - 8.0);
                let right = parent_x + MENU_WIDTH * 2.0 + 2.0 <= f32::from(window.viewport_size().width) - 8.0;
                let panel = self.menu_panel(submenu, true, window, cx);
                row = row.child(deferred(anchored().position_mode(AnchoredPositionMode::Local)
                    .position(point(px(if right { MENU_WIDTH - 16.0 + 2.0 } else { -MENU_WIDTH - 18.0 }), px(-4.0)))
                    .snap_to_window_with_margin(px(8.0)).child(panel)).with_priority(3));
            }
            rows.push(row.into_any_element());
        }
        div().id(if nested { "chrome-submenu" } else { "chrome-menu" }).w(px(MENU_WIDTH))
            .max_h((window.viewport_size().height - px(16.0)).max(px(0.0))).py(px(4.0))
            .flex().flex_col().rounded(px(8.0)).bg(theme.surface).border_1().border_color(theme.border)
            .shadow_lg().occlude().overflow_y_scroll()
            .track_scroll(if nested { &self.submenu_scroll } else { &self.menu_scroll })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(rows).into_any_element()
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
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let viewport = window.viewport_size();
        let mut root = div().id("nomo-chrome").track_focus(&self.focus).key_context("NomoChrome")
            .relative().children(animate_theme.then(crate::ui::motion::theme_frame))
            .h(px(TITLEBAR_HEIGHT)).w_full().flex_none().flex().items_center().gap(px(6.0)).pl(px(8.0))
            .bg(theme.rail_bg).text_color(theme.fg).text_size(px(13.0)).occlude()
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
                .tooltip(tooltip(if self.state.sidebar_visible { "隐藏文件栏" } else { "显示文件栏" }))
                .on_click(cx.listener(|this, _, window, cx| { this.dismiss(window, cx); window.dispatch_action(Box::new(action::ToggleSidebar), cx); })));
        if !cfg!(target_os = "macos") {
            root = root.child(div().text_size(px(14.0)).font_weight(FontWeight(650.0)).opacity(0.9).mr(px(6.0)).child("Nomo"));
            let mut nav = div().flex().items_center().gap(px(1.0)).flex_none();
            for menu in [Menu::File, Menu::Edit, Menu::Paragraph, Menu::Format, Menu::View] {
                let open = self.menu == Some(menu);
                let trigger = div().id(("chrome-menu-trigger", menu.index())).h(px(32.0)).px(px(8.0))
                    .flex().items_center().rounded(px(4.0)).cursor_pointer()
                    .when(cfg!(target_os = "windows"), |el| el.occlude())
                    .when(open, |el| el.bg(gpui::rgba(0x80808026))).hover(|style| style.bg(gpui::rgba(0x80808026))).child(menu.label())
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
                    item = item.child(deferred(anchored().offset(point(px(0.0), px(4.0))).snap_to_window_with_margin(px(8.0)).child(panel)).with_priority(2));
                }
                nav = nav.child(item);
            }
            root = root.child(nav.child(div().id("chrome-settings").h(px(32.0)).px(px(8.0)).flex().items_center()
                .rounded(px(4.0)).cursor_pointer().hover(|style| style.bg(theme.hover)).child("设置")
                .when(cfg!(target_os = "windows"), |el| el.occlude())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| { this.dismiss(window, cx); window.dispatch_action(Box::new(action::OpenSettings), cx); }))));
        }
        root = root.child(div().flex_1().h_full());
        let mut right = div().flex().items_center().h_full().flex_none();
        if let Some(update) = self.state.update {
            let enabled = self.capable(ChromeCapability::SoftwareUpdate);
            right = right.child(self.icon_button("chrome-update", IconName::ArrowDown, &theme).relative()
                .when(!enabled, |el| el.opacity(0.5)).tooltip(tooltip("软件更新"))
                .child(div().absolute().right_0().top_0().text_size(px(8.0)).text_color(theme.accent)
                    .child(match update { ChromeUpdateState::Available => "●".to_owned(), ChromeUpdateState::Downloading(percent) => percent.min(100).to_string(), ChromeUpdateState::Downloaded => "✓".to_owned() }))
                .on_click(cx.listener(move |this, _, window, cx| { if enabled { this.dismiss(window, cx); cx.emit(ChromeEvent::SoftwareUpdate); } })));
        }
        if self.state.has_document {
            let mini_enabled = self.capable(ChromeCapability::Mini);
            right = right.child(self.basic_button("chrome-mini", &theme).child(icon(IconName::PictureInPicture2, 15.0, theme.fg))
                .when(!mini_enabled, |el| el.opacity(0.45)).tooltip(tooltip(if mini_enabled { "Markdown Mini" } else { "Markdown Mini 尚未接入" }))
                .on_click(cx.listener(move |this, _, window, cx| { if mini_enabled { this.dismiss(window, cx); cx.emit(ChromeEvent::ToggleMini); } })));
        }
        right = right.child(self.icon_button("chrome-theme", if theme.dark { IconName::Sun } else { IconName::Moon }, &theme)
                .tooltip(tooltip("切换主题"))
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
            root = root.child(deferred(anchored().position(position).snap_to_window_with_margin(px(8.0)).child(panel)).with_priority(2));
        }
        #[cfg(target_os = "windows")]
        if !window.is_maximized() && !window.is_fullscreen() {
            root = root.child(window_chrome::top_resize_guard(window));
        }
        root
    }
}
