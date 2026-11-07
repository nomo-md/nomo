//! 代码块头部与块内滚动。输入框、订阅仅在展开期间存在，复制反馈最多一个短时任务。
use super::*;
use crate::nomo_mark::code_blocks::{self, FencedCode};
use crate::ui::{components::{icon, tooltip, IconName}, popup::{self, PopupAnchor, PopupPlacement}, text_field::{FieldEvent, TextField}};

pub(super) struct LanguagePicker {
    identity: WindowIdentity,
    source: usize,
    field: Entity<TextField>,
    _changed: gpui::Subscription,
    _blur: gpui::Subscription,
}

impl LanguagePicker {
    pub(super) fn is_stale(&self, identity: &WindowIdentity) -> bool { &self.identity != identity }
    pub(super) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + std::mem::size_of::<TextField>() + 256
            + self.identity.document_id.capacity() + self.identity.window_id.capacity()
    }
}

pub(super) struct CopyFeedback {
    identity: WindowIdentity,
    source: usize,
    success: bool,
    started: std::time::Instant,
    _timer: gpui::Task<()>,
}

impl CopyFeedback {
    pub(super) fn is_stale(&self, identity: &WindowIdentity) -> bool { &self.identity != identity }
    pub(super) fn estimated_bytes(&self) -> usize { std::mem::size_of::<Self>() + self.identity.document_id.capacity() + self.identity.window_id.capacity() }
}

