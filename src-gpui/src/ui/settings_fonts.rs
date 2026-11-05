//! 字体名称来自当前渲染后端；搜索是草稿，只有选中候选才会写入设置。
use super::*;
use crate::ui::toolbar_i18n::ToolbarLocale;
use gpui::{EventEmitter, ScrollStrategy, UniformListScrollHandle, uniform_list};

const MAX_NAMES: usize = 4096;
const MAX_BYTES: usize = 512 * 1024;

struct FamilyList {
    names: Vec<String>,
    // 超预算时只保留当前查询的候选；搜索其他字族时重新查询系统，不永久截断目录。
    filtered_for: Option<String>,
    more: bool,
}

fn families(names: Vec<String>, query: &str) -> FamilyList {
    let cost = |name: &String| name.capacity() + std::mem::size_of::<String>() + std::mem::size_of::<usize>();
    let limited = names.len() > MAX_NAMES || names.iter().map(cost).sum::<usize>() > MAX_BYTES;
    let mut list = FamilyList { names: Vec::new(), filtered_for: limited.then(|| query.into()), more: false };
    let mut bytes = 0;
    for name in names {
        if name.is_empty() || (limited && !name.to_lowercase().contains(query)) { continue; }
        let next = bytes + cost(&name);
        if list.names.len() == MAX_NAMES || next > MAX_BYTES { list.more = true; continue; }
        bytes = next; list.names.push(name);
    }
    list
}

pub(super) struct Dismiss;
pub(super) struct FontPicker {
    key: &'static str,
    search: Entity<TextField>,
    list: Option<FamilyList>,
    matches: Vec<usize>,
    reset: bool,
    selected: usize,
    scroll: UniformListScrollHandle,
    load: Option<gpui::Task<()>>,
    _input: Subscription,
}
impl EventEmitter<Dismiss> for FontPicker {}
impl Focusable for FontPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.search.focus_handle(cx) }
}
impl FontPicker {
    fn new(key: &'static str, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            let mut input = TextField::new(ToolbarLocale::current(cx).app_text("font.search"), cx);
            input.set_settings_style();
            input
        });
        let input = cx.subscribe(&search, |picker, _, event, cx| match event {
            FieldEvent::Changed if !picker.composing(cx) => picker.filter(cx),
            FieldEvent::Submit { .. } if !picker.composing(cx) => picker.choose(picker.selected, cx),
            FieldEvent::Cancel if !picker.composing(cx) => cx.emit(Dismiss),
            _ => {}
        });
        let mut picker = Self { key, search, list: None, matches: Vec::new(), reset: true, selected: 0,
            scroll: UniformListScrollHandle::new(), load: None, _input: input };
        picker.filter(cx);
        picker
    }

    pub(super) fn composing(&self, cx: &App) -> bool { self.search.read(cx).is_composing() }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).content.trim();
        self.matches.clear(); self.selected = 0; self.reset = query.is_empty() && self.list.is_some();
        if query.len() > 1024 { cx.notify(); return; }
        let query = query.to_lowercase();
        if let Some(list) = &self.list
            && list.filtered_for.as_ref().is_none_or(|old| old == &query) {
            self.matches.extend(list.names.iter().enumerate().filter_map(|(i, name)|
                (query.is_empty() || list.filtered_for.is_some() || name.to_lowercase().contains(&query)).then_some(i)));
            if query.is_empty() {
                let current = cx.global::<Settings>().string(self.key);
                if let Some(index) = self.matches.iter().position(|i| list.names[*i].eq_ignore_ascii_case(&current)) { self.selected = index + 1; }
            }
        } else if self.load.is_none() {
            let system = cx.text_system().clone();
            let load = cx.background_executor().spawn(async move { families(system.all_font_names(), &query) });
            self.load = Some(cx.spawn(async move |this, cx| {
                let list = load.await;
                let _ = this.update(cx, |picker, cx| { picker.load = None; picker.list = Some(list); picker.filter(cx); });
            }));
        }
        self.scroll.scroll_to_item(self.selected, ScrollStrategy::Top);
        cx.notify();
    }

    fn value(&self, index: usize) -> Option<&str> {
        if self.reset && index == 0 { return Some(""); }
        let i = self.matches.get(index.checked_sub(usize::from(self.reset))?)?;
        Some(&self.list.as_ref()?.names[*i])
    }
    fn choose(&self, index: usize, cx: &mut Context<Self>) {
        if !self.composing(cx) && let Some(value) = self.value(index) {
            Settings::set(cx, self.key, json!(value));
            cx.emit(Dismiss);
        }
    }
}
impl Render for FontPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::ui(cx);
        let locale = ToolbarLocale::current(cx);
        self.search.update(cx, |field, cx| field.set_placeholder(locale.app_text("font.search"), cx));
        let count = self.matches.len() + usize::from(self.reset);
        let current = cx.global::<Settings>().string(self.key);
        let missing = !current.is_empty() && self.list.as_ref().is_some_and(|list| list.filtered_for.is_none() && !list.names.iter().any(|name| name.eq_ignore_ascii_case(&current)));
        let mut panel = div().id("font-picker").flex().flex_col().gap(px(4.0))
            .on_key_down(cx.listener(move |picker, event: &gpui::KeyDownEvent, window, cx| {
                if picker.composing(cx) || count == 0 { return; }
                picker.selected = match event.keystroke.key.as_str() {
                    "up" => picker.selected.saturating_sub(1), "down" => (picker.selected + 1).min(count - 1), _ => return,
                };
                picker.scroll.scroll_to_item(picker.selected, ScrollStrategy::Nearest);
                window.prevent_default(); cx.stop_propagation(); cx.notify();
            })).child(self.search.clone());
        if missing { panel = panel.child(div().px(px(5.0)).text_size(px(12.0)).text_color(theme.muted).child(locale.app_text("font.missing"))); }
        if count > 0 {
            let list = uniform_list("installed-fonts", count, cx.processor(move |picker, range: std::ops::Range<usize>, _, cx| {
                let theme = Theme::ui(cx);
                range.map(|i| {
                    let name = picker.value(i).unwrap_or_default();
                    select_option(("font-option", i), if name.is_empty() { ToolbarLocale::current(cx).app_text("font.theme").into() } else { name.to_owned() }, name.eq_ignore_ascii_case(&current), &theme)
                        .whitespace_nowrap().overflow_hidden()
                        .when(i == picker.selected, |row| row.bg(theme.hover))
                        .on_click(cx.listener(move |picker, _, _, cx| { cx.stop_propagation(); picker.choose(i, cx); }))
                }).collect()
            })).track_scroll(&self.scroll).h(px(count.min(7) as f32 * 28.0));
            panel = panel.child(list);
        }
        let hint = if self.load.is_some() { Some(locale.app_text("font.loading")) }
            else if count == 0 { Some(locale.app_text("font.empty")) }
            else if self.list.as_ref().is_some_and(|list| list.more) { Some(locale.app_text("font.more")) } else { None };
        panel.children(hint.map(|text| div().p(px(5.0)).text_size(px(12.0)).text_color(theme.muted).child(text)))
    }
}

