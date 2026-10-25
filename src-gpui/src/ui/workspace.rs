//! 窗口根视图：标题栏标签、侧栏文件树、工具栏、编辑区（语义 / 源码 / 分栏）、大纲浮卡、搜索面板与状态栏。
//!
//! 每个标签持有一个 `Buffer` 和两个视图（语义、源码）。单栏模式只渲染当前模式的视图，
//! 另一个视图不参与布局与绘制；分栏时两侧共享 Buffer，滚动通过行号互相同步。

use crate::buffer::{Buffer, BufferEvent};
use crate::editor::{EditorView, ViewEvent, ViewMode};
use crate::markdown::blocks::Heading;
use crate::settings::{EditorMode, Settings, StatsMetric};
use crate::theme::Theme;
use crate::ui::components::{ButtonKind, IconName, button, floating_card, icon, icon_button, segment, segmented, slider, small_icon_button, tooltip, vertical_divider};
use crate::ui::file_tree::FileTree;
use crate::ui::text_field::{FieldEvent, TextField};
use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, PathPromptOptions, PromptLevel, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, actions, div, prelude::*, px,
    uniform_list,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;

actions!(
    workspace,
    [
        NewFile, OpenFile, OpenFolder, Save, SaveAs, CloseTab, NextTab, PrevTab, Find, FindReplace, FindNext,
        FindPrev, ToggleSource, ModeSemantic, ModeSource, ModeSplit, ToggleOutline, ToggleSidebar, ToggleToolbar,
        ToggleTheme, FollowSystemTheme, ToggleTypography, Underline, Highlight, ZoomIn, ZoomOut, ZoomReset, Bold, Italic, Strike, InlineCode, Link,
        Heading1, Heading2, Heading3, Paragraph, Quote, BulletList, OrderedList, TaskList, CodeBlock, Table,
        MathBlock, Rule, OpenSettings, Quit
    ]
);

pub(crate) struct Tab {
    pub(crate) buffer: Entity<Buffer>,
    pub(crate) semantic: Entity<EditorView>,
    pub(crate) source: Entity<EditorView>,
    pub(crate) mode: EditorMode,
    /// 主视图顶部所在源码行，用于大纲高亮当前标题。
    top_line: f64,
    /// 自动保存的延迟任务；再次编辑时被替换（即取消上一次），保证只在停止输入后保存。
    autosave: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    pub(crate) fn primary(&self) -> &Entity<EditorView> {
        match self.mode {
            EditorMode::Source => &self.source,
            _ => &self.semantic,
        }
    }
}

pub(crate) struct SearchPanel {
    pub(crate) query: Entity<TextField>,
    pub(crate) replacement: Entity<TextField>,
    show_replace: bool,
    case_sensitive: bool,
    _subscriptions: Vec<Subscription>,
}

pub struct Workspace {
    focus: FocusHandle,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    tree: Option<FileTree>,
    pub(crate) search: Option<SearchPanel>,
    outline_scroll: UniformListScrollHandle,
    tree_scroll: UniformListScrollHandle,
    notice: Option<SharedString>,
    /// 工具栏内容宽度滑杆是否正在拖动。
    width_dragging: bool,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Workspace {
    pub fn new(paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut workspace = Self {
            focus: cx.focus_handle(),
            tabs: Vec::new(),
            active: 0,
            tree: None,
            search: None,
            outline_scroll: UniformListScrollHandle::new(),
            tree_scroll: UniformListScrollHandle::new(),
            notice: None,
            width_dragging: false,
        };
        for path in paths {
            workspace.open_path(&path, window, cx);
        }
        workspace
    }

    pub(crate) fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn buffer(&self) -> Option<Entity<Buffer>> {
        self.tab().map(|tab| tab.buffer.clone())
    }

    // ---- 标签 ----

    pub fn open_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.tree = Some(FileTree::new(path.to_path_buf()));
            cx.notify();
            return;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(index) = self.tabs.iter().position(|tab| tab.buffer.read(cx).path.as_deref() == Some(canonical.as_path())) {
            self.activate(index, window, cx);
            return;
        }
        match Buffer::open(&canonical) {
            Ok(buffer) => {
                if self.tree.is_none() {
                    if let Some(parent) = canonical.parent() {
                        self.tree = Some(FileTree::new(parent.to_path_buf()));
                    }
                }
                if let Some(reason) = &buffer.read_only {
                    self.notice = Some(reason.clone().into());
                }
                self.add_tab(buffer, window, cx);
            }
            Err(error) => self.notice = Some(format!("无法打开 {}：{error}", canonical.display()).into()),
        }
        cx.notify();
    }

