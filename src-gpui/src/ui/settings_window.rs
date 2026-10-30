//! 设置窗口：左侧分区导航，右侧保留原版各分区的结构与控件。
//!
//! 已接通的修改立即生效；未接通项显示为禁用状态，不创建输入实体或写入配置。
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::components::{IconName, button, ButtonKind, icon, select, select_option, settings_slider, switch, text_segment, tooltip};
use crate::ui::motion::ScalarMotion;
use crate::ui::settings_schema::{CATEGORIES, Control, Item, Page, Support};
use crate::ui::text_field::{FieldEvent, TextField};
#[cfg(target_os = "windows")]
use crate::ui::window_chrome::{self, CaptionAction};
use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, Global,
    IntoElement, ParentElement, Render, ScrollHandle, Styled, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, point, prelude::*, px, size,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

#[path = "settings_pages.rs"]
mod pages;
#[path = "settings_shortcuts.rs"]
mod shortcut_fields;
use pages::{display_value, item_is_visible, readonly_input, section_heading, unavailable_action};

const COMPACT_WIDTH: f32 = 760.0;

fn content_width(viewport: gpui::Size<gpui::Pixels>) -> f32 {
    let compact = f32::from(viewport.width) <= COMPACT_WIDTH;
    let (sidebar, padding) = if compact { (64.0, 16.0) } else { (212.0, 24.0) };
    (f32::from(viewport.width) - sidebar - padding * 2.0).clamp(0.0, 740.0)
}

fn control_width(viewport: gpui::Size<gpui::Pixels>) -> f32 {
    let width = content_width(viewport);
    if f32::from(viewport.width) <= COMPACT_WIDTH { width } else { (width - 220.0 - 24.0).clamp(0.0, 300.0) }
}

fn eid(prefix: &str, key: &str) -> gpui::ElementId {
    gpui::ElementId::Name(format!("{prefix}:{key}").into())
}

#[derive(Default)]
struct SettingsWindowHandle(Option<WindowHandle<SettingsWindow>>);
impl Global for SettingsWindowHandle {}

pub(crate) fn close_if_orphaned(cx: &mut App) {
    let Some(handle) = cx.try_global::<SettingsWindowHandle>().and_then(|state| state.0) else { return; };
    let windows = cx.windows();
    if !windows.contains(&handle.into()) {
        cx.global_mut::<SettingsWindowHandle>().0 = None;
        return;
    }
    // 设置由所有主窗口共享；正式工作区和开发窗口全部关闭后才释放设置。
    if windows.len() > 1 { return; }
    match handle.update(cx, |_, window, _| window.remove_window()) {
        Ok(()) => cx.global_mut::<SettingsWindowHandle>().0 = None,
        Err(error) => eprintln!("关闭设置窗口失败：{error}"),
    }
}

pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindowHandle>().and_then(|handle| handle.0) {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() { return; }
    }
    let bounds = Bounds::centered(None, size(px(860.0), px(620.0)), cx);
    let options = WindowOptions {
        app_id: Some("com.nomo.desktop".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("偏好设置 - Nomo".into()), appears_transparent: true, traffic_light_position: Some(point(px(14.0), px(14.0))) }),
        window_min_size: Some(size(px(760.0), px(520.0))),
        ..Default::default()
    };
    if let Ok(handle) = cx.open_window(options, |_, cx| cx.new(SettingsWindow::new)) {
        cx.set_global(SettingsWindowHandle(Some(handle)));
    }
}

