//! 按需加载并虚拟化显示 Emoji 词库；选择器关闭后释放目录引用和订阅。
use crate::editor_control::{document_host::shared_emoji_catalog, emoji::EmojiCatalog};
use crate::theme::Theme;
use crate::ui::{components::{IconName, icon, tooltip}, text_field::{FieldEvent, TextField}};
use gpui::{App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, ScrollStrategy, Subscription, Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list};
use std::sync::Arc;

pub enum EmojiPickerEvent { Choose(String), Dismiss }

pub struct EmojiPicker {
    focus: FocusHandle,
    trigger_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    search: Entity<TextField>,
    catalog: Option<Arc<EmojiCatalog>>,
    results: Arc<Vec<usize>>,
    group: Option<u8>,
    selected: usize,
    scroll: UniformListScrollHandle,
    error: Option<String>,
    loading: bool,
    task: Option<Task<()>>,
    _subscription: Subscription,
}
impl EventEmitter<EmojiPickerEvent> for EmojiPicker {}
impl Focusable for EmojiPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.search.read(cx).focus_handle(cx) }
}
impl EmojiPicker {
    pub fn new(trigger_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>, cx: &mut Context<Self>) -> Self {
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let search = cx.new(|cx| { let mut field = TextField::new(locale.text("emoji_search"), cx); field.set_emoji_search_style(cx); field });
        let subscription = cx.subscribe(&search, |picker, _, event, cx| match event {
            FieldEvent::Changed => picker.refresh(cx),
            FieldEvent::Submit { .. } if !picker.search.read(cx).is_composing() => picker.choose(cx),
            FieldEvent::Cancel if !picker.search.read(cx).is_composing() => cx.emit(EmojiPickerEvent::Dismiss),
            _ => {},
        });
        let mut picker = Self { focus: cx.focus_handle(), trigger_bounds, search, catalog: None, results: Arc::new(Vec::new()), group: None, selected: 0, scroll: UniformListScrollHandle::new(), error: None, loading: false, task: None, _subscription: subscription };
        picker.load(cx);
        picker
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true; self.error = None;
        let task = cx.background_executor().spawn(async move {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
            runtime.block_on(shared_emoji_catalog())
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |picker, cx| {
                picker.loading = false;
                match result { Ok(catalog) => { picker.catalog = Some(catalog); picker.refresh(cx); }, Err(error) => picker.error = Some(error) }
                cx.notify();
            });
        }));
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = &self.search.read(cx).content;
        let indices = self.catalog.as_ref().map_or_else(Vec::new, |catalog| catalog.search_indices(query, self.group, usize::MAX));
        self.results = Arc::new(indices);
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }
    fn choose(&self, cx: &mut Context<Self>) {
        if let Some(entry) = self.results.get(self.selected).and_then(|index| self.catalog.as_ref()?.entries.get(*index)) {
            cx.emit(EmojiPickerEvent::Choose(entry.unicode.clone()));
        }
    }
}
impl Render for EmojiPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::ui(cx);
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        self.search.update(cx, |field, cx| field.set_placeholder(locale.text("emoji_search"), cx));
        let mut groups = div().id("emoji-groups").flex().flex_wrap().gap(px(3.0));
        for (index, (label, group)) in [("emoji_all", None), ("emoji_0", Some(0)), ("emoji_1", Some(1)), ("emoji_3", Some(3)), ("emoji_4", Some(4)), ("emoji_6", Some(6)), ("emoji_5", Some(5)), ("emoji_7", Some(7)), ("emoji_8", Some(8)), ("emoji_9", Some(9))].into_iter().enumerate() {
            groups = groups.child(div().id(("emoji-group", index)).tab_index(0).px(px(6.0)).h(px(24.0)).flex().flex_none().items_center().rounded(px(4.0))
                .text_size(px(13.0)).text_color(theme.fg).when(self.group == group, |el| el.bg(theme.surface.blend(theme.accent.opacity(0.12))))
                .hover(|style| style.bg(theme.surface.blend(theme.accent.opacity(0.12)))).cursor_pointer().child(locale.text(label))
                .on_click(cx.listener(move |picker, _, _, cx| { picker.group = group; picker.refresh(cx); })));
        }
        let mut body = div().flex().flex_col().gap(px(6.0));
        if self.loading { body = body.child(div().h(px(40.0)).text_size(px(13.0)).text_color(theme.muted).child(locale.text("emoji_loading"))); }
        else if self.error.is_some() {
            body = body.child(div().flex().items_center().gap(px(8.0)).text_size(px(13.0)).child(locale.text("emoji_failed"))
                .child(div().id("emoji-retry").cursor_pointer().text_color(theme.accent_strong).child(locale.text("emoji_retry"))
                    .on_click(cx.listener(|picker, _, _, cx| picker.load(cx)))));
        } else if self.results.is_empty() { body = body.child(div().h(px(40.0)).text_size(px(13.0)).text_color(theme.muted).child(locale.text("emoji_empty"))); }
        else if let Some(catalog) = self.catalog.clone() {
            let results = self.results.clone();
            let selected = self.selected;
            let footer = results.get(selected).and_then(|index| catalog.entries.get(*index)).map(|entry| {
                let label = if locale.is_chinese() { &entry.zh } else { &entry.en };
                (label.clone(), entry.shortcodes.first().cloned().unwrap_or_default())
            });
            let count = results.len().div_ceil(8);
            let list = uniform_list("emoji-grid", count, {
                let theme = theme.clone();
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range.map(|row| {
                        let mut cells = div().h(px(40.0)).flex();
                        for column in 0..8 {
                            let index = row * 8 + column;
                            let Some(entry) = results.get(index).and_then(|item| catalog.entries.get(*item)) else {
                                cells = cells.child(div().flex_1().h(px(40.0)));
                                continue;
                            };
                            let unicode = entry.unicode.clone();
                            let label = if locale.is_chinese() { &entry.zh } else { &entry.en };
                            let tip = format!("{} · :{}:", label, entry.shortcodes.first().map_or("", String::as_str));
                            cells = cells.child(div().id(("emoji-item", index)).flex_1().h(px(40.0)).flex().items_center().justify_center().rounded(px(4.0))
                                .text_size(px(24.0)).when(index == selected, |el| el.bg(theme.surface.blend(theme.accent.opacity(0.12))))
                                .hover(|style| style.bg(theme.surface.blend(theme.accent.opacity(0.12)))).cursor_pointer().tooltip(tooltip(tip)).child(unicode.clone())
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(EmojiPickerEvent::Choose(unicode.clone())))));
                        }
                        cells
                    }).collect()
                })
            }).h(px(240.0)).track_scroll(&self.scroll);
            body = body.child(list);
            if let Some((label, shortcode)) = footer {
                body = body.child(div().mt(px(2.0)).min_h(px(36.0)).flex().flex_col().gap(px(3.0))
                    .text_size(px(14.0)).text_color(theme.fg).child(label)
                    .child(div().text_size(px(13.0)).text_color(theme.muted).child(format!(":{shortcode}:"))));
            }
        }
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        div().id("emoji-picker").track_focus(&self.focus).tab_group()
            .w(px(352.0).min((window.viewport_size().width - px(16.0)).max(px(0.0))))
            .max_h((window.viewport_size().height - px(16.0)).max(px(0.0))).overflow_hidden()
            .p(px(12.0)).flex().flex_col().gap(px(8.0))
            .rounded(px(12.0)).border_1().border_color(theme.border).bg(theme.surface)
            .shadow(vec![gpui::BoxShadow { inset: false, color: theme.fg.opacity(0.12), offset: gpui::point(px(0.0), px(8.0)), blur_radius: px(28.0), spread_radius: px(0.0) }]).occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|picker, event: &gpui::MouseDownEvent, _, cx| {
                if !picker.trigger_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) {
                    cx.emit(EmojiPickerEvent::Dismiss);
                }
            }))
            .capture_key_down(cx.listener(|picker, event: &KeyDownEvent, _, cx| {
                if picker.search.read(cx).is_composing() { return; }
                let len = picker.results.len();
                if len == 0 { return; }
                match event.keystroke.key.as_str() {
                    "down" => picker.selected = (picker.selected + 8).min(len - 1),
                    "up" => picker.selected = picker.selected.saturating_sub(8),
                    _ => return,
                }
                picker.scroll.scroll_to_item(picker.selected / 8, ScrollStrategy::Top);
                cx.notify(); cx.stop_propagation();
            }))
            .child(div().flex().items_center().justify_between().text_size(px(14.0)).font_weight(FontWeight::BOLD).text_color(theme.fg).child(locale.text("emoji"))
                .child(div().id("emoji-close").size(px(26.0)).flex().items_center().justify_center().rounded(px(4.0)).cursor_pointer()
                    .hover(|style| style.bg(theme.surface.blend(theme.accent.opacity(0.12))))
                    .child(icon(IconName::Close, 16.0, theme.fg)).tooltip(tooltip(locale.text("emoji_close")))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(EmojiPickerEvent::Dismiss)))))
            .child(div().h(px(36.0)).px(px(8.0)).flex().items_center().gap(px(7.0)).rounded(px(4.0)).bg(theme.rail_bg)
                .border_1().border_color(if search_focused { theme.accent } else { theme.rail_bg })
                .child(icon(IconName::Search, 15.0, theme.muted)).child(self.search.clone()))
            .child(groups).child(body)
    }
}
