//! 工具栏外观与交互。文档状态只由活动 DocumentHost 提供，不持有正文副本。
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolbarPopupKind { Width, Math, Diagram }

pub(super) const TABLE_ANCHOR: usize = 3;
pub(super) const LINK_ANCHOR: usize = 4;

impl ToolbarPopupKind {
    fn anchor_index(self) -> usize {
        match self { Self::Width => 0, Self::Math => 1, Self::Diagram => 2 }
    }
}

pub(super) struct ToolbarPopup {
    kind: ToolbarPopupKind,
    position: gpui::Point<gpui::Pixels>,
    document: Option<gpui::WeakEntity<DocumentHost>>,
    target: Option<EditTarget>,
    selected: usize,
    keyboard_selection: bool,
    scroll: ScrollHandle,
}

const DIAGRAMS: &[(&str, &str, &str)] = &[
    ("空白图表", "mermaid", ""),
    crate::nomo_mark::quick_insert::DIAGRAMS[0],
    crate::nomo_mark::quick_insert::DIAGRAMS[1],
    crate::nomo_mark::quick_insert::DIAGRAMS[2],
    crate::nomo_mark::quick_insert::DIAGRAMS[3],
    crate::nomo_mark::quick_insert::DIAGRAMS[4],
    crate::nomo_mark::quick_insert::DIAGRAMS[5],
    crate::nomo_mark::quick_insert::DIAGRAMS[6],
];

fn toolbar_button(id: &'static str, name: IconName, active: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let size = if matches!(id, "search" | "outline" | "split-guide") { 18.0 }
        else if id == "hide-toolbar" { 16.0 } else { 17.0 };
    div().id(id).group(id).w(px(32.0)).h(px(28.0)).flex().flex_none().items_center().justify_center().tab_index(0)
        .rounded(px(4.0))
        .when(id == "hide-toolbar", |el| el.ml(px(2.0)).border_1().border_color(gpui::transparent_black()))
        .when(active, |el| el.bg(theme.surface.blend(theme.accent.opacity(0.10))))
        .text_color(if active { theme.accent_strong } else { theme.muted })
        .hover(|style| {
            let style = style.bg(theme.hover).text_color(theme.accent_strong);
            if id == "hide-toolbar" { style.border_color(theme.border) } else { style }
        })
        .focus(|style| style.border_1().border_color(theme.accent)).cursor_pointer()
        .child(gpui::svg().path(name.path()).size(px(size)).flex_none()
            .text_color(if active { theme.accent_strong } else { theme.muted })
            .group_hover(id, |style| style.text_color(theme.accent_strong)))
}

fn toolbar_divider(theme: &Theme) -> gpui::Div {
    div().w(px(1.0)).h(px(24.0)).mx(px(4.0)).flex_none().bg(theme.border)
}

fn toolbar_modes(theme: &Theme) -> gpui::Div {
    div().relative().flex().p(px(2.0)).rounded(px(4.0)).bg(theme.rail_bg).border_1().border_color(theme.border)
}

fn trigger_marker(bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>) -> impl IntoElement {
    gpui::canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
        .absolute().top_0().left_0().size_full()
}

// 与旧版工具栏高度过渡使用同一 cubic-bezier(0.4, 0, 0.2, 1)。
fn toolbar_ease(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    if progress == 0.0 || progress == 1.0 { return progress; }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..12 {
        let t = (low + high) * 0.5;
        let x = 3.0 * (1.0 - t) * (1.0 - t) * t * 0.4
            + 3.0 * (1.0 - t) * t * t * 0.2 + t * t * t;
        if x < progress { low = t; } else { high = t; }
    }
    let t = (low + high) * 0.5;
    3.0 * (1.0 - t) * t * t + t * t * t
}

fn toolbar_mode_button(id: &'static str, name: IconName, active: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div().id(id).group(id).w(px(34.0)).h(px(28.0)).flex().flex_none().items_center().justify_center().tab_index(0)
        .rounded(px(4.0)).text_color(if active { theme.accent_strong } else { theme.muted })
        .when(!active, |el| el.hover(|style| style.bg(theme.hover).text_color(theme.accent_strong)))
        .focus(|style| style.border_1().border_color(theme.accent)).cursor_pointer()
        .child(gpui::svg().path(name.path()).size(px(17.0)).flex_none()
            .text_color(if active { theme.accent_strong } else { theme.muted })
            .group_hover(id, |style| style.text_color(theme.accent_strong)))
}

