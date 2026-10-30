//! 当前窗口内的标题标识、代码语言与结构块菜单。
use super::*;
use crate::ui::components::{icon, tooltip, IconName};
use gpui::{AnyElement, ClickEvent};
use crate::ui::popup::{self, PopupAnchor, PopupPlacement};

#[derive(Clone, Copy)]
struct HeadingAnchor {
    source: u64,
    level: u8,
    top: f32,
}

/// 仅保留当前角标的几何和短时动效，不持有正文、布局或视图引用。
#[derive(Default)]
pub(super) struct HeadingIndicator {
    anchor: Option<HeadingAnchor>,
    motion: crate::ui::motion::ScalarMotion,
    visible: bool,
}

/// 以本帧的块内位置为锚点；延后绘制避免被文档滚动区域裁切。
pub(super) fn block_popup(
    id: &'static str, x: f32, y: f32, panel: gpui::Stateful<gpui::Div>,
    placement: PopupPlacement, priority: usize, cx: &gpui::App,
) -> AnyElement {
    div().absolute().left(px(x)).top(px(y)).w(px(1.0)).h(px(0.0))
        .child(gpui::deferred(popup::popup(id, PopupAnchor::Parent, placement, panel, popup::reduced_motion(cx)))
            .with_priority(priority))
        .into_any_element()
}