    fn add_tab(&mut self, buffer: Buffer, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = cx.new(|_| buffer);
        let semantic = cx.new(|cx| EditorView::new(buffer.clone(), ViewMode::Semantic, window, cx));
        let source = cx.new(|cx| EditorView::new(buffer.clone(), ViewMode::Source, window, cx));
        let mode = cx.global::<Settings>().default_mode;
        let subscriptions = vec![
            cx.subscribe(&semantic, |workspace, view, event, cx| workspace.on_view_event(view, event, cx)),
            cx.subscribe(&source, |workspace, view, event, cx| workspace.on_view_event(view, event, cx)),
            cx.subscribe_in(&buffer, window, |workspace, buffer, event, window, cx| {
                if matches!(event, BufferEvent::Edited) {
                    workspace.schedule_autosave(buffer.clone(), window, cx);
                }
                if matches!(event, BufferEvent::Saved | BufferEvent::Edited | BufferEvent::SearchUpdated) {
                    cx.notify();
                }
            }),
            cx.observe(&buffer, |_, _, cx| cx.notify()),
        ];
        buffer.update(cx, |buffer, cx| buffer.ensure_stats(cx));
        self.tabs.push(Tab { buffer, semantic, source, mode, top_line: 0.0, autosave: None, _subscriptions: subscriptions });
        let index = self.tabs.len() - 1;
        self.activate(index, window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        self.apply_mode(window, cx);
        if let Some(tab) = self.tab() {
            let title = tab.buffer.read(cx).title();
            window.set_window_title(&format!("{title} — Nomo"));
            let primary = tab.primary().clone();
            window.focus(&primary.focus_handle(cx));
        }
        if let Some(search) = &self.search {
            let query = search.query.read(cx).content.clone();
            let case = search.case_sensitive;
            if let Some(buffer) = self.buffer() {
                buffer.update(cx, |buffer, cx| buffer.set_search(query, case, cx));
            }
        }
        cx.notify();
    }

    fn on_view_event(&mut self, view: Entity<EditorView>, event: &ViewEvent, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        match event {
            ViewEvent::Scrolled(line) => {
                if view == *tab.primary() || tab.mode == EditorMode::Split {
                    tab.top_line = *line;
                }
                // 分栏：用户滚动的一侧带动另一侧。
                if tab.mode == EditorMode::Split {
                    let other = if view == tab.semantic { tab.source.clone() } else { tab.semantic.clone() };
                    let line = *line;
                    other.update(cx, |other, cx| other.scroll_to_line(line, cx));
                }
                cx.notify();
            }
            ViewEvent::Focused => cx.notify(),
        }
    }

    pub(crate) fn set_mode(&mut self, mode: EditorMode, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.mode == mode {
            return;
        }
        let top = tab.primary().read(cx).top_line(cx);
        tab.mode = mode;
        let (semantic, source) = (tab.semantic.clone(), tab.source.clone());
        semantic.update(cx, |view, cx| view.scroll_to_line(top, cx));
        source.update(cx, |view, cx| view.scroll_to_line(top, cx));
        self.apply_mode(window, cx);
        let primary = self.tabs[self.active].primary().clone();
        window.focus(&primary.focus_handle(cx));
        cx.notify();
    }

    fn apply_mode(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab() else { return };
        let split = tab.mode == EditorMode::Split;
        for view in [tab.semantic.clone(), tab.source.clone()] {
            view.update(cx, |view, cx| {
                view.split = split;
                cx.notify();
            });
        }
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        let buffer = tab.buffer.clone();
        if buffer.read(cx).is_dirty() {
            let title = buffer.read(cx).title();
            let answer = window.prompt(PromptLevel::Warning, &format!("是否保存对“{title}”的修改？"), Some("不保存将丢失这些修改。"), &["保存", "不保存", "取消"], cx);
            cx.spawn_in(window, async move |this, cx| {
                let Ok(choice) = answer.await else { return };
                this.update_in(cx, |workspace, window, cx| match choice {
                    0 => {
                        if workspace.save_buffer(&buffer, false, window, cx) {
                            workspace.remove_tab(&buffer, window, cx);
                        }
                    }
                    1 => workspace.remove_tab(&buffer, window, cx),
                    _ => {}
                })
                .ok();
            })
            .detach();
            return;
        }
        self.remove_tab(&buffer, window, cx);
    }

    fn remove_tab(&mut self, buffer: &Entity<Buffer>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.buffer == *buffer) else { return };
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        if self.tabs.is_empty() {
            window.set_window_title("Nomo");
            window.focus(&self.focus);
            cx.notify();
        } else {
            self.activate(self.active, window, cx);
        }
    }

    // ---- 文件 ----