pub(super) fn diagram_template(kind: crate::ui::chrome::DiagramKind) -> &'static str {
    use crate::ui::chrome::DiagramKind;
    let index = match kind { DiagramKind::Blank=>0, DiagramKind::Flowchart=>1, DiagramKind::Sequence=>2, DiagramKind::Class=>3, DiagramKind::State=>4, DiagramKind::Pie=>5, DiagramKind::Gantt=>6, DiagramKind::EntityRelationship=>7 };
    DIAGRAMS[index].2
}

impl Workspace {
    pub(super) fn render_toolbar_animated(&mut self, visible: bool, theme: &Theme, mode: EditorMode, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.explorer.reduce_motion {
            self.toolbar_target = Some(visible);
            self.toolbar_motion = None;
            self.toolbar_motion_task = None;
            return visible.then(|| self.render_toolbar(theme, mode, cx).into_any_element());
        }
        let target = *self.toolbar_target.get_or_insert(visible);
        if target != visible {
            let from = self.toolbar_motion.map(|(started, _, from, to, duration)| {
                let progress = (started.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
                from + (to - from) * toolbar_ease(progress)
            }).unwrap_or(if target { 1.0 } else { 0.0 });
            let to = if visible { 1.0 } else { 0.0 };
            let duration = Duration::from_millis(220);
            self.toolbar_target = Some(visible);
            self.toolbar_motion_id = self.toolbar_motion_id.wrapping_add(1);
            let id = self.toolbar_motion_id;
            self.toolbar_motion = Some((Instant::now(), id, from, to, duration));
            self.toolbar_motion_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(duration).await;
                let _ = this.update(cx, |workspace, cx| {
                    if workspace.toolbar_motion.is_some_and(|(_, current, _, _, _)| current == id) {
                        workspace.toolbar_motion = None;
                        cx.notify();
                    }
                });
            }));
        }
        if !visible && self.toolbar_motion.is_none() { return None; }
        let motion = self.toolbar_motion;
        let toolbar = self.render_toolbar(theme, mode, cx).into_any_element();
        if let Some((_, id, from, to, duration)) = motion {
            Some(div().relative().w_full().flex_none().overflow_hidden().child(toolbar)
                .when(!visible, |element| element.child(div().absolute().top_0().left_0().size_full().occlude()))
                .with_animation(("toolbar-motion", id), Animation::new(duration).with_easing(toolbar_ease),
                    move |element, progress| element.h(px(44.0 * (from + (to - from) * progress))))
                .into_any_element())
        } else {
            Some(toolbar)
        }
    }

    pub(super) fn render_toolbar(&mut self, theme: &Theme, mode: EditorMode, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>().clone();
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let marks = self.document().map_or(0, |document| document.read(cx).toolbar_marks());
        type Action = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);
        let tool = |id: &'static str, name: IconName, tip: &'static str, active: bool, action: Action, cx: &mut Context<Self>| {
            let key = if id == "outline" { if active { "outline_hide" } else { "outline_show" } } else { id };
            let tip = locale.tooltip(key, tip);
            toolbar_button(id, name, active, theme)
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| action(workspace, window, cx)))
                .into_any_element()
        };
        let left: Vec<AnyElement> = vec![
            tool("h1", IconName::Heading1, "一级标题 ⌘⌥1", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.set_heading(1, cx)), cx),
            tool("bold", IconName::Bold, "加粗 ⌘B", marks & 1 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.wrap_selection("**", "**", cx)), cx),
            tool("italic", IconName::Italic, "斜体 ⌘I", marks & 2 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.wrap_selection("*", "*", cx)), cx),
            tool("strike", IconName::Strikethrough, "删除线 Alt+Shift+5", marks & 4 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.wrap_selection("~~", "~~", cx)), cx),
            tool("underline", IconName::Underline, "下划线 ⌘U", marks & 8 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.wrap_selection("<u>", "</u>", cx)), cx),
            tool("highlight", IconName::Highlighter, "高亮", marks & 16 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.wrap_selection("<mark>", "</mark>", cx)), cx),
            tool("superscript", IconName::Superscript, "上标", marks & 32 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::ToggleInline(crate::nomo_mark::parse::InlineFormat::Superscript), cx)), cx),
            tool("subscript", IconName::Subscript, "下标", marks & 64 != 0, |w, win, cx| w.with_document(win, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::ToggleInline(crate::nomo_mark::parse::InlineFormat::Subscript), cx)), cx),
            toolbar_button("link", IconName::Link, self.popover.as_ref().is_some_and(|p| p.link.is_some()), theme)
                .relative().child(trigger_marker(self.toolbar_popup_anchors[LINK_ANCHOR].clone()))
                .relative().children(self.popover.as_ref().is_some_and(|p| p.link.is_some()).then(|| trigger_marker(self.toolbar_trigger_bounds.clone())))
                .tooltip(tooltip(locale.tooltip("link", "⌘K")))
                .on_click(cx.listener(|w, event: &ClickEvent, win, cx| w.open_editor_popover_at(EditorPopoverKind::Link, Some(event.position()), win, cx))).into_any_element(),
            tool("inline-comment", IconName::MessageSquare, "行内注释", false, |w, win, cx| w.open_inline_comment(win, cx), cx),
            tool("quote", IconName::Quote, "引用", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.toggle_line_prefix("> ", cx)), cx),
            tool("callout", IconName::Info, "提示块", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.insert_block("> [!NOTE]\n> ", 12, cx)), cx),
            tool("details", IconName::PanelTopClose, "折叠块", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.command(crate::nomo_mark::commands::EditIntent::InsertDetails, cx)), cx),
            tool("ul", IconName::List, "无序列表", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.toggle_line_prefix("- ", cx)), cx),
            tool("task", IconName::SquareCheck, "任务列表", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.toggle_line_prefix("- [ ] ", cx)), cx),
            toolbar_button("table", IconName::Table, self.popover.as_ref().is_some_and(|p| p.link.is_none()), theme).tooltip(tooltip(locale.text("table")))
                .relative().child(trigger_marker(self.toolbar_popup_anchors[TABLE_ANCHOR].clone()))
                .relative().children(self.popover.as_ref().is_some_and(|p| p.link.is_none()).then(|| trigger_marker(self.toolbar_trigger_bounds.clone())))
                .on_click(cx.listener(|w, event: &ClickEvent, win, cx| w.open_editor_popover_at(EditorPopoverKind::Table, Some(event.position()), win, cx))).into_any_element(),
            tool("codeblock", IconName::CodeXml, "代码块 ⌘⇧K", false, |w, win, cx| w.with_document(win, cx, |b, cx| insert_code_block(b, cx)), cx),
            toolbar_button("math", IconName::Sigma, self.toolbar_popup.as_ref().is_some_and(|p| p.kind == ToolbarPopupKind::Math), theme)
                .relative().child(trigger_marker(self.toolbar_popup_anchors[ToolbarPopupKind::Math.anchor_index()].clone()))
                .relative().children(self.toolbar_popup.as_ref().is_some_and(|p| p.kind == ToolbarPopupKind::Math).then(|| trigger_marker(self.toolbar_trigger_bounds.clone())))
                .tooltip(tooltip(locale.text("math")))
                .on_click(cx.listener(|w, event: &ClickEvent, win, cx| w.open_toolbar_popup(ToolbarPopupKind::Math, event.position(), win, cx))).into_any_element(),
            toolbar_button("diagram", IconName::Braces, self.toolbar_popup.as_ref().is_some_and(|p| p.kind == ToolbarPopupKind::Diagram), theme)
                .relative().child(trigger_marker(self.toolbar_popup_anchors[ToolbarPopupKind::Diagram.anchor_index()].clone()))
                .relative().children(self.toolbar_popup.as_ref().is_some_and(|p| p.kind == ToolbarPopupKind::Diagram).then(|| trigger_marker(self.toolbar_trigger_bounds.clone())))
                .tooltip(tooltip(locale.text("diagram")))
                .on_click(cx.listener(|w, event: &ClickEvent, win, cx| w.open_toolbar_popup(ToolbarPopupKind::Diagram, event.position(), win, cx))).into_any_element(),
            tool("footnote", IconName::Superscript, "插入脚注", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.insert_footnote(cx)), cx),
            tool("toc", IconName::TableOfContents, "插入目录", false, |w, win, cx| w.with_document(win, cx, |b, cx| b.insert_toc(cx)), cx),
            toolbar_divider(theme).into_any_element(),
        ];
        let mode_segment = |id: &'static str, name: IconName, _label: &'static str, tip: &'static str, target: EditorMode, cx: &mut Context<Self>| {
            toolbar_mode_button(id, name, mode == target, theme)
                .tooltip(tooltip(locale.tooltip(id, tip)))
                .on_click(cx.listener(move |workspace, _: &ClickEvent, window, cx| workspace.set_mode(target, window, cx)))
        };
        let target = match mode { EditorMode::Semantic => 0.0, EditorMode::Source => 34.0, EditorMode::Split => 68.0 };
        let previous = self.toolbar_mode_target.unwrap_or(target);
        if previous != target {
            let from = self.toolbar_mode_motion.map(|(started, _, from, to)| {
                let progress = (started.elapsed().as_secs_f32() / 0.14).min(1.0);
                from + (to - from) * (1.0 - (1.0 - progress).powi(3))
            }).unwrap_or(previous);
            self.toolbar_mode_motion_id = self.toolbar_mode_motion_id.wrapping_add(1);
            self.toolbar_mode_motion = (!self.explorer.reduce_motion).then_some((Instant::now(), self.toolbar_mode_motion_id, from, target));
        }
        self.toolbar_mode_target = Some(target);
        if self.explorer.reduce_motion || self.toolbar_mode_motion.is_some_and(|(started, _, _, _)| started.elapsed() >= Duration::from_millis(140)) {
            self.toolbar_mode_motion = None;
        }
        let indicator = div().absolute().top(px(2.0)).left(px(2.0 + target)).w(px(34.0)).h(px(28.0))
            .rounded(px(4.0)).bg(theme.surface).shadow(vec![gpui::BoxShadow { inset: false,
                color: gpui::rgba(if theme.dark { 0x0000002e } else { 0x16202c14 }).into(),
                offset: gpui::point(px(0.0), px(1.0)), blur_radius: px(4.0), spread_radius: px(0.0),
            }]);
        let indicator = if let Some((_, id, from, to)) = self.toolbar_mode_motion {
            indicator.with_animation(("toolbar-mode-indicator", id), Animation::new(Duration::from_millis(140))
                .with_easing(|progress| 1.0 - (1.0 - progress).powi(3)),
                move |element, progress| element.left(px(2.0 + from + (to - from) * progress))).into_any_element()
        } else { indicator.into_any_element() };
        // 内容宽度滑杆（Web 版工具栏同款，45%–90%）：拖动中只改内存，松手写入设置。
        let width_percent = (settings.content_width_percent * 100.0).round();
        let entity = cx.entity().downgrade();
        let on_width = std::rc::Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
            if mode==EditorMode::Split {return;}
            let percent = (45.0 + 45.0 * fraction).round() as i64;
            Settings::set_with(cx, "contentWidthPercent", serde_json::json!(percent), !dragging);
            entity.update(cx, |workspace, cx| {
                workspace.width_dragging = dragging;
                cx.notify();
            })
            .ok();
        });
        let narrow = self.outline_ui.bounds.get().is_some_and(|bounds| f32::from(bounds.size.width) <= 820.0);
        let width_control = if narrow {
            div().id("content-width-compact").h(px(28.0)).px(px(7.0)).flex().flex_none().items_center().gap(px(5.0)).tab_index(0)
                .relative().child(trigger_marker(self.toolbar_popup_anchors[ToolbarPopupKind::Width.anchor_index()].clone()))
                .relative().children(self.toolbar_popup.as_ref().is_some_and(|p| p.kind == ToolbarPopupKind::Width).then(|| trigger_marker(self.toolbar_trigger_bounds.clone())))
                .rounded(px(4.0)).border_1().border_color(theme.border).text_color(theme.muted).text_size(px(12.0))
                .font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family)
                .when(mode == EditorMode::Split, |el| el.opacity(0.48))
                .child(icon(IconName::AlignHorizontalSpaceAround, 15.0, theme.muted)).child(format!("{width_percent:.0}%"))
                .tooltip(tooltip(locale.text(if mode == EditorMode::Split { "width_adaptive" } else { "width" })))
                .on_click(cx.listener(move |w, event: &ClickEvent, win, cx| {
                    if mode != EditorMode::Split { w.open_toolbar_popup(ToolbarPopupKind::Width, event.position(), win, cx); }
                })).into_any_element()
        } else {
            div().id("content-width").flex().items_center().gap(px(4.0)).tab_index(0)
                .on_key_down(cx.listener(move |_, event: &gpui::KeyDownEvent, _, cx| {
                    if mode == EditorMode::Split { return; }
                    let value = (cx.global::<Settings>().content_width_percent * 100.0).round() as i64;
                    let next = match event.keystroke.key.as_str() { "left" | "down" => value - 1, "right" | "up" => value + 1, "home" => 45, "end" => 90, _ => return };
                    Settings::set(cx, "contentWidthPercent", serde_json::json!(next.clamp(45, 90))); cx.stop_propagation();
                }))
                .tooltip(tooltip(locale.text(if mode==EditorMode::Split {"width_adaptive"}else{"width"})))
                .when(mode==EditorMode::Split,|el|el.opacity(0.48))
                .child(div().w(px(18.0)).child(icon(IconName::AlignHorizontalSpaceAround, 16.0, theme.muted)))
                .child(div().w(px(34.0)).flex().justify_end().text_size(px(13.0)).text_color(theme.muted)
                    .font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family).child(format!("{width_percent:.0}%")))
                .child(slider("content-width-slider", (width_percent - 45.0) / 45.0, self.width_dragging, 88.0, theme, on_width))
                .into_any_element()
        };
        let mut right: Vec<AnyElement> = vec![
            tool("search", IconName::Search, "查找 ⌘F", self.search.is_some(), |w, win, cx| w.open_search(false, win, cx), cx),
            width_control,
            toolbar_modes(theme)
                .child(indicator)
                .child(mode_segment("mode-semantic", IconName::BookOpenText, "语义", "语义模式", EditorMode::Semantic, cx))
                .child(mode_segment("mode-source", IconName::CodeXml, "源码", "切换源码 ⌘E", EditorMode::Source, cx))
                .child(mode_segment("mode-split", IconName::Columns, "分栏", "分栏模式", EditorMode::Split, cx))
                .into_any_element(),
        ];
        if mode == EditorMode::Split {
            right.push(tool("split-guide", IconName::ScanLine, "显示 / 隐藏分栏对齐线", self.split_guide_visible, |w, _, cx| { w.split_guide_visible = !w.split_guide_visible; cx.notify(); }, cx));
        }
        right.push(tool("outline", IconName::ListTree, "文档大纲 ⌘⌥O", settings.show_outline, |_, _, cx| Settings::toggle(cx, "outlineVisible"), cx));
        right.push(tool("hide-toolbar", IconName::ChevronUp, "隐藏工具栏 ⌘⇧B", false, |w, _, cx| w.toggle_toolbar(cx), cx));
        let scroll_offset = self.toolbar_scroll.offset().x;
        let scroll_max = self.toolbar_scroll.max_offset().x;
        let shadow = |left: bool| {
            div().absolute().top_0().h_full().w(px(1.0)).bg(theme.fg.opacity(0.08))
                .when(left, |el| el.left_0()).when(!left, |el| el.right_0())
                .shadow(vec![gpui::BoxShadow { inset: false, color: theme.fg.opacity(0.14), offset: gpui::point(px(0.0), px(0.0)), blur_radius: px(12.0), spread_radius: px(0.0) }])
        };
        let show_left = scroll_offset < px(-1.0);
        let show_right = -scroll_offset + px(1.0) < scroll_max;
        div()
            .h(px(44.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border.opacity(0.52))
            .tab_group()
            .child(div().relative().flex_1().min_w_0()
                .child(div().id("toolbar-actions").w_full().flex().items_center().gap(px(4.0)).overflow_x_scroll().track_scroll(&self.toolbar_scroll)
                    .on_scroll_wheel(cx.listener(|w, event: &gpui::ScrollWheelEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(32.0));
                        if f32::from(delta.x).abs() < 0.01 && f32::from(delta.y).abs() > 0.01 {
                            let max = w.toolbar_scroll.max_offset().x.max(px(0.0));
                            if max > px(0.0) {
                                let current = w.toolbar_scroll.offset();
                                let next = (current.x + delta.y).clamp(-max, px(0.0));
                                w.toolbar_scroll.set_offset(gpui::point(next, current.y));
                                cx.stop_propagation();
                            }
                        }
                        cx.notify();
                    })).children(left))
                .children(show_left.then(|| shadow(true))).children(show_right.then(|| shadow(false))))
            .child(div().flex().flex_none().items_center().gap(px(4.0))
                .ml(px(if self.outline_ui.bounds.get().is_some_and(|bounds| f32::from(bounds.size.width) <= 620.0) { 0.0 } else { 16.0 }))
                .children(right))
    }

    fn open_toolbar_popup(&mut self, kind: ToolbarPopupKind, position: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.toolbar_popup.as_ref().is_some_and(|popup| popup.kind == kind) {
            self.toolbar_popup = None;
            if let Some(document) = self.document() { window.focus(&document.focus_handle(cx), cx); }
            cx.notify();
            return;
        }
        let (document, target) = if kind == ToolbarPopupKind::Width {
            (None, None)
        } else {
            let Some(document) = self.document() else { return; };
            let target = match document.read(cx).edit_target(cx) {
                Ok(target) => target,
                Err(error) => { self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()); cx.notify(); return; }
            };
            (Some(document.downgrade()), Some(target))
        };
        let selected = usize::from(kind == ToolbarPopupKind::Math && target.as_ref().is_some_and(|t| !t.is_semantic()));
        self.popover = None;
        let anchor = self.toolbar_popup_anchors[kind.anchor_index()].get();
        self.toolbar_trigger_bounds.set(anchor);
        self.toolbar_popup = Some(ToolbarPopup { kind, position, document, target, selected, keyboard_selection: false, scroll: ScrollHandle::new() });
        window.focus(&self.toolbar_popup_focus, cx);
        cx.notify();
    }

    fn choose_toolbar_popup(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.toolbar_popup.take() else { return; };
        let Some(document) = popup.document.and_then(|document| document.upgrade()) else { cx.notify(); return; };
        let Some(target) = popup.target else { cx.notify(); return; };
        if self.document().as_ref() != Some(&document) { self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).text("selection_changed").into()); cx.notify(); return; }
        let mut restore = true;
        let result = match popup.kind {
            ToolbarPopupKind::Math if index == 0 => {
                restore = false;
                document.update(cx, |host, cx| host.begin_inline_math(&target, window, cx))
            }
            ToolbarPopupKind::Math if index == 1 => document.update(cx, |host, cx| host.command_at_target(&target, crate::nomo_mark::commands::EditIntent::InsertMathBlock, cx)),
            ToolbarPopupKind::Diagram if index < DIAGRAMS.len() => document.update(cx, |host, cx| host.command_at_target(&target, crate::nomo_mark::commands::EditIntent::InsertDiagram(DIAGRAMS[index].2.into()), cx)),
            _ => Err(ControlError::InvalidRange),
        };
        if let Err(error) = result { self.notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).error(&error).into()); restore = true; }
        if restore { window.focus(&document.focus_handle(cx), cx); }
        cx.notify();
    }

    pub(super) fn render_toolbar_popup(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.toolbar_popup.as_ref()?;
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let kind = state.kind;
        let position = state.position;
        let selected = state.selected;
        let keyboard_selection = state.keyboard_selection;
        let inline_enabled = state.target.as_ref().is_some_and(EditTarget::is_semantic);
        let mut panel = div().id("toolbar-popup").track_focus(&self.toolbar_popup_focus)
            .track_scroll(&state.scroll)
            .w(px(if kind == ToolbarPopupKind::Width { 220.0 } else { 190.0 }))
            .p(px(if kind == ToolbarPopupKind::Width { 10.0 } else { 6.0 }))
            .flex().flex_col().gap(px(if kind == ToolbarPopupKind::Width { 10.0 } else { 0.0 }))
            .rounded(px(8.0)).border_1().border_color(theme.border).bg(theme.surface)
            .shadow(vec![gpui::BoxShadow { inset: false,
                color: gpui::rgba(if theme.dark { 0x00000052 } else { 0x16202c14 }).into(),
                offset: gpui::point(px(0.0), px(if theme.dark { 16.0 } else { 14.0 })),
                blur_radius: px(if theme.dark { 44.0 } else { 34.0 }), spread_radius: px(0.0),
            }]).occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|w, event: &gpui::MouseDownEvent, _, cx| {
                if !w.toolbar_trigger_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) {
                    w.toolbar_popup = None; w.width_dragging = false; cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |w, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => { w.toolbar_popup = None; w.width_dragging = false; if let Some(document)=w.document() { window.focus(&document.focus_handle(cx), cx); } cx.notify(); }
                    "up" | "down" if kind != ToolbarPopupKind::Width => {
                        if let Some(popup) = &mut w.toolbar_popup {
                            let count = if kind == ToolbarPopupKind::Math { 2 } else { DIAGRAMS.len() };
                            let first = usize::from(kind == ToolbarPopupKind::Math && !inline_enabled);
                            let available = count - first;
                            popup.selected = first + if event.keystroke.key == "up" { (popup.selected - first + available - 1) % available } else { (popup.selected - first + 1) % available };
                            popup.keyboard_selection = true;
                            let row = if kind == ToolbarPopupKind::Diagram { popup.selected + 1 + usize::from(popup.selected > 0) } else { popup.selected };
                            popup.scroll.scroll_to_item(row);
                            cx.notify();
                        }
                    }
                    "enter" | "space" if kind != ToolbarPopupKind::Width => { let index=w.toolbar_popup.as_ref().map_or(0, |p| p.selected); w.choose_toolbar_popup(index, window, cx); }
                    "left" | "right" | "home" | "end" if kind == ToolbarPopupKind::Width => {
                        let value = (cx.global::<Settings>().content_width_percent * 100.0).round() as i64;
                        let next = match event.keystroke.key.as_str() { "left" => value - 1, "right" => value + 1, "home" => 45, _ => 90 };
                        Settings::set(cx, "contentWidthPercent", serde_json::json!(next.clamp(45, 90))); cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }));
        if kind == ToolbarPopupKind::Width {
            let value = (cx.global::<Settings>().content_width_percent * 100.0).round();
            let entity = cx.entity().downgrade();
            let on_width = std::rc::Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
                let percent = (45.0 + 45.0 * fraction).round() as i64;
                Settings::set_with(cx, "contentWidthPercent", serde_json::json!(percent), !dragging);
                let _ = entity.update(cx, |w, cx| { w.width_dragging = dragging; cx.notify(); });
            });
            panel = panel.child(div().flex().justify_between().text_size(px(13.0)).text_color(theme.fg)
                .child(div().font_weight(FontWeight::BOLD).child(locale.text("width")))
                .child(div().font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family)
                    .text_color(theme.accent_strong).child(format!("{value:.0}%"))))
                .child(slider("compact-width-slider", (value - 45.0) / 45.0, self.width_dragging, 200.0, theme, on_width));
        } else {
            let entries: Vec<(&str, &str, bool)> = if kind == ToolbarPopupKind::Math {
                vec![(locale.text("inline_math"), "$…$", inline_enabled), (locale.text("block_math"), "$$…$$", true)]
            } else { DIAGRAMS.iter().enumerate().map(|(index, (_, short, _))| {
                let key = ["diagram_blank", "diagram_flowchart", "diagram_sequence", "diagram_class", "diagram_state", "diagram_pie", "diagram_gantt", "diagram_er"][index];
                (locale.text(key), *short, true)
            }).collect() };
            for (index, (label, suffix, enabled)) in entries.into_iter().enumerate() {
                if kind == ToolbarPopupKind::Diagram && index <= 1 {
                    panel = panel.child(div().pt(px(6.0)).pb(px(8.0)).px(px(8.0)).text_size(px(13.0))
                        .font_weight(FontWeight::BOLD).text_color(theme.muted)
                        .child(locale.text(if index == 0 { "diagram_section" } else { "diagram_templates" })));
                }
                panel = panel.child(div().id(("toolbar-choice", index)).h(px(28.0)).px(px(8.0)).flex().flex_none().items_center().justify_between().when(enabled, |el| el.tab_index(0))
                    .rounded(px(4.0)).text_size(px(13.0)).text_color(theme.fg)
                    .when(!enabled, |el| el.opacity(0.48))
                    .when(keyboard_selection && index == selected && enabled, |el| el.bg(theme.hover))
                    .when(enabled, |el| el.cursor_pointer().hover(|style| style.bg(theme.hover)))
                    .child(label.to_string()).child(div().text_size(px(12.0)).text_color(theme.muted)
                        .font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family).child(suffix.to_string()))
                    .on_click(cx.listener(move |w, _, window, cx| { if enabled { w.choose_toolbar_popup(index, window, cx); } })));
            }
        }
        let placement = if kind == ToolbarPopupKind::Width { PopupPlacement::BelowRight } else { PopupPlacement::BelowLeft };
        Some(gpui::deferred(popup(("toolbar-popup-open", kind.anchor_index()),
            PopupAnchor::Tracked(self.toolbar_popup_anchors[kind.anchor_index()].clone(), position),
            placement, panel, self.explorer.reduce_motion)).with_priority(5).into_any_element())
    }

}