impl NomoMark {
    pub(super) fn prepare_heading_indicator(&mut self, bounds: Bounds<Pixels>, window: &Window, cx: &App) {
        let zoom = cx.global::<Settings>().metrics().zoom;
        if self.effective_mode() != ViewMode::Semantic || self.host.presentation != Presentation::Editable
            || self.auxiliary_input || self.object.is_some()
            || self.layout_key.as_ref().is_none_or(|key| key.0.identity != self.host.parsed.identity)
            || bounds.left() - window.content_mask().bounds.left() < px(48.0 * zoom)
        {
            self.heading_indicator = Default::default();
            return;
        }
        let parsed = &self.host.parsed;
        let next = self.focus.is_focused(window).then(|| {
            let selection = local_range(&parsed.source, parsed.identity.from_byte, self.host.selection.range()).ok()?;
            let (level, range) = parsed.events.iter().find_map(|(event, range)| {
                let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Heading { level, .. }) = event else { return None; };
                let end = range.start + parsed.source[range.clone()].trim_end_matches(['\r', '\n']).len();
                (range.start <= selection.start && selection.end <= end).then_some((*level as u8, range))
            })?;
            // 隐藏的 # 位于正文映射之外；使用标题自己的首个文本项，不能向最近光标位置回退。
            let (top, line_height) = self.layout.as_ref()?.items.iter().find_map(|item| {
                let DisplayItem::Text { y, layout, map, .. } = item else { return None; };
                map.iter().any(|segment| range.start <= segment.source && segment.source_end <= range.end
                    && (segment.source < range.end || range.end == parsed.source.len()))
                    .then_some((*y, layout.line_height))
            })?;
            Some(HeadingAnchor { source: parsed.identity.from_byte + range.start as u64, level,
                top: top + ((line_height - 18.0 * zoom) / 2.0).max(0.0) })
        }).flatten();
        let indicator = &mut self.heading_indicator;
        let reduced = popup::reduced_motion(cx);
        if let Some(anchor) = next {
            if !indicator.visible || indicator.anchor.is_none_or(|old| old.source != anchor.source || old.level != anchor.level) {
                indicator.motion.reset();
            }
            indicator.anchor = Some(anchor);
        }
        indicator.visible = next.is_some();
        if reduced {
            indicator.motion = crate::ui::motion::ScalarMotion::settled(indicator.visible);
        } else {
            indicator.motion.set(indicator.visible, false,
                std::time::Duration::from_millis(if indicator.visible { 140 } else { 100 }));
        }
        if !indicator.visible && !indicator.motion.running() { indicator.anchor = None; }
    }

    pub(super) fn paint_heading_indicator(&self, bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window, cx: &App) {
        let indicator = &self.heading_indicator;
        let Some(anchor) = indicator.anchor else { return; };
        let zoom = cx.global::<Settings>().metrics().zoom;
        let height = px(18.0 * zoom);
        let top = bounds.top() + px(anchor.top);
        let clip = window.content_mask().bounds;
        if top + height <= clip.top() || top >= clip.bottom() { return; }
        if indicator.motion.running() { window.request_animation_frame(); }
        let opacity = indicator.motion.value();
        let shift = if popup::reduced_motion(cx) { 0.0 } else {
            -(if indicator.visible { 6.0 } else { 4.0 }) * (1.0 - opacity) * zoom
        };
        let label = ["H1", "H2", "H3", "H4", "H5", "H6"][usize::from(anchor.level - 1)];
        let font_size = px(10.0 * zoom);
        let color = theme.accent.opacity(opacity);
        let run = gpui::TextRun { len: label.len(), font: crate::theme::font_for(Family::Mono, true, false),
            color, background_color: None, underline: None, strikethrough: None };
        let line = window.text_system().layout_line(label, font_size, &[run], None);
        let tracking = px(0.5 * zoom);
        let origin = point(bounds.left() - px(14.0 * zoom) - line.width - tracking * 2.0 + px(shift),
            top + (height - line.ascent - line.descent) / 2.0 + line.ascent);
        // 两个 ASCII 字符按旧版 0.05em 字距绘制；不注册命中区域，也不参与正文选区。
        for run in &line.runs {
            for glyph in &run.glyphs {
                let at = origin + glyph.position + point(tracking * glyph.index as f32, px(0.0));
                let _ = window.paint_glyph(at, run.font_id, glyph.id, font_size, color);
            }
        }
    }

    pub(super) fn active_code(&self) -> Option<usize> {
        let at = self.local_cursor();
        if let Some(code) = super::super::code_blocks::FencedCode::at(&self.host.parsed, at) {
            if code.body.start <= at && at <= code.body.end { return Some(code.source); }
        }
        self.host.parsed.nodes.iter().find(|n| n.kind == "code" && n.source.contains(&at)
            && self.host.parsed.source[n.source.start..at].contains('\n')).map(|n| n.source.start)
    }
    fn replace_node(&mut self, identity: &WindowIdentity, range: Range<usize>, text: String, cx: &mut Context<Self>) {
        if &self.host.parsed.identity != identity { self.error(ControlError::Stale, cx); return; }
        let base = identity.from_byte;
        let edits = vec![SourceEdit { from_byte: base + range.start as u64, to_byte: base + range.end as u64, inserted_text: text }];
        let result = commands::EditResult { selection: commands::map_selection(self.host.selection, &edits), edits, template_update: None };
        self.history_serial += 1;
        let group = format!("{}:node:{}", identity.window_id, self.history_serial);
        if let Err(error) = self.queue_result(result, &group, cx) { self.error(error, cx); }
    }
    pub(super) fn node_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.effective_mode() != ViewMode::Semantic || self.auxiliary_input || self.object.is_some() { return None; }
        let layout = self.layout.as_ref()?;
        let parsed = &self.host.parsed;
        let at = self.local_cursor();
        let theme = cx.global::<Theme>().clone();
        let mut overlay = div().absolute().top_0().left_0().w_full().h(px(0.0));
        for item in &layout.items {
            let DisplayItem::TocLink { x, y, w, h, target, source } = item else { continue; };
            let target = target.clone();
            overlay = overlay.child(div().id(("node-link", *source)).absolute().left(px(*x)).top(px(*y)).w(px(*w)).h(px(*h))
                .rounded(px(4.0)).cursor_pointer().hover(|style| style.bg(theme.hover.opacity(0.4)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(ControlEvent::OpenLink(target.clone())))));
        }
        let references = parsed.events.iter().filter_map(|(event, range)| {
            if let pulldown_cmark::Event::FootnoteReference(label) = event { Some((range, label.as_ref())) } else { None }
        }).chain(parsed.html_cells().flat_map(|cell| &cell.runs).filter_map(|run| {
            if let Some(super::super::html_table::InlineAtom::Footnote(label)) = &run.atom { Some((&run.source, label.as_str())) } else { None }
        }));
        for (range, label) in references {
                    let preview = parsed.events.iter().find_map(|(event, range)| {
                        matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::FootnoteDefinition(id)) if id.as_ref() == label)
                            .then(|| parsed.source[range.clone()].split_once("]:").map_or("", |(_, text)| text).trim().chars().take(360).collect::<String>())
                    }).unwrap_or_else(|| format!("脚注 {label} · 点击查看"));
                    for (i, (x, y, w, h)) in layout.source_rects_clipped(range.clone(), 0.0..layout.height).into_iter().enumerate() {
                        let target = format!("#fn-{}", label.replace('%', "%25"));
                        let owner = cx.entity().downgrade();
                        let identity = parsed.identity.clone();
                        let label = label.to_string();
                        let preview = preview.clone();
                        overlay = overlay.child(div().id(format!("footnote-{}-{i}", range.start)).absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h))
                            .cursor_pointer().tooltip(move |_, cx| {
                                let tooltip = cx.new(|_| crate::nomo_mark::footnote_preview::FootnotePreview { text: preview.clone(), request: Default::default() });
                                let _ = owner.update(cx, |_, cx| cx.emit(ControlEvent::FootnotePreview {
                                    identity: identity.clone(), label: label.clone(), tooltip: tooltip.downgrade(),
                                }));
                                tooltip.into()
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(ControlEvent::OpenLink(target.clone())))));
                    }
        }
        if self.host.presentation != Presentation::Editable || self.host.interaction.protected() { return Some(overlay.into_any_element()); }
        let zoom = cx.global::<Settings>().metrics().zoom;
        for item in &layout.items {
            let DisplayItem::BlockControls { x, y, w, source, kind: "diagram-toolbar", .. } = item else { continue; };
            let Some(node) = parsed.nodes.iter().find(|n| n.kind == "code" && n.source.contains(&source.start)) else { continue; };
            let range = node.source.clone();
            overlay = overlay.child(div().id(("diagram-toolbar", range.start)).absolute().left(px(*x)).top(px(*y)).w(px(*w))
                .h(px(32.0 * zoom)).flex().justify_end()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().id(("diagram-fullscreen", range.start)).size(px(32.0 * zoom)).flex_shrink_0()
                    .flex().items_center().justify_center().rounded(px(3.0 * zoom)).border_1().border_color(theme.border)
                    .bg(theme.surface).hover(|s| s.bg(theme.hover)).cursor_pointer().tooltip(tooltip("放大查看"))
                    .child(gpui::svg().path("icons/maximize.svg").size(px(16.0 * zoom)).text_color(theme.muted))
                    .on_click(cx.listener(move |view, _, window, cx| view.open_diagram_viewer(range.clone(), window, cx)))));
        }
        for item in &layout.items {
            let DisplayItem::BlockControls { x, y, w, source, kind, .. } = item else { continue; };
            if matches!(*kind, "rule" | "diagram-toolbar") { continue; }
            let mut buttons = div().absolute().left(px(x + w - 62.0)).top(px(y + 8.0)).flex().gap(px(4.0));
            for (action, label, glyph) in [(0, "编辑", IconName::SquarePen), (1, "删除", IconName::Trash2)] {
                if *kind == "toc" && action == 0 { continue; }
                let identity = parsed.identity.clone(); let source = source.clone();
                let front_matter = *kind == "front-matter" && action == 1;
                let delete_key = (identity.revision, identity.view_generation);
                let confirming = front_matter && self.front_matter_delete == Some(delete_key);
                buttons = buttons.child(div().id(format!("block-{kind}-{action}-{}", source.start)).size(px(24.0))
                    .flex().items_center().justify_center().cursor_pointer().rounded(px(4.0)).hover(|s| s.bg(theme.hover))
                    .tooltip(tooltip(if confirming { "再次点击确认删除文档属性" } else { label }))
                    .when(confirming, |button| button.bg(theme.selection))
                    .child(icon(glyph, 14.0, if confirming { theme.accent } else { theme.muted }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |v, _, window, cx| {
                        if v.host.parsed.identity != identity { v.error(ControlError::Stale, cx); return; }
                        if front_matter && v.front_matter_delete != Some(delete_key) { v.front_matter_delete = Some(delete_key); cx.notify(); return; }
                        v.front_matter_delete = None;
                        if action == 0 { v.begin_object(source.clone(), super::super::widgets::ObjectKind::Html, window, cx); }
                        else { v.replace_node(&identity, source.clone(), String::new(), cx); window.focus(&v.focus, cx); }
                    })));
            }
            overlay = overlay.child(buttons);
        }
        for item in &layout.items {
            let DisplayItem::CalloutHeader { x, y, size, button_offset, source, role, kind } = item else { continue; };
            let start = *source;
            let zoom = size / 16.0;
            let Some(quote) = parsed.nodes.iter().find(|n| n.kind == "quote" && n.source.start == start) else { continue; };
            let active = quote.source.start <= at && at <= quote.source.end;
            let open = self.block_menu_open && active;
            let body = layout.items.iter().find_map(|item| {
                let DisplayItem::Text { map, .. } = item else { return None; };
                map.iter().find(|segment| start < segment.source && segment.source <= quote.source.end).map(|segment| segment.source)
            }).unwrap_or(start);
            let color = role.resolve(&theme);
            overlay = overlay.child(div().id(("callout-type", start)).absolute().left(px(x + button_offset)).top(px(y - 4.0 * zoom))
                .size(px(24.0 * zoom)).flex().items_center().justify_center().rounded(px(4.0 * zoom))
                .text_size(px(12.0 * zoom)).text_color(color).opacity(if open { 1.0 } else { 0.6 })
                .when(open, |s| s.bg(color.opacity(0.12)))
                .cursor_pointer().hover(move |s| s.opacity(1.0).bg(color.opacity(0.1))).child("▾")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |v, _, _, cx| { if !active { v.move_to(body, false, cx); } v.block_menu_open = !open; cx.notify(); })));
            if open {
                let line_end = parsed.source[start..].find('\n').map_or(parsed.source.len(), |n| start + n);
                if let Some(open) = parsed.source[start..line_end].find("[!") {
                    let from = start + open + 2;
                    if let Some(end) = parsed.source[from..line_end].find(']') {
                        let mut menu = div().id("callout-types").w(px(156.0 * zoom)).text_size(px(13.0 * zoom)).text_color(theme.fg)
                            .p(px(4.0 * zoom)).bg(theme.surface).border_1().border_color(theme.border).rounded(px(8.0 * zoom));
                        for (name, label, path, value) in [
                            ("NOTE", "提醒", IconName::Info.path(), pulldown_cmark::BlockQuoteKind::Note),
                            ("TIP", "建议", "icons/callout-tip.svg", pulldown_cmark::BlockQuoteKind::Tip),
                            ("IMPORTANT", "重要", "icons/callout-important.svg", pulldown_cmark::BlockQuoteKind::Important),
                            ("WARNING", "警告", IconName::AlertTriangle.path(), pulldown_cmark::BlockQuoteKind::Warning),
                            ("CAUTION", "风险", "icons/callout-caution.svg", pulldown_cmark::BlockQuoteKind::Caution),
                        ] {
                            let identity = parsed.identity.clone();
                            menu = menu.child(div().id(name).flex().items_center().gap(px(8.0 * zoom)).px(px(10.0 * zoom)).py(px(6.0 * zoom)).rounded(px(4.0 * zoom))
                                .when(*kind == value, |s| s.bg(theme.accent.opacity(0.15)).font_weight(gpui::FontWeight::SEMIBOLD))
                                .cursor_pointer().hover(|s| s.bg(theme.selection)).child(gpui::svg().path(path).size(px(16.0 * zoom)).text_color(theme.fg)).child(label)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |v, _, window, cx| { v.block_menu_open = false; v.replace_node(&identity, from..from + end, name.into(), cx); window.focus(&v.focus, cx); })));
                        }
                        overlay = overlay.child(block_popup("callout-types-open", x + 22.0 * zoom, y + 26.0 * zoom, menu, PopupPlacement::BelowLeft, 4, cx));
                    }
                }
            }
        }
        if let Some(block) = parsed.details.iter().filter(|b| b.source.start <= at && at < b.source.end).min_by_key(|b| b.source.len()) {
            let zoom = cx.global::<crate::settings::Settings>().metrics().zoom;
            if let Some((x, y, _)) = layout.caret_at(block.title(&parsed.source).start) {
                let right = layout.items.iter().filter_map(|item| match item {
                    DisplayItem::Rect { x: left, y: top, w, h, border: Some(_), .. } if *left <= x && x < left + w && *top <= y && y < top + h => Some(left + w),
                    _ => None,
                }).min_by(f32::total_cmp).unwrap_or_else(|| self.bounds.map_or(x + 28.0 * zoom, |bounds| f32::from(bounds.size.width)));
                let menu_x = (right - 28.0 * zoom).max(x);
                overlay = overlay.child(div().id("details-menu-button").absolute().left(px(menu_x)).top(px(y))
                    .size(px(22.0 * zoom)).flex().items_center().justify_center().text_color(theme.muted)
                    .cursor_pointer().tooltip(tooltip("折叠块选项")).child("⋯")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|v, _, _, cx| { v.block_menu_open = !v.block_menu_open; cx.notify(); })));
                if self.block_menu_open {
                    let mut menu = div().id("details-menu").w(px(170.0)).p(px(5.0)).bg(theme.surface).border_1().border_color(theme.border).rounded(px(5.0));
                    let opening_end = parsed.source[block.source.clone()].find('>').map(|p| block.source.start + p + 1)?;
                    for (i, (label, range, text)) in [
                        (if block.default_open { "默认折叠" } else { "默认展开" }, block.source.start..opening_end, if block.default_open { "<details>" } else { "<details open>" }.to_string()),
                        ("取消折叠块", block.source.clone(), block.unwrapped(&parsed.source)),
                        ("删除折叠块", block.source.clone(), String::new()),
                    ].into_iter().enumerate() {
                        let identity = parsed.identity.clone();
                        menu = menu.child(div().id(("details-option", i)).px(px(8.0)).py(px(5.0)).cursor_pointer().hover(|s| s.bg(theme.hover)).child(label)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |v, _: &ClickEvent, window, cx| { v.block_menu_open = false; v.replace_node(&identity, range.clone(), text.clone(), cx); window.focus(&v.focus, cx); })));
                    }
                    overlay = overlay.child(block_popup("details-menu-open", x, y + 20.0, menu, PopupPlacement::BelowLeft, 4, cx));
                }
            }
        }
        Some(overlay.into_any_element())
    }
}