    fn prompt_open(&mut self, directories: bool, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions { files: !directories, directories, multiple: !directories, prompt: Some("打开".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else { return };
            this.update_in(cx, |workspace, window, cx| {
                for path in paths {
                    workspace.open_path(&path, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// 保存；无路径或 `save_as` 时弹出保存面板。返回是否已同步完成保存。
    fn save_buffer(&mut self, buffer: &Entity<Buffer>, save_as: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let path = buffer.read(cx).path.clone();
        match path.filter(|_| !save_as) {
            Some(path) => {
                let snapshot = cx.global::<Settings>().snapshot_before_save;
                let result = buffer.update(cx, |buffer, cx| buffer.save_to(path, snapshot, cx));
                match result {
                    Ok(()) => {
                        self.notice = None;
                        true
                    }
                    Err(error) => {
                        self.notice = Some(format!("保存失败：{error}").into());
                        cx.notify();
                        false
                    }
                }
            }
            None => {
                let directory = buffer.read(cx).base_dir().or_else(|| self.tree.as_ref().map(|tree| tree.root.clone())).or_else(dirs_home).unwrap_or_else(|| PathBuf::from("/"));
                let name = buffer.read(cx).title();
                let receiver = cx.prompt_for_new_path(&directory, Some(&name));
                let buffer = buffer.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Ok(Some(path))) = receiver.await else { return };
                    this.update_in(cx, |workspace, window, cx| {
                        let snapshot = cx.global::<Settings>().snapshot_before_save;
                        let result = buffer.update(cx, |buffer, cx| buffer.save_to(path, snapshot, cx));
                        if let Err(error) = result {
                            workspace.notice = Some(format!("保存失败：{error}").into());
                        } else {
                            let title = buffer.read(cx).title();
                            window.set_window_title(&format!("{title} — Nomo"));
                            if let Some(tree) = &mut workspace.tree {
                                tree.refresh();
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
                false
            }
        }
    }

    /// 设置“自动保存”开启且文件已有路径时，停止输入一段时间后保存；未命名文档不自动弹出保存面板。
    fn schedule_autosave(&mut self, buffer: Entity<Buffer>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(delay) = cx.global::<Settings>().autosave else { return };
        if buffer.read(cx).path.is_none() || buffer.read(cx).read_only.is_some() {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.buffer == buffer) else { return };
        let target = buffer.clone();
        tab.autosave = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update_in(cx, |workspace, window, cx| {
                if target.read(cx).is_dirty() {
                    workspace.save_buffer(&target, false, window, cx);
                }
            })
            .ok();
        }));
    }

    // ---- 搜索 ----

    pub(crate) fn open_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(buffer) = self.buffer() else { return };
        let selected = buffer.read(cx).selected_text();
        if self.search.is_none() {
            let query = cx.new(|cx| TextField::new("查找", cx));
            let replacement = cx.new(|cx| TextField::new("替换为", cx));
            let subscriptions = vec![
                cx.subscribe_in(&query, window, |workspace, _, event, window, cx| workspace.on_query_event(event, window, cx)),
                cx.subscribe_in(&replacement, window, |workspace, _, event, window, cx| match event {
                    FieldEvent::Submit { .. } => workspace.replace_one(window, cx),
                    FieldEvent::Cancel => workspace.close_search(window, cx),
                    FieldEvent::Changed => {}
                }),
            ];
            self.search = Some(SearchPanel { query, replacement, show_replace: replace, case_sensitive: false, _subscriptions: subscriptions });
        }
        let search = self.search.as_mut().expect("created");
        search.show_replace |= replace;
        let query = search.query.clone();
        if !selected.is_empty() && !selected.contains('\n') && selected.len() < 200 {
            query.update(cx, |field, cx| field.set_text(&selected, cx));
        } else {
            let text = query.read(cx).content.clone();
            query.update(cx, |field, cx| field.set_text(&text, cx));
        }
        window.focus(&query.focus_handle(cx));
        cx.notify();
    }

    fn on_query_event(&mut self, event: &FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FieldEvent::Changed => self.run_search(cx),
            FieldEvent::Submit { backwards } => self.step_match(!backwards, window, cx),
            FieldEvent::Cancel => self.close_search(window, cx),
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let (Some(search), Some(buffer)) = (&self.search, self.buffer()) else { return };
        let query = search.query.read(cx).content.clone();
        let case = search.case_sensitive;
        buffer.update(cx, |buffer, cx| buffer.set_search(query, case, cx));
    }

    pub(crate) fn step_match(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tab() else { return };
        let buffer = tab.buffer.clone();
        let views = [tab.semantic.clone(), tab.source.clone()];
        let found = buffer.update(cx, |buffer, cx| buffer.step_match(forward, cx));
        if let Some(range) = found {
            let line = buffer.read(cx).doc.line_of_offset(range.start);
            for view in views {
                view.update(cx, |view, cx| view.scroll_to_line((line as f64 - 4.0).max(0.0), cx));
            }
        }
    }

    fn replace_one(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(search), Some(buffer)) = (&self.search, self.buffer()) else { return };
        let replacement = search.replacement.read(cx).content.clone();
        buffer.update(cx, |buffer, cx| buffer.replace_current(&replacement, cx));
        self.step_match(true, window, cx);
    }

    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let (Some(search), Some(buffer)) = (&self.search, self.buffer()) else { return };
        let replacement = search.replacement.read(cx).content.clone();
        let count = buffer.update(cx, |buffer, cx| buffer.replace_all(&replacement, cx));
        self.notice = Some(format!("已替换 {count} 处").into());
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        if let Some(tab) = self.tab() {
            let buffer = tab.buffer.clone();
            let primary = tab.primary().clone();
            buffer.update(cx, |buffer, cx| buffer.set_search(String::new(), false, cx));
            window.focus(&primary.focus_handle(cx));
        }
        cx.notify();
    }

    // ---- 大纲 ----

    pub(crate) fn jump_to_heading(&mut self, heading: &Heading, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let line = heading.line;
        tab.top_line = line as f64;
        let buffer = tab.buffer.clone();
        let views = [tab.semantic.clone(), tab.source.clone()];
        let primary = tab.primary().clone();
        buffer.update(cx, |buffer, cx| {
            let start = buffer.doc.line_range(line).start;
            buffer.set_selection(start, start, cx);
        });
        for view in views {
            view.update(cx, |view, cx| view.scroll_to_line(line as f64, cx));
        }
        window.focus(&primary.focus_handle(cx));
        cx.notify();
    }

    // ---- 格式命令 ----

    fn with_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>, update: impl FnOnce(&mut Buffer, &mut Context<Buffer>)) {
        let Some(tab) = self.tab() else { return };
        let buffer = tab.buffer.clone();
        let primary = tab.primary().clone();
        buffer.update(cx, update);
        window.focus(&primary.focus_handle(cx));
    }

    pub(crate) fn zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        // 与 Web 版“缩放级别”一致：80%–160%，步长 5%。
        let current = cx.global::<Settings>().zoom;
        let next = if delta == 0.0 { 1.0 } else { (current + delta).clamp(0.8, 1.6) };
        Settings::set(cx, "zoomPercent", serde_json::json!((next * 100.0).round() as i64));
    }

    // ---- 渲染 ----

    fn render_titlebar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let buffer = tab.buffer.read(cx);
                let active = index == self.active;
                let title = buffer.title();
                let dirty = buffer.is_dirty();
                let close: AnyElement = if dirty {
                    // 未保存：显示圆点，悬停时显示关闭图标（与 Web 版标签一致）。
                    div()
                        .id(("close", index))
                        .group("tab-close")
                        .size(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(theme.hover))
                        .child(div().size(px(7.0)).rounded_full().bg(theme.accent))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            workspace.close_tab(index, window, cx);
                        }))
                        .into_any_element()
                } else {
                    small_icon_button(("close", index), IconName::Close, false, theme)
                        .size(px(18.0))
                        .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            workspace.close_tab(index, window, cx);
                        }))
                        .into_any_element()
                };
                div()
                    .id(("tab", index))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(30.0))
                    .pl(px(10.0))
                    .pr(px(6.0))
                    .mt(px(8.0))
                    .rounded_t(px(10.0))
                    .text_size(px(13.0))
                    .when(active, |el| el.bg(theme.surface).text_color(theme.fg).border_1().border_b_0().border_color(theme.border))
                    .when(!active, |el| el.text_color(theme.muted).hover(|style| style.bg(theme.hover)))
                    .child(icon(IconName::FileText, 14.0, if active { theme.accent } else { theme.muted }))
                    .child(div().max_w(px(180.0)).overflow_hidden().whitespace_nowrap().child(title))
                    .child(close)
                    .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| workspace.activate(index, window, cx)))
                    .into_any_element()
            })
            .collect();
        div()
            .id("titlebar")
            .h(px(38.0))
            .w_full()
            .flex()
            .flex_none()
            .items_end()
            .bg(theme.rail_bg)
            .pl(px(84.0))
            .gap(px(2.0))
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .children(tabs)
            .child(
                small_icon_button("new-tab", IconName::Plus, false, theme)
                    .mb(px(5.0))
                    .size(px(24.0))
                    .tooltip(tooltip("新建 ⌘N"))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.new_file(window, cx))),
            )
    }

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(Buffer::new(Vec::new(), None), window, cx);
    }

    fn render_sidebar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active_path = self.tab().and_then(|tab| tab.buffer.read(cx).path.clone());
        let (root_name, rows) = match &self.tree {
            Some(tree) => (tree.root_name(), tree.rows().to_vec()),
            None => ("当前文件夹".to_string(), Vec::new()),
        };
        let theme = theme.clone();
        let count = rows.len();
        let rows = Rc::new(rows);
        div()
            .w(px(232.0))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.rail_bg)
            .text_size(px(13.0))
            .child(
                div()
                    .id("tree-root")
                    .mx(px(6.0))
                    .px(px(8.0))
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(6.0))
                    .text_color(theme.heading)
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|style| style.bg(theme.hover))
                    .tooltip(tooltip("打开文件夹 ⌘⇧O"))
                    .child(icon(IconName::ChevronDown, 14.0, theme.muted))
                    .child(icon(IconName::FolderOpen, 15.0, theme.muted))
                    .child(div().overflow_hidden().whitespace_nowrap().child(root_name))
                    .on_click(cx.listener(|workspace, _: &ClickEvent, window, cx| workspace.prompt_open(true, window, cx))),
            )
            .child(
                uniform_list("file-tree", count, {
                    let theme = theme.clone();
                    cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let row = rows[index].clone();
                                let active = active_path.as_deref() == Some(row.entry.path.as_path());
                                let path = row.entry.path.clone();
                                let is_dir = row.entry.is_dir;
                                let chevron: AnyElement = if is_dir {
                                    icon(if row.expanded { IconName::ChevronDown } else { IconName::ChevronRight }, 13.0, theme.muted).into_any_element()
                                } else {
                                    div().w(px(13.0)).flex_none().into_any_element()
                                };
                                let kind = match (is_dir, row.expanded) {
                                    (true, true) => IconName::FolderOpen,
                                    (true, false) => IconName::Folder,
                                    _ => IconName::FileText,
                                };
                                div()
                                    .id(("tree", index))
                                    .h(px(28.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .pl(px(8.0 + row.depth as f32 * 14.0))
                                    .pr(px(8.0))
                                    .mx(px(6.0))
                                    .rounded(px(6.0))
                                    .text_color(if active { theme.accent_strong } else { theme.fg })
                                    .when(active, |el| el.bg(theme.sidebar_active).font_weight(FontWeight::SEMIBOLD))
                                    .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
                                    .child(chevron)
                                    .child(icon(kind, 15.0, if is_dir { theme.muted } else if active { theme.accent } else { theme.muted }))
                                    .child(div().overflow_hidden().whitespace_nowrap().child(row.entry.name.clone()))
                                    .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| {
                                        if is_dir {
                                            if let Some(tree) = &mut workspace.tree {
                                                tree.toggle(&path);
                                            }
                                            cx.notify();
                                        } else {
                                            workspace.open_path(&path, window, cx);
                                        }
                                    }))
                            })
                            .collect()
                    })
                })
                .track_scroll(self.tree_scroll.clone())
                .flex_1(),
            )
    }

    fn render_toolbar(&mut self, theme: &Theme, mode: EditorMode, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>().clone();
        type Action = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
        let tool = |id: &'static str, name: IconName, tip: &'static str, active: bool, action: Action, cx: &mut Context<Self>| {
            icon_button(id, name, active, theme)
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| action(workspace, window, cx)))
                .into_any_element()
        };
        let left: Vec<AnyElement> = vec![
            tool("h1", IconName::Heading1, "一级标题 ⌘⌥1", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.set_heading(1, cx)), cx),
            tool("h2", IconName::Heading2, "二级标题 ⌘⌥2", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.set_heading(2, cx)), cx),
            tool("h3", IconName::Heading3, "三级标题 ⌘⌥3", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.set_heading(3, cx)), cx),
            vertical_divider(theme).into_any_element(),
            tool("bold", IconName::Bold, "加粗 ⌘B", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("**", "**", cx)), cx),
            tool("italic", IconName::Italic, "斜体 ⌘I", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("*", "*", cx)), cx),
            tool("strike", IconName::Strikethrough, "删除线 ⌘⇧X", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("~~", "~~", cx)), cx),
            tool("underline", IconName::Underline, "下划线 ⌘U", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("<u>", "</u>", cx)), cx),
            tool("highlight", IconName::Highlighter, "高亮 ⌘⇧H", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("<mark>", "</mark>", cx)), cx),
            tool("code", IconName::CodeXml, "行内代码 ⌘⇧C", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("`", "`", cx)), cx),
            tool("link", IconName::Link, "链接 ⌘K", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.wrap_selection("[", "](https://)", cx)), cx),
            vertical_divider(theme).into_any_element(),
            tool("quote", IconName::Quote, "引用", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.toggle_line_prefix("> ", cx)), cx),
            tool("callout", IconName::Info, "提示块", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.insert_block("> [!NOTE]\n> ", 12, cx)), cx),
            tool("ul", IconName::List, "无序列表", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.toggle_line_prefix("- ", cx)), cx),
            tool("ol", IconName::ListOrdered, "有序列表", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.toggle_line_prefix("1. ", cx)), cx),
            tool("task", IconName::SquareCheck, "任务列表", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.toggle_line_prefix("- [ ] ", cx)), cx),
            vertical_divider(theme).into_any_element(),
            tool("codeblock", IconName::Braces, "代码块 ⌘⇧K", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| insert_code_block(b, cx)), cx),
            tool("table", IconName::Table, "表格 ⌘⇧T", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.insert_block(TABLE_TEMPLATE, 2, cx)), cx),
            tool("math", IconName::Sigma, "公式块 ⌘⇧M", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.insert_block("$$\n\n$$", 3, cx)), cx),
            tool("rule", IconName::Minus, "分隔线", false, |w, win, cx| w.with_buffer(win, cx, |b, cx| b.insert_block("---", 3, cx)), cx),
        ];
        let mode_segment = |id: &'static str, name: IconName, label: &'static str, tip: &'static str, target: EditorMode, cx: &mut Context<Self>| {
            segment(id, name, label, mode == target, theme)
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| workspace.set_mode(target, window, cx)))
        };
        // 内容宽度滑杆（Web 版工具栏同款，45%–90%）：拖动中只改内存，松手写入设置。
        let width_percent = (settings.content_width_percent * 100.0).round();
        let entity = cx.entity().downgrade();
        let on_width = std::rc::Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
            let percent = (45.0 + 45.0 * fraction).round() as i64;
            Settings::set_with(cx, "contentWidthPercent", serde_json::json!(percent), !dragging);
            entity.update(cx, |workspace, cx| {
                workspace.width_dragging = dragging;
                cx.notify();
            })
            .ok();
        });
        let width_control = div()
            .id("content-width")
            .flex()
            .items_center()
            .gap(px(4.0))
            .tooltip(tooltip("内容宽度"))
            .child(icon(IconName::Columns, 14.0, theme.muted))
            .child(slider("content-width-slider", (width_percent - 45.0) / 45.0, self.width_dragging, 72.0, theme, on_width))
            .child(div().w(px(32.0)).text_size(px(12.0)).text_color(theme.muted).child(format!("{width_percent:.0}%")))
            .into_any_element();
        let right: Vec<AnyElement> = vec![
            tool("search", IconName::Search, "查找 ⌘F", self.search.is_some(), |w, win, cx| w.open_search(false, win, cx), cx),
            tool("kp", IconName::Pilcrow, "精细排版（Knuth–Plass 两端对齐）", settings.typography, |_, _, cx| Settings::toggle(cx, "typographyEnabled"), cx),
            width_control,
            vertical_divider(theme).into_any_element(),
            segmented(theme)
                .child(mode_segment("mode-semantic", IconName::BookOpenText, "语义", "语义模式 ⌘1", EditorMode::Semantic, cx))
                .child(mode_segment("mode-source", IconName::CodeXml, "源码", "源码模式 ⌘2 / ⌘E", EditorMode::Source, cx))
                .child(mode_segment("mode-split", IconName::Columns, "分栏", "分栏模式 ⌘3", EditorMode::Split, cx))
                .into_any_element(),
            tool("outline", IconName::ListTree, "文档大纲 ⌘⌥O", settings.show_outline, |_, _, cx| Settings::toggle(cx, "outlineVisible"), cx),
            tool("theme", if theme.dark { IconName::Sun } else { IconName::Moon }, "切换深色 / 浅色 ⌘⇧L", false, |_, _, cx| Theme::toggle(cx), cx),
            tool("settings", IconName::Settings, "设置 ⌘,", false, |_, _, cx| crate::ui::settings_window::open(cx), cx),
        ];
        div()
            .h(px(42.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .child(div().flex().items_center().gap(px(2.0)).overflow_hidden().children(left))
            .child(div().flex().flex_none().items_center().gap(px(4.0)).children(right))
    }

    fn render_outline(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !cx.global::<Settings>().show_outline {
            return None;
        }
        let tab = self.tab()?;
        let buffer = tab.buffer.clone();
        let top_line = tab.top_line;
        let all = buffer.update(cx, |buffer, _| buffer.outline());
        // 设置“大纲默认展开层级”：只列出相对最高级标题不超过该层数的标题。
        let top_level = all.iter().map(|heading| heading.level).min().unwrap_or(1);
        let depth = cx.global::<Settings>().outline_level;
        let headings: Rc<Vec<Heading>> = Rc::new(all.iter().filter(|heading| heading.level < top_level + depth).cloned().collect());
        let current = headings.partition_point(|heading| heading.line as f64 <= top_line + 0.5).saturating_sub(1);
        let count = headings.len();
        let theme = theme.clone();
        let min_level = headings.iter().map(|heading| heading.level).min().unwrap_or(1);
        let list_height = (count as f32 * 28.0).clamp(28.0, 420.0);
        let body: AnyElement = if count == 0 {
            div().px(px(14.0)).pb(px(12.0)).text_color(theme.muted).text_size(px(12.0)).child("当前文档还没有标题").into_any_element()
        } else {
            let headings_list = headings.clone();
            uniform_list("outline", count, {
                let theme = theme.clone();
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            let heading = headings_list[index].clone();
                            let active = index == current;
                            div()
                                .id(("outline", index))
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .pl(px(10.0 + (heading.level.saturating_sub(min_level)) as f32 * 14.0))
                                .pr(px(8.0))
                                .mx(px(6.0))
                                .rounded(px(6.0))
                                .text_size(px(12.5))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(if active { theme.accent_strong } else if heading.level <= min_level { theme.fg } else { theme.muted })
                                .when(active, |el| el.bg(theme.sidebar_active).font_weight(FontWeight::SEMIBOLD))
                                .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
                                .child(if heading.text.is_empty() { "（空标题）".to_string() } else { heading.text.clone() })
                                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| workspace.jump_to_heading(&heading, window, cx)))
                        })
                        .collect()
                })
            })
            .track_scroll(self.outline_scroll.clone())
            .h(px(list_height))
            .into_any_element()
        };
        if count > 0 {
            self.outline_scroll.scroll_to_item(current, ScrollStrategy::Center);
        }
        Some(
            floating_card("outline-card", &theme)
                .top(px(14.0))
                .right(px(22.0))
                .w(px(236.0))
                .pt(px(12.0))
                .pb(px(6.0))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px(px(14.0))
                        .pb(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.heading)
                        .child(icon(IconName::ListTree, 14.0, theme.muted))
                        .child("文档大纲")
                        .child(div().text_color(theme.muted).font_weight(FontWeight::NORMAL).child(count.to_string())),
                )
                .child(body)
                .into_any_element(),
        )
    }

    fn render_search(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.search.as_ref()?;
        let buffer = self.buffer()?;
        let state = &buffer.read(cx).search;
        let status = if state.pending {
            "…".to_string()
        } else if state.matches.is_empty() {
            if search.query.read(cx).content.is_empty() { String::new() } else { "无结果".into() }
        } else {
            format!("{}/{}{}", state.current.map_or(0, |index| index + 1), state.matches.len(), if state.truncated { "+" } else { "" })
        };
        let case = search.case_sensitive;
        let show_replace = search.show_replace;
        let replace_row = show_replace.then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(search.replacement.clone())
                .child(small_icon_button("replace-one", IconName::Replace, false, theme).tooltip(tooltip("替换当前")).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.replace_one(window, cx))))
                .child(small_icon_button("replace-all", IconName::ReplaceAll, false, theme).tooltip(tooltip("全部替换")).on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.replace_all(cx))))
                .child(div().w(px(22.0)))
        });
        Some(
            floating_card("search-panel", theme)
                .top(px(10.0))
                .right(px(272.0))
                .w(px(400.0))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(icon(IconName::Search, 14.0, theme.muted))
                        .child(search.query.clone())
                        .child(div().min_w(px(52.0)).text_size(px(12.0)).text_color(theme.muted).child(status))
                        .child(small_icon_button("case", IconName::CaseSensitive, case, theme).tooltip(tooltip("区分大小写")).on_click(cx.listener(|w, _: &ClickEvent, _, cx| {
                            if let Some(search) = &mut w.search {
                                search.case_sensitive = !search.case_sensitive;
                            }
                            w.run_search(cx);
                            cx.notify();
                        })))
                        .child(small_icon_button("prev", IconName::ArrowUp, false, theme).tooltip(tooltip("上一个 ⇧⏎")).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.step_match(false, window, cx))))
                        .child(small_icon_button("next", IconName::ArrowDown, false, theme).tooltip(tooltip("下一个 ⏎")).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.step_match(true, window, cx))))
                        .child(small_icon_button("toggle-replace", IconName::Replace, show_replace, theme).tooltip(tooltip("替换 ⌘⌥F")).on_click(cx.listener(|w, _: &ClickEvent, _, cx| {
                            if let Some(search) = &mut w.search {
                                search.show_replace = !search.show_replace;
                            }
                            cx.notify();
                        })))
                        .child(small_icon_button("close-search", IconName::Close, false, theme).tooltip(tooltip("关闭 Esc")).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.close_search(window, cx)))),
                )
                .children(replace_row)
                .into_any_element(),
        )
    }

    fn render_status(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>().clone();
        let (left, line_col, words, mode_label) = match self.tab() {
            Some(tab) => {
                let buffer = tab.buffer.read(cx);
                let (line, col) = buffer.cursor_line_col();
                let words = match buffer.stats {
                    None => "统计中…".to_string(),
                    Some((_, stats)) => {
                        let mut parts = Vec::new();
                        if settings.stats_visible {
                            parts.push(match settings.stats_metric {
                                StatsMetric::Lines => format!("{} 行", stats.lines),
                                StatsMetric::Words => format!("{} 字", stats.words),
                                StatsMetric::VisibleChars => format!("{} 字符（不含空白）", stats.visible_chars),
                                StatsMetric::Chars => format!("{} 字符", stats.chars),
                            });
                        }
                        if settings.reading_time {
                            // 中文按每分钟约 400 字估算阅读时间。
                            parts.push(format!("约 {} 分钟", stats.words.div_ceil(400).max(1)));
                        }
                        parts.join(" · ")
                    }
                };
                let path = buffer.path.as_ref().map_or("未保存的新文档".to_string(), |path| path.display().to_string());
                let mode = match tab.mode {
                    EditorMode::Semantic => "语义",
                    EditorMode::Source => "源码",
                    EditorMode::Split => "分栏",
                };
                (path, format!("第 {line} 行，第 {col} 列"), words, mode)
            }
            None => (String::new(), String::new(), String::new(), ""),
        };
        let left = self.notice.clone().map(|notice| notice.to_string()).unwrap_or(left);
        div()
            .h(px(28.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(14.0))
            .text_size(px(12.0))
            .text_color(theme.muted)
            .border_t_1()
            .border_color(theme.border)
            .child(div().overflow_hidden().whitespace_nowrap().child(left))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .child(line_col)
                    .child(words)
                    .child(mode_label)
                    .child("UTF-8")
                    .child(small_icon_button("zoom-out", IconName::ZoomOut, false, theme).tooltip(tooltip("缩小 ⌘-")).on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.zoom(-0.1, cx))))
                    .child(div().px(px(6.0)).h(px(20.0)).flex().items_center().rounded(px(5.0)).border_1().border_color(theme.border).child(format!("{:.0}%", settings.zoom * 100.0)))
                    .child(small_icon_button("zoom-in", IconName::ZoomIn, false, theme).tooltip(tooltip("放大 ⌘=")).on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.zoom(0.1, cx)))),
            )
    }

    fn render_empty(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(div().size(px(44.0)).rounded(px(10.0)).bg(theme.hover).flex().items_center().justify_center().child(icon(IconName::FileText, 20.0, theme.accent_strong)))
            .child(div().text_size(px(17.0)).font_weight(FontWeight::BOLD).text_color(theme.heading).child("未打开文档"))
            .child(div().text_size(px(13.0)).text_color(theme.muted).child("从左侧选择 Markdown 文件，或新建一个文档开始编辑。"))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .child(button("empty-new", "新建文件", Some(IconName::SquarePen), ButtonKind::Primary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.new_file(window, cx))))
                    .child(button("empty-open", "打开文件", Some(IconName::FileText), ButtonKind::Secondary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.prompt_open(false, window, cx))))
                    .child(button("empty-folder", "打开文件夹", Some(IconName::FolderOpen), ButtonKind::Secondary, theme).on_click(cx.listener(|w, _: &ClickEvent, window, cx| w.prompt_open(true, window, cx)))),
            )
    }
}

