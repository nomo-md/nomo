//! 设置窗口：左侧分区导航，右侧按 `settings_schema` 渲染分组与设置项。
//!
//! 每次修改立即经 `Settings::set` 生效并写回配置文件，没有“应用 / 取消”步骤。
//! 窗口为单例：重复打开时激活已有窗口。文本类设置项各自持有一个 `TextField`，在首次显示时创建。

use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::components::{IconName, badge, button, ButtonKind, icon, select, select_option, slider, switch, text_segment};
use crate::ui::settings_schema::{CATEGORIES, Control, Item, Page, SHORTCUTS, Support};
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

/// 由前缀与设置键组成的元素 ID（gpui 0.2.2 不支持两个字符串的元组 ID）。
fn eid(prefix: &str, key: &str) -> gpui::ElementId {
    gpui::ElementId::Name(format!("{prefix}:{key}").into())
}

/// 已打开的设置窗口句柄，用于单例复用。
#[derive(Default)]
struct SettingsWindowHandle(Option<WindowHandle<SettingsWindow>>);

impl Global for SettingsWindowHandle {}

/// 打开设置窗口；已打开时把它带到前台。
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindowHandle>().and_then(|handle| handle.0) {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(880.0), px(640.0)), cx);
    let options = WindowOptions {
        app_id: Some("com.nomo.desktop".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("设置".into()), appears_transparent: true, traffic_light_position: Some(point(px(14.0), px(14.0))) }),
        window_min_size: Some(size(px(720.0), px(480.0))),
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
    /// 当前展开的下拉选择（设置键）。
    open_select: Option<&'static str>,
    /// 正在拖动的滑杆（设置键）。
    dragging: Option<&'static str>,
    fields: HashMap<&'static str, Entity<TextField>>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsWindow {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            #[cfg(target_os = "windows")]
            caption_focus: window_chrome::CaptionFocus::new(cx),
            category: 0,
            open_select: None,
            dragging: None,
            fields: HashMap::new(),
            scroll: ScrollHandle::new(),
            _subscriptions: Vec::new(),
        }
    }

    /// 文本设置项的输入框：首次使用时创建并填入当前值，输入变化即写入设置。
    fn field(&mut self, item: &'static Item, placeholder: &'static str, cx: &mut Context<Self>) -> Entity<TextField> {
        if let Some(field) = self.fields.get(item.key) {
            return field.clone();
        }
        let current = cx.global::<Settings>().string(item.key);
        let field = cx.new(|cx| {
            let mut field = TextField::new(placeholder, cx);
            field.content = current;
            field
        });
        let key = item.key;
        self._subscriptions.push(cx.subscribe(&field, move |_, field, event, cx| {
            if matches!(event, FieldEvent::Changed) {
                let value = field.read(cx).content.trim().to_string();
                Settings::set(cx, key, json!(value));
            }
        }));
        self.fields.insert(item.key, field.clone());
        field
    }

    fn render_control(&mut self, item: &'static Item, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let key = item.key;
        match &item.control {
            Control::Toggle => {
                let on = settings.bool(key);
                switch(eid("toggle", key), on, theme)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Settings::set(cx, key, json!(!on))))
                    .into_any_element()
            }
            Control::Choice(options) if options.len() <= 3 => {
                let current = settings.string(key);
                div()
                    .flex()
                    .p(px(2.0))
                    .gap(px(2.0))
                    .rounded(px(7.0))
                    .bg(theme.app_bg)
                    .border_1()
                    .border_color(theme.border)
                    .children(options.iter().map(|(value, label)| {
                        let value = *value;
                        text_segment(eid(key, value), *label, current == value, theme)
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| Settings::set(cx, key, json!(value))))
                    }))
                    .into_any_element()
            }
            Control::Choice(options) => {
                let current = settings.string(key);
                let label = options.iter().find(|(value, _)| *value == current).map_or(current.as_str(), |(_, label)| label).to_string();
                let open = self.open_select == Some(key);
                let rows: Vec<AnyElement> = options
                    .iter()
                    .map(|(value, label)| {
                        let value = *value;
                        select_option(eid(key, value), *label, current == value, theme)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_select = None;
                                Settings::set(cx, key, json!(value));
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .collect();
                let entity = cx.entity().downgrade();
                let trigger = select(eid("select", key), label, open, 180.0, viewport, theme, rows, move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.open_select = None;
                        cx.notify();
                    })
                    .ok();
                });
                div()
                    .id(eid("select-wrap", key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_select = Some(key);
                        cx.notify();
                    }))
                    .child(trigger)
                    .into_any_element()
            }
            Control::Range { min, max, step, unit } => {
                let (min, max, step) = (*min, *max, *step);
                let value = settings.number(key).clamp(min, max);
                let fraction = ((value - min) / (max - min)) as f32;
                let entity = cx.entity().downgrade();
                let on_change = Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
                    let raw = min + (max - min) * fraction as f64;
                    let snapped = ((raw - min) / step).round() * step + min;
                    let value: Value = if step < 1.0 { json!((snapped * 100.0).round() / 100.0) } else { json!(snapped.round() as i64) };
                    // 拖动中只改内存，松手时写入配置文件。
                    Settings::set_with(cx, key, value, !dragging);
                    entity.update(cx, |this, cx| {
                        this.dragging = dragging.then_some(key);
                        cx.notify();
                    })
                    .ok();
                });
                let display = if step < 1.0 { format!("{value:.2}{unit}") } else { format!("{}{unit}", value.round() as i64) };
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(slider(eid("slider", key), fraction, self.dragging == Some(key), 180.0, theme, on_change))
                    .child(div().w(px(64.0)).text_size(px(12.5)).text_color(theme.muted).child(display))
                    .into_any_element()
            }
            Control::Text { placeholder } => div().w(px(220.0)).flex().child(self.field(item, placeholder, cx)).into_any_element(),
            Control::Info => div().into_any_element(),
        }
    }

    fn render_item(&mut self, item: &'static Item, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let note = match &item.support {
            Support::Full => None,
            Support::Partial(reason) => Some(("部分支持", *reason, theme.accent)),
            Support::None(reason) => Some(("暂未支持", *reason, theme.muted)),
        };
        let control = self.render_control(item, theme, viewport, cx);
        div()
            .px(px(16.0))
            .py(px(12.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.heading)
                            .child(item.label)
                            .children(note.map(|(label, _, color)| badge(label, color, theme))),
                    )
                    .child(div().text_size(px(12.0)).text_color(theme.muted).child(item.description))
                    .children(note.map(|(_, reason, _)| div().text_size(px(12.0)).text_color(theme.muted).child(format!("当前支持范围：{reason}。")))),
            )
            .child(div().flex_none().child(control))
            .into_any_element()
    }

    fn render_groups(&mut self, groups: &'static [crate::ui::settings_schema::Group], theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        groups
            .iter()
            .filter(|group| group.items.iter().any(|item| !matches!(item.support, Support::None(_))))
            .map(|group| {
                let rows: Vec<AnyElement> = group
                    .items
                    .iter()
                    .filter(|item| !matches!(item.support, Support::None(_)))
                    .enumerate()
                    .flat_map(|(index, item)| {
                        let divider = (index > 0).then(|| div().h(px(1.0)).mx(px(16.0)).bg(theme.border).into_any_element());
                        divider.into_iter().chain(std::iter::once(self.render_item(item, theme, viewport, cx)))
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(theme.muted).child(group.title))
                    .child(div().rounded(px(10.0)).border_1().border_color(theme.border).bg(theme.surface).children(rows))
                    .into_any_element()
            })
            .collect()
    }

    fn render_shortcuts(&self, theme: &Theme) -> Vec<AnyElement> {
        let mut sections: Vec<AnyElement> = vec![div().text_size(px(12.5)).text_color(theme.muted).child("暂不支持修改快捷键；下列为当前绑定（macOS 上 Ctrl 对应 ⌘）。").into_any_element()];
        for (title, rows) in SHORTCUTS {
            let items: Vec<AnyElement> = rows
                .iter()
                .enumerate()
                .flat_map(|(index, (label, keys))| {
                    let divider = (index > 0).then(|| div().h(px(1.0)).mx(px(16.0)).bg(theme.border).into_any_element());
                    let row = div()
                        .px(px(16.0))
                        .py(px(9.0))
                        .flex()
                        .justify_between()
                        .text_size(px(13.0))
                        .child(div().text_color(theme.fg).child(*label))
                        .child(div().px(px(8.0)).rounded(px(5.0)).bg(theme.app_bg).border_1().border_color(theme.border).text_color(theme.muted).child(*keys))
                        .into_any_element();
                    divider.into_iter().chain(std::iter::once(row))
                })
                .collect();
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(theme.muted).child(*title))
                    .child(div().rounded(px(10.0)).border_1().border_color(theme.border).bg(theme.surface).children(items))
                    .into_any_element(),
            );
        }
        sections
    }

    fn render_about(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let config = crate::settings::config_path().map_or_else(|| "未知".to_string(), |path| path.display().to_string());
        let snapshots = crate::settings::snapshot_dir();
        vec![
            div()
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(theme.heading).child("Nomo"))
                .child(div().text_size(px(13.0)).text_color(theme.muted).child(format!("版本 {} · 基于 GPUI {} 的原生 Markdown 编辑器", env!("CARGO_PKG_VERSION"), "0.2.2")))
                .child(div().text_size(px(12.5)).text_color(theme.muted).child(format!("设置文件：{config}")))
                .child(
                    div()
                        .mt(px(8.0))
                        .flex()
                        .gap(px(8.0))
                        .child(button("reveal-config", "打开设置目录", Some(IconName::FolderOpen), ButtonKind::Secondary, theme).on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                            if let Some(path) = crate::settings::config_path() {
                                cx.reveal_path(&path);
                            }
                        })))
                        .child(button("reveal-snapshots", "打开快照目录", Some(IconName::History), ButtonKind::Secondary, theme).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            if let Some(dir) = &snapshots {
                                let _ = std::fs::create_dir_all(dir);
                                cx.reveal_path(dir);
                            }
                        }))),
                )
                .into_any_element(),
        ]
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let viewport = window.viewport_size();
        let category = &CATEGORIES[self.category];
        let nav: Vec<AnyElement> = CATEGORIES
            .iter()
            .enumerate()
            .filter(|(_, entry)| match &entry.page {
                Page::Groups(groups) => groups.iter().any(|group| group.items.iter().any(|item| !matches!(item.support, Support::None(_)))),
                _ => true,
            })
            .map(|(index, entry)| {
                let active = index == self.category;
                div()
                    .id(("category", index))
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(7.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .text_color(if active { theme.accent_strong } else { theme.fg })
                    .when(active, |el| el.bg(theme.sidebar_active).font_weight(FontWeight::SEMIBOLD))
                    .when(!active, |el| el.hover(|style| style.bg(theme.hover)))
                    .child(icon(entry.icon, 15.0, if active { theme.accent } else { theme.muted }))
                    .child(entry.title)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.category = index;
                        this.open_select = None;
                        this.scroll.set_offset(point(px(0.0), px(0.0)));
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let content: Vec<AnyElement> = match &category.page {
            Page::Groups(groups) => self.render_groups(groups, &theme, viewport, cx),
            Page::Shortcuts => self.render_shortcuts(&theme),
            Page::About => self.render_about(&theme, cx),
        };
        let mut nav_drag = div().id("settings-nav-drag-area").relative().h(px(46.0)).flex_none();
        #[cfg(target_os = "windows")]
        { nav_drag = nav_drag.child(window_chrome::drag_surface("settings-nav-drag")); }
        #[cfg(not(target_os = "windows"))]
        { nav_drag = nav_drag.on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move()); }

        #[cfg(target_os = "windows")]
        let header = div().id("settings-header").relative().h(px(42.0)).flex_none().flex().items_center()
            .pl(px(16.0)).border_b_1().border_color(theme.border)
            .child(window_chrome::drag_surface("settings-header-drag"))
            .child(div().flex_1().min_w(px(0.0)).text_size(px(16.0)).font_weight(FontWeight::BOLD)
                .text_color(theme.heading).child(category.title))
            .child(window_chrome::caption_controls(&theme, window, &self.caption_focus, |action, window, _| match action {
                CaptionAction::Minimize => window.minimize_window(),
                CaptionAction::Maximize => window_chrome::toggle_maximize(window),
                CaptionAction::Close => window.remove_window(),
            }));
        #[cfg(not(target_os = "windows"))]
        let header = div().h(px(56.0)).flex_none().flex().items_end().px(px(28.0)).pb(px(10.0))
            .text_size(px(18.0)).font_weight(FontWeight::BOLD).text_color(theme.heading)
            .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.start_window_move())
            .child(category.title);

        let mut root = div()
            .id("settings")
            .relative().children(animate_theme.then(crate::ui::motion::theme_frame))
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(theme.surface)
            .text_color(theme.fg)
            .font_family(crate::theme::ui_font_family())
            .child(
                div()
                    .w(px(200.0))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .px(px(10.0))
                    .bg(theme.rail_bg)
                    .border_r_1()
                    .border_color(theme.border)
                    .child(nav_drag)
                    .children(nav),
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
                            .px(px(28.0))
                            .pb(px(28.0))
                            .child(div().flex().flex_col().gap(px(22.0)).pt(px(6.0)).children(content)),
                    ),
            );
        #[cfg(target_os = "windows")]
        if !window.is_maximized() && !window.is_fullscreen() {
            root = root.child(window_chrome::top_resize_guard(window));
        }
        root
    }
}