impl SettingsWindow {
    pub(super) fn render_font_control(&mut self, item: &'static Item, theme: &Theme, width: f32, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let key = item.key;
        let current = cx.global::<Settings>().string(key);
        let picker = self.font_picker.as_ref().filter(|(picker, _)| picker.read(cx).key == key);
        let owner = cx.entity().downgrade();
        let select = select(eid("font-select", key), if current.is_empty() { ToolbarLocale::current(cx).app_text("font.theme").into() } else { current }, picker.is_some(), width, viewport,
            crate::ui::popup::reduced_motion(cx), theme, picker.map(|(picker, _)| picker.clone().into_any_element()).into_iter().collect(),
            move |window, cx| { let _ = owner.update(cx, |view, cx| { view.font_picker = None; window.focus(&view.focus, cx); cx.notify(); }); });
        div().id(eid("font-trigger", key)).tab_index(0).child(select)
            .on_click(cx.listener(move |view, _, window, cx| view.open_fonts(key, window, cx)))
            .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, window, cx| {
                if view.font_picker.is_none() && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    view.open_fonts(key, window, cx); window.prevent_default(); cx.stop_propagation();
                }
            })).into_any_element()
    }
    fn open_fonts(&mut self, key: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.font_picker.as_ref().is_some_and(|(picker, _)| picker.read(cx).key == key) { return; }
        self.open_select = None;
        let picker = cx.new(|cx| FontPicker::new(key, cx));
        let sub = cx.subscribe_in(&picker, window, |view, _, _: &Dismiss, window, cx| {
            view.font_picker = None; window.focus(&view.focus, cx); cx.notify();
        });
        window.focus(&picker.focus_handle(cx), cx);
        self.font_picker = Some((picker, sub));
        cx.notify();
    }
}