const TABLE_TEMPLATE: &str = "| 列 1 | 列 2 | 列 3 |\n| --- | --- | --- |\n|  |  |  |";

/// 插入代码块，语言取设置“默认代码块语言”。
fn insert_code_block(buffer: &mut Buffer, cx: &mut Context<Buffer>) {
    let language = cx.global::<Settings>().default_code_language.clone();
    let block = format!("```{language}\n\n```");
    let caret = language.len() + 4;
    buffer.insert_block(&block, caret, cx);
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let settings = cx.global::<Settings>().clone();
        let ui_font = Some(settings.string("uiFontFamily")).filter(|name| !name.trim().is_empty()).unwrap_or_else(|| ".SystemUIFont".into());
        let mode = self.tab().map(|tab| tab.mode);
        let titlebar = self.render_titlebar(&theme, cx).into_any_element();
        let sidebar = settings.show_sidebar.then(|| self.render_sidebar(&theme, cx).into_any_element());
        let main: AnyElement = match mode {
            None => self.render_empty(&theme, cx).into_any_element(),
            Some(mode) => {
                let toolbar = settings.show_toolbar.then(|| self.render_toolbar(&theme, mode, cx).into_any_element());
                let tab = self.tab().expect("active tab");
                let editors: AnyElement = match mode {
                    EditorMode::Semantic => div().size_full().child(tab.semantic.clone()).into_any_element(),
                    EditorMode::Source => div().size_full().child(tab.source.clone()).into_any_element(),
                    EditorMode::Split => {
                        // 设置“双栏排列”：默认左语义右源码，可切换为左源码右语义。
                        let (left, right) = if settings.split_source_first { (tab.source.clone(), tab.semantic.clone()) } else { (tab.semantic.clone(), tab.source.clone()) };
                        div()
                            .size_full()
                            .flex()
                            .child(div().flex_1().h_full().child(left))
                            .child(div().w(px(1.0)).h_full().bg(theme.border))
                            .child(div().flex_1().h_full().child(right))
                            .into_any_element()
                    }
                };
                let outline = if mode == EditorMode::Split { None } else { self.render_outline(&theme, cx) };
                let search = self.render_search(&theme, cx);
                let status = self.render_status(&theme, cx).into_any_element();
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .children(toolbar)
                    .child(div().relative().flex_1().min_h(px(0.0)).child(editors).children(outline).children(search))
                    .child(status)
                    .into_any_element()
            }
        };
        let _ = window;
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.rail_bg)
            .text_color(theme.fg)
            .font_family(ui_font)
            .on_action(cx.listener(|w, _: &NewFile, window, cx| w.new_file(window, cx)))
            .on_action(cx.listener(|w, _: &OpenFile, window, cx| w.prompt_open(false, window, cx)))
            .on_action(cx.listener(|w, _: &OpenFolder, window, cx| w.prompt_open(true, window, cx)))
            .on_action(cx.listener(|w, _: &Save, window, cx| {
                if let Some(buffer) = w.buffer() {
                    w.save_buffer(&buffer, false, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &SaveAs, window, cx| {
                if let Some(buffer) = w.buffer() {
                    w.save_buffer(&buffer, true, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                let index = w.active;
                w.close_tab(index, window, cx);
            }))
            .on_action(cx.listener(|w, _: &NextTab, window, cx| {
                if !w.tabs.is_empty() {
                    let next = (w.active + 1) % w.tabs.len();
                    w.activate(next, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &PrevTab, window, cx| {
                if !w.tabs.is_empty() {
                    let previous = (w.active + w.tabs.len() - 1) % w.tabs.len();
                    w.activate(previous, window, cx);
                }
            }))
            .on_action(cx.listener(|w, _: &Find, window, cx| w.open_search(false, window, cx)))
            .on_action(cx.listener(|w, _: &FindReplace, window, cx| w.open_search(true, window, cx)))
            .on_action(cx.listener(|w, _: &FindNext, window, cx| w.step_match(true, window, cx)))
            .on_action(cx.listener(|w, _: &FindPrev, window, cx| w.step_match(false, window, cx)))
            .on_action(cx.listener(|w, _: &ToggleSource, window, cx| {
                let next = if w.tab().map(|tab| tab.mode) == Some(EditorMode::Source) { EditorMode::Semantic } else { EditorMode::Source };
                w.set_mode(next, window, cx);
            }))
            .on_action(cx.listener(|w, _: &ModeSemantic, window, cx| w.set_mode(EditorMode::Semantic, window, cx)))
            .on_action(cx.listener(|w, _: &ModeSource, window, cx| w.set_mode(EditorMode::Source, window, cx)))
            .on_action(cx.listener(|w, _: &ModeSplit, window, cx| w.set_mode(EditorMode::Split, window, cx)))
            .on_action(cx.listener(|_, _: &ToggleOutline, _, cx| Settings::toggle(cx, "outlineVisible")))
            .on_action(cx.listener(|_, _: &ToggleSidebar, _, cx| Settings::toggle(cx, "sidebarHidden")))
            .on_action(cx.listener(|_, _: &ToggleToolbar, _, cx| Settings::toggle(cx, "toolbarHidden")))
            .on_action(cx.listener(|_, _: &ToggleTheme, _, cx| Theme::toggle(cx)))
            .on_action(cx.listener(|_, _: &FollowSystemTheme, _, cx| Settings::set(cx, "themeMode", serde_json::json!("system"))))
            .on_action(cx.listener(|w, _: &Underline, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("<u>", "</u>", cx))))
            .on_action(cx.listener(|w, _: &Highlight, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("<mark>", "</mark>", cx))))
            .on_action(cx.listener(|_, _: &ToggleTypography, _, cx| Settings::toggle(cx, "typographyEnabled")))
            .on_action(cx.listener(|w, _: &ZoomIn, _, cx| w.zoom(0.1, cx)))
            .on_action(cx.listener(|w, _: &ZoomOut, _, cx| w.zoom(-0.1, cx)))
            .on_action(cx.listener(|w, _: &ZoomReset, _, cx| w.zoom(0.0, cx)))
            .on_action(cx.listener(|w, _: &Bold, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("**", "**", cx))))
            .on_action(cx.listener(|w, _: &Italic, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("*", "*", cx))))
            .on_action(cx.listener(|w, _: &Strike, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("~~", "~~", cx))))
            .on_action(cx.listener(|w, _: &InlineCode, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("`", "`", cx))))
            .on_action(cx.listener(|w, _: &Link, window, cx| w.with_buffer(window, cx, |b, cx| b.wrap_selection("[", "](https://)", cx))))
            .on_action(cx.listener(|w, _: &Heading1, window, cx| w.with_buffer(window, cx, |b, cx| b.set_heading(1, cx))))
            .on_action(cx.listener(|w, _: &Heading2, window, cx| w.with_buffer(window, cx, |b, cx| b.set_heading(2, cx))))
            .on_action(cx.listener(|w, _: &Heading3, window, cx| w.with_buffer(window, cx, |b, cx| b.set_heading(3, cx))))
            .on_action(cx.listener(|w, _: &Paragraph, window, cx| w.with_buffer(window, cx, |b, cx| b.set_heading(0, cx))))
            .on_action(cx.listener(|w, _: &Quote, window, cx| w.with_buffer(window, cx, |b, cx| b.toggle_line_prefix("> ", cx))))
            .on_action(cx.listener(|w, _: &BulletList, window, cx| w.with_buffer(window, cx, |b, cx| b.toggle_line_prefix("- ", cx))))
            .on_action(cx.listener(|w, _: &OrderedList, window, cx| w.with_buffer(window, cx, |b, cx| b.toggle_line_prefix("1. ", cx))))
            .on_action(cx.listener(|w, _: &TaskList, window, cx| w.with_buffer(window, cx, |b, cx| b.toggle_line_prefix("- [ ] ", cx))))
            .on_action(cx.listener(|w, _: &CodeBlock, window, cx| w.with_buffer(window, cx, |b, cx| insert_code_block(b, cx))))
            .on_action(cx.listener(|w, _: &Table, window, cx| w.with_buffer(window, cx, |b, cx| b.insert_block(TABLE_TEMPLATE, 2, cx))))
            .on_action(cx.listener(|w, _: &MathBlock, window, cx| w.with_buffer(window, cx, |b, cx| b.insert_block("$$\n\n$$", 3, cx))))
            .on_action(cx.listener(|w, _: &Rule, window, cx| w.with_buffer(window, cx, |b, cx| b.insert_block("---", 3, cx))))
            .child(titlebar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .children(sidebar)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .bg(theme.surface)
                            .border_t_1()
                            .border_l_1()
                            .border_color(theme.border)
                            .rounded_tl(px(12.0))
                            .overflow_hidden()
                            .child(main),
                    ),
            )
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let context = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("cmd-n", NewFile, context),
        KeyBinding::new("cmd-o", OpenFile, context),
        KeyBinding::new("cmd-shift-o", OpenFolder, context),
        KeyBinding::new("cmd-s", Save, context),
        KeyBinding::new("cmd-shift-s", SaveAs, context),
        KeyBinding::new("cmd-w", CloseTab, context),
        KeyBinding::new("ctrl-tab", NextTab, context),
        KeyBinding::new("ctrl-shift-tab", PrevTab, context),
        KeyBinding::new("cmd-f", Find, context),
        KeyBinding::new("cmd-alt-f", FindReplace, context),
        KeyBinding::new("cmd-g", FindNext, context),
        KeyBinding::new("cmd-shift-g", FindPrev, context),
        KeyBinding::new("cmd-e", ToggleSource, context),
        KeyBinding::new("cmd-1", ModeSemantic, context),
        KeyBinding::new("cmd-2", ModeSource, context),
        KeyBinding::new("cmd-3", ModeSplit, context),
        KeyBinding::new("cmd-alt-o", ToggleOutline, context),
        KeyBinding::new("cmd-alt-s", ToggleSidebar, context),
        KeyBinding::new("cmd-shift-b", ToggleToolbar, context),
        KeyBinding::new("cmd-shift-l", ToggleTheme, context),
        KeyBinding::new("cmd-=", ZoomIn, context),
        KeyBinding::new("cmd-+", ZoomIn, context),
        KeyBinding::new("cmd--", ZoomOut, context),
        KeyBinding::new("cmd-0", ZoomReset, context),
        KeyBinding::new("cmd-b", Bold, context),
        KeyBinding::new("cmd-i", Italic, context),
        KeyBinding::new("cmd-shift-x", Strike, context),
        KeyBinding::new("cmd-u", Underline, context),
        KeyBinding::new("cmd-shift-h", Highlight, context),
        KeyBinding::new("cmd-shift-c", InlineCode, context),
        KeyBinding::new("cmd-k", Link, context),
        KeyBinding::new("cmd-alt-1", Heading1, context),
        KeyBinding::new("cmd-alt-2", Heading2, context),
        KeyBinding::new("cmd-alt-3", Heading3, context),
        KeyBinding::new("cmd-alt-0", Paragraph, context),
        KeyBinding::new("cmd-shift-k", CodeBlock, context),
        KeyBinding::new("cmd-shift-t", Table, context),
        KeyBinding::new("cmd-shift-m", MathBlock, context),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
    ]);
}