pub struct SettingsWindow {
    focus: FocusHandle,
    #[cfg(target_os = "windows")]
    caption_focus: window_chrome::CaptionFocus,
    category: usize,
    open_select: Option<&'static str>,
    dragging: Option<&'static str>,
    /// 上限为当前分区开关数，切换分区即清空；窗口关闭时一并释放。
    toggles: HashMap<&'static str, ScalarMotion>,
    fields: HashMap<&'static str, Entity<TextField>>,
    /// 仅当前文本设置的校验结果，不缓存系统字体列表。
    shortcut_errors: HashMap<&'static str, String>,
    font_errors: HashMap<&'static str, &'static str>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    image_test: Option<crate::image_import::ImportCancellation>,
    image_test_message: Option<String>,
}
impl Drop for SettingsWindow {
    fn drop(&mut self) { if let Some(cancel) = &self.image_test { cancel.cancel(); } }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl SettingsWindow {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            #[cfg(target_os = "windows")]
            caption_focus: window_chrome::CaptionFocus::new(cx),
            category: 0, open_select: None, dragging: None,
            toggles: HashMap::new(), fields: HashMap::new(), font_errors: HashMap::new(), shortcut_errors: HashMap::new(),
            scroll: ScrollHandle::new(), _subscriptions: Vec::new(),
            image_test: None, image_test_message: None,
        }
    }

    fn field(&mut self, item: &'static Item, placeholder: &'static str, cx: &mut Context<Self>) -> Entity<TextField> {
        if let Some(field) = self.fields.get(item.key) { return field.clone(); }
        let current = cx.global::<Settings>().string(item.key);
        let font = matches!(item.control, Control::FontFamily);
        let field = cx.new(|cx| {
            let mut field = TextField::new(placeholder, cx);
            field.set_settings_style();
            if font { field.reserve_clear_button(); }
            if matches!(item.key, "monoFontFamily" | "quickInsertTrigger") { field.set_settings_monospace(); }
            field.content = current;
            field
        });
        let key = item.key;
        self._subscriptions.push(cx.subscribe(&field, move |this, field, event, cx| {
            if matches!(event, FieldEvent::Changed) {
                let value = field.read(cx).content.trim().to_string();
                if field.read(cx).is_composing() {return;}
                if key=="quickInsertTrigger" || key=="imageHandlingSettings.defaultImageWidth" {
                    let invalid=if key=="quickInsertTrigger" {!crate::nomo_mark::quick_insert::valid_trigger(&value)}else{crate::image_import::normalize_width(&value).is_none()};
                    field.update(cx,|field,cx|field.set_invalid(invalid,cx));
                    if invalid {this.font_errors.insert(key,if key=="quickInsertTrigger" {"请输入一个标点或符号。"}else{"请输入像素宽度或百分比，例如 480px、50%。"});}
                    else {this.font_errors.remove(key);let value=if key=="imageHandlingSettings.defaultImageWidth" {crate::image_import::normalize_width(&value).unwrap()}else{value};Settings::set(cx,key,json!(value));}
                    cx.notify();return;
                }
                let invalid = font && value.chars().any(|ch| matches!(ch, '"' | '\'' | ',' | '(' | ')' | ';' | '!'));
                field.update(cx, |field, cx| field.set_invalid(invalid, cx));
                if invalid {
                    this.font_errors.insert(key, "请输入单个字体名称，不能包含引号、逗号、括号、分号或感叹号。");
                } else {
                    this.font_errors.remove(key);
                    Settings::set(cx, key, json!(value));
                }
                cx.notify();
            }
        }));
        self.fields.insert(item.key, field.clone());
        field
    }

    fn render_control(&mut self, item: &'static Item, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let key = item.key;
        let width = control_width(viewport);
        let enabled = !matches!(item.support, Support::None(_)) && (key != "quickInsertTrigger" || settings.bool("quickInsertEnabled"));
        let value = display_value(&settings, key);
        match &item.control {
            Control::Toggle => {
                let on = value.as_bool().unwrap_or(false);
                let (progress, running) = if enabled {
                    let reduced = crate::ui::popup::reduced_motion(cx);
                    let motion = self.toggles.entry(key).or_insert_with(|| ScalarMotion::settled(on));
                    motion.set(on, reduced, Duration::from_millis(160));
                    (motion.value(), motion.running())
                } else { (if on { 1.0 } else { 0.0 }, false) };
                switch(eid("toggle", key), progress, theme)
                    .children(running.then(crate::ui::motion::theme_frame))
                    .when(enabled, |el| el.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Settings::set(cx, key, json!(!on)))))
                    .when(!enabled, |el| el.opacity(0.55).cursor(gpui::CursorStyle::OperationNotAllowed))
                    .into_any_element()
            }
            Control::Choice(options) if options.len() <= 4 => {
                let current = value.as_str().unwrap_or_default();
                let buttons = |options: &[(&'static str, &'static str)]| -> Vec<AnyElement> {
                    options.iter().map(|(value, label)| {
                        let value = *value;
                        let active = current == value;
                        let option_enabled = enabled && !(key == "imageHandlingSettings.imageInsertStrategy" && value == "upload") && !(key=="closeWindowBehavior" && value=="close-to-tray" && !cfg!(windows));
                        text_segment(eid(key, value), *label, active, option_enabled, theme)
                            .when(option_enabled, |el| el.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Settings::set(cx, key, json!(value)))))
                            .when(enabled && !option_enabled, |el| el.opacity(0.55).tooltip(tooltip("图片上传尚未接入。")))
                            .into_any_element()
                    }).collect()
                };
                let group = div().w(px(width)).flex().p(px(4.0)).gap(px(4.0)).rounded(px(8.0))
                    .bg(crate::ui::motion::color(theme.app_bg, theme.surface, 0.82))
                    .border_1().border_color(theme.border).when(!enabled, |el| el.opacity(0.55));
                if options.len() == 4 {
                    group.flex_col().children(options.chunks(2).map(|pair| div().flex().gap(px(4.0)).children(buttons(pair)))).into_any_element()
                } else { group.children(buttons(options)).into_any_element() }
            }
            Control::Choice(options) => {
                let current = value.as_str().unwrap_or_default();
                let label = options.iter().find(|(value, _)| *value == current).map_or(current, |(_, label)| label).to_string();
                if !enabled {
                    return readonly_input(label, width, theme).justify_between()
                        .child(icon(IconName::ChevronDown, 14.0, theme.muted)).into_any_element();
                }
                let open = self.open_select == Some(key);
                let rows = options.iter().map(|(value, label)| {
                    let value = *value;
                    select_option(eid(key, value), *label, current == value, theme)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_select = None;
                            Settings::set(cx, key, json!(value));
                            cx.notify();
                        })).into_any_element()
                }).collect();
                let entity = cx.entity().downgrade();
                let trigger = select(eid("select", key), label, open, width, viewport, crate::ui::popup::reduced_motion(cx), theme, rows, move |_, cx| {
                    entity.update(cx, |this, cx| { this.open_select = None; cx.notify(); }).ok();
                });
                div().id(eid("select-wrap", key)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_select = Some(key); cx.notify();
                })).child(trigger).into_any_element()
            }
            Control::Range { min, max, step, unit } => {
                let (min, max, step) = (*min, *max, *step);
                let value = value.as_f64().unwrap_or(min).clamp(min, max);
                let fraction = ((value - min) / (max - min)) as f32;
                let display = if key == "outlineDefaultExpandLevel" { format!("H{}", value.round() as i64) }
                    else if step < 1.0 { format!("{value:.2}{unit}") } else { format!("{}{unit}", value.round() as i64) };
                if !enabled { return readonly_input(display, width, theme).into_any_element(); }
                let entity = cx.entity().downgrade();
                let on_change = Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
                    let raw = min + (max - min) * fraction as f64;
                    let snapped = ((raw - min) / step).round() * step + min;
                    let value: Value = if step < 1.0 { json!((snapped * 100.0).round() / 100.0) } else { json!(snapped.round() as i64) };
                    Settings::set_with(cx, key, value, !dragging);
                    entity.update(cx, |this, cx| { this.dragging = dragging.then_some(key); cx.notify(); }).ok();
                });
                div().flex().items_center().gap(px(12.0))
                    .child(settings_slider(eid("slider", key), fraction, self.dragging == Some(key), (width - 86.0).max(16.0), theme, on_change))
                    .child(div().w(px(74.0)).text_right().text_size(px(13.0))
                        .font_family(crate::theme::font_for(crate::theme::Family::Mono, false, false).family)
                        .text_color(theme.muted).child(display)).into_any_element()
            }
            Control::Text { placeholder, compact } => {
                let width = if key == "quickInsertTrigger" { width.min(64.0) } else if *compact { width.min(180.0) } else { width };
                if enabled { div().w(px(width)).flex().child(self.field(item, placeholder, cx)).into_any_element() }
                else {
                    let text = value.as_str().filter(|value| !value.is_empty()).unwrap_or(placeholder);
                    readonly_input(text.to_string(), width, theme).into_any_element()
                }
            }
            Control::FontFamily => self.render_font_control(item, theme, width.min(220.0), cx),
            Control::Number { unit } => div().flex().items_center().gap(px(10.0))
                .child(readonly_input(value.as_i64().unwrap_or_default().to_string(), width.min(180.0), theme))
                .child(div().text_size(px(13.0)).font_family(crate::theme::font_for(crate::theme::Family::Mono, false, false).family)
                    .text_color(theme.muted).child(*unit)).into_any_element(),
            Control::Action(label) if key=="updateCheck" => {
                let busy=super::updates::busy(cx);
                button("update-check",if busy {"正在检查…"}else{label},None,ButtonKind::Secondary,theme)
                    .when(!busy,|el|el.on_click(cx.listener(|_,_,window,cx|super::updates::check(window,cx,false))))
                    .when(busy,|el|el.opacity(0.55)).into_any_element()
            },
            Control::Action(label) if key == "imageConnectionTest" => {
                let busy = self.image_test.is_some();
                div().flex().flex_col().gap(px(6.0)).child(
                    button("image-connection-test", if busy { "正在检查…" } else { label }, None,
                        if busy { ButtonKind::Disabled } else { ButtonKind::Secondary }, theme)
                        .when(!busy, |el| el.on_click(cx.listener(|view, _, _, cx| {
                            let value = cx.global::<Settings>().value("imageHandlingSettings");
                            let settings = crate::image_import::ImageSettings::read(&value);
                            let cancel = crate::image_import::ImportCancellation::default();
                            view.image_test = Some(cancel.clone()); view.image_test_message = None;
                            let task = cx.background_executor().spawn(async move { crate::image_import::test_connection(settings, cancel) });
                            cx.spawn(async move |this, cx| {
                                let result = task.await;
                                let _ = this.update(cx, |view, cx| {
                                    view.image_test = None;
                                    if cx.global::<Settings>().value("imageHandlingSettings") == value {
                                        view.image_test_message = Some(result.map_or_else(|error| error, |_| "连接成功".into()));
                                    }
                                    cx.notify();
                                });
                            }).detach(); cx.notify();
                        }))))
                    .children(self.image_test_message.clone().map(|message| div().text_size(px(12.0)).text_color(theme.muted).child(message)))
                    .into_any_element()
            },
            Control::Action(label) => unavailable_action(key, label, theme).into_any_element(),
            Control::Info => div().px(px(9.0)).py(px(5.0)).rounded_full().border_1().border_color(theme.border)
                .text_size(px(13.0)).text_color(theme.muted).child("后续版本支持").into_any_element(),
            Control::ThemePicker | Control::Shortcuts => div().into_any_element(),
        }
    }

    fn render_item(&mut self, item: &'static Item, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        if matches!(item.control, Control::ThemePicker) { return self.render_theme_picker(theme, viewport, cx); }
        if matches!(item.control, Control::Shortcuts) { return self.render_shortcuts(theme, viewport, cx); }
        let compact = f32::from(viewport.width) <= COMPACT_WIDTH;
        let note = match &item.support {
            Support::Full => None,
            Support::Partial(reason) | Support::None(reason) => Some(*reason),
        };
        let control = self.render_control(item, theme, viewport, cx);
        let error = self.font_errors.get(item.key).copied();
        let (label, description) = if item.key == "ctrlWheelZoomEnabled" && cfg!(target_os = "macos") {
            ("⌘ 滚轮缩放", "按住 ⌘ 并滚动鼠标滚轮调整缩放。")
        } else { (item.label, item.description) };
        div().flex_none().min_h(px(64.0)).py(px(12.0)).border_t_1().border_color(theme.border.opacity(0.72))
            .flex().justify_between()
            .when(compact, |el| el.flex_col().items_start().gap(px(10.0)))
            .when(!compact, |el| el.items_center().gap(px(24.0)))
            .child(div().when(!compact, |el| el.flex_1()).when(compact, |el| el.w_full())
                .min_w(px(0.0)).flex().flex_col().gap(px(4.0))
                .child(div().id(eid("setting-label", item.key)).text_size(px(14.0))
                    .line_height(gpui::relative(1.35)).font_weight(FontWeight(650.0)).text_color(theme.fg).child(label)
                    .when_some(note, |el, reason| el.tooltip(tooltip(format!("当前支持范围：{reason}。")))))
                .when(!description.is_empty(), |el| el.child(div().text_size(px(13.0))
                    .line_height(gpui::relative(1.45)).text_color(theme.muted).child(description)))
                .children(error.map(|error| div().text_size(px(13.0)).line_height(gpui::relative(1.45))
                    .text_color(gpui::rgb(0xd14343)).child(error))))
            .child(div().w(px(control_width(viewport))).flex_none().flex().justify_end().child(control)).into_any_element()
    }

    fn render_groups(&mut self, groups: &'static [crate::ui::settings_schema::Group], theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let settings = cx.global::<Settings>().clone();
        groups.iter().map(|group| {
            let rows: Vec<AnyElement> = group.items.iter().filter(|item| item_is_visible(item, &settings))
                .map(|item| self.render_item(item, theme, viewport, cx)).collect();
            div().flex_none().flex().flex_col().w_full().max_w(px(740.0))
                .child(section_heading(group.title, group.badge, theme)).children(rows).into_any_element()
        }).collect()
    }
}


impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::updates::defer_pending(window,cx);
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let viewport = window.viewport_size();
        let compact = f32::from(viewport.width) <= COMPACT_WIDTH;
        let ui_font = cx.global::<Settings>().string("uiFontFamily");
        let ui_font = if ui_font.trim().is_empty() { crate::theme::ui_font_family().to_string() } else { ui_font };
        let category = &CATEGORIES[self.category];
        let category_title = if matches!(category.page, Page::About) { "关于 Nomo" } else { category.title };
        let nav: Vec<AnyElement> = CATEGORIES
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let active = index == self.category;
                div()
                    .id(("category", index))
                    .h(px(38.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(4.0))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .text_color(if active { theme.accent_strong } else { theme.muted })
                    .when(active, |el| el.bg(theme.sidebar_active))
                    .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
                    .when(compact, |el| el.justify_center().tooltip(tooltip(entry.title)))
                    .child(icon(entry.icon, 16.0, if active { theme.accent_strong } else { theme.muted }))
                    .when(!compact, |el| el.child(entry.title))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.category = index;
                        this.open_select = None;
                        this.toggles.clear();
                        this.scroll.set_offset(point(px(0.0), px(0.0)));
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let content: Vec<AnyElement> = match &category.page {
            Page::Groups(groups) => self.render_groups(groups, &theme, viewport, cx),
            Page::About => self.render_about(&theme, viewport, cx),
        };
        let mut nav_drag = div().id("settings-nav-drag-area").relative().h(px(42.0)).flex_none()
            .flex().items_center().gap(px(10.0)).px(px(if compact { 0.0 } else { 14.0 }))
            .when(compact, |el| el.justify_center())
            .border_b_1().border_color(theme.border)
            .child(gpui::img("app/128x128.png").size(px(22.0)).flex_none())
            .when(!compact, |el| el.child(div().text_size(px(14.0)).font_weight(FontWeight::BOLD).child("偏好设置")));
        #[cfg(target_os = "windows")]
        { nav_drag = nav_drag.child(window_chrome::drag_surface("settings-nav-drag")); }
        #[cfg(not(target_os = "windows"))]
        { nav_drag = nav_drag.on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move()); }

        #[cfg(target_os = "windows")]
        let header = div().id("settings-header").relative().h(px(42.0)).flex_none().flex().items_center()
            .pl(px(16.0)).border_b_1().border_color(theme.border)
            .child(window_chrome::drag_surface("settings-header-drag"))
            .child(div().flex_1().min_w(px(0.0)).text_size(px(16.0)).font_weight(FontWeight::BOLD)
                .text_color(theme.heading).child(category_title))
            .child(window_chrome::caption_controls(&theme, window, &self.caption_focus, |action, window, _| match action {
                CaptionAction::Minimize => window.minimize_window(),
                CaptionAction::Maximize => window_chrome::toggle_maximize(window),
                CaptionAction::Close => window.remove_window(),
            }));
        #[cfg(not(target_os = "windows"))]
        let header = div().h(px(56.0)).flex_none().flex().items_end().px(px(24.0)).pb(px(10.0))
            .text_size(px(18.0)).font_weight(FontWeight::BOLD).text_color(theme.heading)
            .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move())
            .child(category_title);

        let mut root = div()
            .id("settings")
            .relative().children(animate_theme.then(crate::ui::motion::theme_frame))
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(theme.app_bg)
            .text_color(theme.fg)
            .font_family(ui_font)
            .child(
                div()
                    .w(px(if compact { 64.0 } else { 212.0 }))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(crate::ui::motion::color(theme.app_bg, theme.rail_bg, 0.78))
                    .border_r_1()
                    .border_color(theme.border)
                    .child(nav_drag)
                    .child(div().id("settings-nav-list").flex_1().min_h(px(0.0)).overflow_y_scroll()
                        .flex().flex_col().gap(px(4.0)).py(px(12.0)).px(px(10.0)).children(nav)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .p(px(if compact { 16.0 } else { 24.0 }))
                            .child(div().w_full().max_w(px(740.0)).flex().flex_col().gap(px(28.0)).children(content)),
                    ),
            );
        #[cfg(target_os = "windows")]
        if !window.is_maximized() && !window.is_fullscreen() {
            root = root.child(window_chrome::top_resize_guard(window));
        }
        root
    }
}