impl NomoMark {
    fn close_code_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.code_language = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn open_code_language(&mut self, source: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.code_language.as_ref().is_some_and(|picker| picker.source == source) {
            self.close_code_language(window, cx);
            return;
        }
        if self.host.presentation != Presentation::Editable || self.host.interaction.protected() { return; }
        let Some(code) = FencedCode::at(&self.host.parsed, source) else { return; };
        let language = &self.host.parsed.source[code.language];
        let mut end = language.len().min(256);
        while !language.is_char_boundary(end) { end -= 1; }
        let language = &language[..end];
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let field = cx.new(|cx| {
            let mut field = TextField::new(locale.text("code_language_search"), cx);
            field.set_code_language_style();
            field.set_text(language, cx);
            field
        });
        let changed = cx.subscribe_in(&field, window, |view, field, event, window, cx| {
            match event {
                FieldEvent::Changed => {
                    // 搜索状态有明确上限，不保留任意大小的剪贴板文本。
                    let value = &field.read(cx).content;
                    if value.len() > 256 {
                        let mut end = 256;
                        while !value.is_char_boundary(end) { end -= 1; }
                        let value = value[..end].to_string();
                        field.update(cx, |field, cx| field.set_text(&value, cx));
                    }
                    cx.notify();
                }
                FieldEvent::Submit { .. } => {
                    let query = field.read(cx).content.trim().to_string();
                    let lower = query.to_ascii_lowercase();
                    let value = code_blocks::LANGUAGES.iter().find(|entry| code_blocks::language_matches(entry, &lower))
                        .map_or(query.as_str(), |entry| entry.1).to_string();
                    view.apply_code_language(&value, window, cx);
                }
                FieldEvent::Cancel => view.close_code_language(window, cx),
            }
        });
        let focus = field.focus_handle(cx);
        let blur = cx.on_focus_out(&focus, window, move |view, _, _, cx| {
            if view.code_language.as_ref().is_some_and(|picker| picker.source == source) {
                view.code_language = None;
                cx.notify();
            }
        });
        self.dismiss_quick_insert(cx);
        self.code_language = Some(LanguagePicker {
            identity: self.host.parsed.identity.clone(), source, field, _changed: changed, _blur: blur,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn apply_code_language(&mut self, language: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.code_language.as_ref() else { return; };
        if picker.identity != self.host.parsed.identity {
            self.close_code_language(window, cx);
            self.error(ControlError::Stale, cx);
            return;
        }
        let language = language.trim();
        if language.is_empty() || language.len() > 256 || language.chars().any(char::is_whitespace) || language.contains('`') { return; }
        let Some(code) = FencedCode::at(&self.host.parsed, picker.source) else { return; };
        if &self.host.parsed.source[code.language.clone()] == language {
            self.close_code_language(window, cx);
            return;
        }
        let base = picker.identity.from_byte;
        let edits = vec![SourceEdit { from_byte: base + code.language.start as u64,
            to_byte: base + code.language.end as u64, inserted_text: language.into() }];
        let selection = commands::map_selection(self.host.selection, &edits);
        self.history_serial += 1;
        let group = format!("{}:language:{}", picker.identity.window_id, self.history_serial);
        let result = commands::EditResult { edits, selection, template_update: None };
        if let Err(error) = self.queue_result(result, &group, cx) { self.error(error, cx); }
        self.close_code_language(window, cx);
    }

    pub(super) fn code_language_controls(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.effective_mode() != ViewMode::Semantic || self.auxiliary_input || self.object.is_some()
            || self.host.presentation != Presentation::Editable || self.host.interaction.protected() { return None; }
        let layout = self.layout.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let mut overlay = div().absolute().top_0().left_0().w_full().h(px(0.0));
        for item in &layout.items {
            let DisplayItem::CodeLanguage { x, y, w, h, source } = item else { continue; };
            let Some(code) = FencedCode::at(&self.host.parsed, *source) else { continue; };
            let current = &self.host.parsed.source[code.language];
            let source = *source;
            let mut trigger = div().id(("code-language", source)).absolute().left(px(*x)).top(px(*y)).w(px(*w)).h(px(*h))
                .cursor_pointer().tooltip(tooltip(locale.text("code_language_choose")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, window, cx| { cx.stop_propagation(); view.open_code_language(source, window, cx); }));
            if let Some(picker) = self.code_language.as_ref().filter(|picker| picker.source == source && picker.identity == self.host.parsed.identity) {
                let query = picker.field.read(cx).content.trim();
                let lower = query.to_ascii_lowercase();
                let mut list = div().id("code-language-list").flex().flex_col().min_h(px(0.0)).max_h(px(209.0))
                    .flex_shrink(1.0).overflow_y_scroll().py(px(4.0));
                for entry in code_blocks::LANGUAGES.iter().filter(|entry| code_blocks::language_matches(entry, &lower)) {
                    let (label, value, aliases) = *entry;
                    let selected = value.eq_ignore_ascii_case(current) || aliases.iter().any(|alias| alias.eq_ignore_ascii_case(current));
                    list = list.child(div().id(value).flex_none().px(px(10.0)).py(px(4.0)).cursor_pointer()
                        .when(selected, |row| row.bg(theme.accent.opacity(0.20)).font_weight(gpui::FontWeight::SEMIBOLD))
                        .hover(|row| row.bg(theme.accent.opacity(0.15))).child(label)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, window, cx| { cx.stop_propagation(); view.apply_code_language(value, window, cx); })));
                }
                if !query.is_empty() && !query.chars().any(char::is_whitespace) && !query.contains('`')
                    && !code_blocks::LANGUAGES.iter().any(|entry| entry.1 == lower) {
                    let custom = query.to_string();
                    list = list.child(div().id("code-language-custom").flex_none().px(px(10.0)).py(px(4.0)).cursor_pointer()
                        .text_color(theme.muted).hover(|row| row.bg(theme.accent.opacity(0.15)))
                        .child(locale.text("code_language_use").replace("{language}", query))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, window, cx| { cx.stop_propagation(); view.apply_code_language(&custom, window, cx); })));
                }
                let trigger_bounds = self.content_bounds().map(|bounds| Bounds::new(bounds.origin + point(px(*x), px(*y)), size(px(*w), px(*h))));
                let panel = div().id("code-language-panel").w(px(200.0)).max_h(px(240.0)).flex().flex_col()
                    .text_size(px(12.0)).text_color(theme.fg).bg(theme.surface).border_1().border_color(theme.code_border)
                    .font_family(crate::theme::font_for(Family::Code, false, false).family)
                    .rounded(px(4.0)).shadow_md().occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_mouse_down_out(cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                        if !trigger_bounds.is_some_and(|bounds| bounds.contains(&event.position)) {
                            view.code_language = None;
                            cx.notify();
                        }
                    }))
                    .child(div().flex_none().border_b_1().border_color(theme.code_border).child(picker.field.clone()))
                    .child(list);
                trigger = trigger.child(gpui::deferred(popup::popup_with_list("code-language-popup", PopupAnchor::Parent,
                    PopupPlacement::BelowLeft, panel, popup::reduced_motion(cx))).with_priority(4));
            }
            overlay = overlay.child(trigger);
        }
        Some(overlay.into_any_element())
    }

    pub(super) fn code_header_controls(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.effective_mode() != ViewMode::Semantic || self.auxiliary_input || self.object.is_some()
            || !matches!(self.host.presentation, Presentation::Editable | Presentation::ReadOnly) { return None; }
        if self.layout_key.as_ref().is_none_or(|key| key.0.identity != self.host.parsed.identity) { return None; }
        let layout = self.layout.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let zoom = cx.global::<Settings>().metrics().zoom;
        let mut overlay = div().absolute().top_0().left_0().w_full().h(px(0.0));
        for item in &layout.items {
            let DisplayItem::CodeHeader { x, y, w, h, source, text, expanded } = item else { continue; };
            let source = *source;
            let identity = self.host.parsed.identity.clone();
            let feedback = self.code_copy.as_ref().filter(|state| state.source == source && state.identity == identity);
            let color = feedback.map_or(theme.code_fg, |state| if state.success { theme.accent } else { theme.danger });
            let border = feedback.map_or(theme.code_border, |state| if state.success { theme.accent.opacity(0.7) } else { theme.danger });
            let background = if feedback.is_some_and(|state| state.success) { crate::ui::motion::color(theme.code_bg, theme.accent, 0.16) }
                else { crate::ui::motion::color(theme.code_bg, theme.surface, 0.18) };
            let hint = match feedback { Some(state) if state.success => "code_copied", Some(_) => "code_copy_failed", None => "code_copy" };
            let text = text.clone();
            let button = div().id(("code-copy", source)).absolute().left(px(*x)).top(px(*y)).w(px(*w)).h(px(*h))
                .flex().items_center().justify_center().border_1().border_color(border).rounded(px(4.0*zoom)).bg(background)
                .cursor_pointer().tooltip(tooltip(locale.text(hint)))
                .child(icon(if feedback.is_some_and(|state| state.success) { IconName::Check } else { IconName::Copy }, 16.0*zoom, color))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    if view.host.parsed.identity != identity { return; }
                    view.copy_code(source, &text, cx);
                }));
            overlay = overlay.child(button);
            if let Some(open) = expanded {
                let open = *open;
                let identity = self.host.parsed.identity.clone();
                overlay = overlay.child(div().id(("code-expand", source)).absolute().left(px(x-w-6.0*zoom)).top(px(*y)).w(px(*w)).h(px(*h))
                    .flex().items_center().justify_center().border_1().border_color(theme.code_border).rounded(px(4.0*zoom))
                    .bg(crate::ui::motion::color(theme.code_bg, theme.surface, 0.18))
                    .cursor_pointer().tooltip(tooltip(locale.text(if open { "code_collapse" } else { "code_expand" })))
                    .hover(|button| button.border_color(crate::ui::motion::color(theme.code_border, theme.accent, 0.55)).bg(crate::ui::motion::color(theme.code_bg, theme.accent, 0.12)))
                    .child(gpui::svg().path(if open { "icons/chevrons-up.svg" } else { "icons/chevrons-down.svg" }).size(px(14.0*zoom)).text_color(theme.code_fg))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        if view.host.parsed.identity != identity || view.host.interaction.protected() { return; }
                        let at = identity.from_byte+source as u64;
                        view.host.interaction.details_open.insert(at, !open);
                        if let Some(offset) = view.host.interaction.code_offsets.get_mut(&at) { offset.1 = 0.0; }
                        view.invalidate();
                        cx.notify();
                    })));
            }
        }
        Some(overlay.into_any_element())
    }

    fn copy_code(&mut self, source: usize, text: &str, cx: &mut Context<Self>) {
        self.code_copy = None;
        let result = crate::clipboard::write_text(text, cx);
        let success = result.is_ok();
        let started = std::time::Instant::now();
        let identity = self.host.parsed.identity.clone();
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(if success { 1200 } else { 1500 })).await;
            let _ = this.update(cx, |view, cx| {
                if view.code_copy.as_ref().is_some_and(|state| state.started == started) { view.code_copy = None; cx.notify(); }
            });
        });
        self.code_copy = Some(CopyFeedback { identity, source, success, started, _timer: timer });
        if let Err(error) = result { cx.emit(ControlEvent::Failure(error)); }
        cx.notify();
    }

    pub(super) fn scroll_code(&mut self, event: &gpui::ScrollEvent, cx: &mut Context<Self>) -> bool {
        if self.host.interaction.composing || self.host.interaction.draft { return false; }
        if self.layout_key.as_ref().is_none_or(|key| key.0.identity != self.host.parsed.identity) { return false; }
        let (Some(bounds), Some(layout)) = (self.bounds, &self.layout) else { return false; };
        let x = f32::from(event.position.x-bounds.left()); let y = f32::from(event.position.y-bounds.top());
        let delta = event.delta.pixel_delta(px(20.0));
        for item in &layout.items {
            let DisplayItem::CodeBody { x: ix, y: iy, w, h, source, body } = item else { continue; };
            if x < *ix || x > ix+w || y < *iy || y > iy+h { continue; }
            let old = body.offset.get();
            let dx = if event.modifiers.shift && f32::from(delta.x).abs() < 0.01 { f32::from(delta.y) } else { f32::from(delta.x) };
            let dy = if event.modifiers.shift { 0.0 } else { f32::from(delta.y) };
            let next = body.clamp_offset(*w, *h, (old.0-dx, old.1-dy));
            if next == old { return false; }
            body.offset.set(next);
            self.host.interaction.code_offsets.insert(self.host.parsed.identity.from_byte+source.start as u64, next);
            cx.stop_propagation(); cx.notify(); return true;
        }
        false
    }

    pub(super) fn code_scroll_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        if self.layout_key.as_ref().is_none_or(|key| key.0.identity != self.host.parsed.identity) { return false; }
        let (Some(bounds), Some(layout)) = (self.bounds, &self.layout) else { return false; };
        if self.host.interaction.protected() { return false; }
        let x = f32::from(event.position.x-bounds.left()); let y = f32::from(event.position.y-bounds.top());
        for item in &layout.items {
            let DisplayItem::CodeBody { x: ix, y: iy, w, h, source, body } = item else { continue; };
            let (width, height) = body.viewport(*w, *h);
            if x < ix+body.gutter || x > ix+w || y < *iy || y > iy+h { continue; }
            let vertical = x >= ix+body.gutter+width && body.code.height > height;
            if !vertical && !(y >= iy+height && body.content_width > width) { continue; }
            let (position, origin, visible, total) = if vertical { (y, *iy, height, body.code.height) } else { (x, ix+body.gutter, width, body.content_width) };
            let old = body.offset.get();
            let current = if vertical { old.1 } else { old.0 };
            let ratio = visible/total;
            let thumb_start = origin+current*ratio;
            let value = if position >= thumb_start && position <= thumb_start+visible*ratio { current }
                else { ((position-origin)/ratio-visible/2.0).clamp(0.0, (total-visible).max(0.0)) };
            let next = if vertical { (old.0, value) } else { (value, old.1) };
            body.offset.set(next);
            self.host.interaction.code_offsets.insert(self.host.parsed.identity.from_byte+source.start as u64, next);
            self.code_scroll_drag = Some((source.start, vertical, position, value));
            self.host.interaction.dragging = true;
            cx.stop_propagation(); cx.notify(); return true;
        }
        false
    }

    pub(super) fn code_scroll_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some((source, vertical, start, initial)) = self.code_scroll_drag else { return false; };
        let (Some(bounds), Some(layout)) = (self.bounds, &self.layout) else { return true; };
        for item in &layout.items {
            let DisplayItem::CodeBody { source: range, w, h, body, .. } = item else { continue; };
            if range.start != source { continue; }
            let (width, height) = body.viewport(*w, *h);
            let (position, visible, total) = if vertical { (f32::from(event.position.y-bounds.top()), height, body.code.height) }
                else { (f32::from(event.position.x-bounds.left()), width, body.content_width) };
            let value = (initial+(position-start)*total/visible).clamp(0.0, (total-visible).max(0.0));
            let old = body.offset.get();
            let next = if vertical { (old.0, value) } else { (value, old.1) };
            body.offset.set(next);
            self.host.interaction.code_offsets.insert(self.host.parsed.identity.from_byte+source as u64, next);
            cx.notify(); break;
        }
        true
    }

    pub(super) fn move_in_code(&mut self, down: bool, extend: bool, page: bool, cx: &mut Context<Self>) -> bool {
        let Some(layout) = &self.layout else { return false; };
        let at = self.local_cursor();
        for item in &layout.items {
            let DisplayItem::CodeBody { x, source, w, h, body, .. } = item else { continue; };
            if !source.contains(&at) { continue; }
            let Some((sx, sy, line)) = body.code.caret_at(at) else { return false; };
            let old = body.offset.get();
            let screen_x = self.preferred_x.unwrap_or(x+body.gutter+sx-old.0);
            let distance = if page { (body.viewport(*w, *h).1-line).max(line) } else { line };
            let target_y = sy+line/2.0+if down { distance } else { -distance };
            let Some(target) = body.code.source_at(screen_x-x-body.gutter+old.0, target_y) else { return false; };
            // 首末行之外跳过卡片内边距，交回相邻正文或宿主窗口导航。
            let Some((_, row, _)) = body.code.caret_at(target) else { return false; };
            if (!down && row >= sy) || (down && row <= sy) {
                let Some(node) = self.host.parsed.nodes.iter().find(|node| node.kind == "code" && node.source.start == source.start) else { return false; };
                let edge = if down { node.source.end.saturating_sub(1) } else { node.source.start };
                if let Some(next) = layout.horizontal_caret(edge, crate::nomo_mark::navigation::CursorUnit::Grapheme, down)
                    .filter(|next| if down { *next >= node.source.end } else { *next < node.source.start }) {
                    self.move_to(next, extend, cx);
                } else {
                    cx.emit(ControlEvent::WindowBoundary { identity: self.host.parsed.identity.clone(), forward: down, extend_selection: extend });
                }
                return true;
            }
            self.preferred_x = Some(screen_x);
            self.move_to(target, extend, cx);
            return true;
        }
        false
    }
}
