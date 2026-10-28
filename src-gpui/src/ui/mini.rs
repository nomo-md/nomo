//! Markdown Mini 的轻量悬浮标题栏；正文和原生窗口状态仍由 Workspace 持有。
use super::{components::{icon, tooltip, IconName}, motion::{MotionPreference, ScalarMotion}};
use crate::theme::Theme;
use gpui::{prelude::*, div, point, px, App, BoxShadow, Context, EventEmitter, FocusHandle,
    Focusable, IntoElement, MouseButton, Render, SharedString, Subscription, Window};
use std::time::Duration;

mod native;
pub(crate) use native::{MiniGeometry, MiniWindow};

#[derive(Clone, Copy)]
pub(crate) enum MiniEvent { Pin, Return }

#[derive(Clone, PartialEq)]
pub(crate) struct MiniTitle {
    pub name: SharedString,
    pub dirty: bool,
    pub external: bool,
    pub pinned: bool,
}

pub(crate) struct MiniBar {
    title: MiniTitle,
    focus: FocusHandle,
    buttons: [FocusHandle; 2],
    hovered: bool,
    reveal: ScalarMotion,
    _focus: [Subscription; 2],
}

impl EventEmitter<MiniEvent> for MiniBar {}
impl Focusable for MiniBar {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl MiniBar {
    pub(crate) fn new(title: MiniTitle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        let subscriptions = [
            cx.on_focus_in(&focus, window, |_, _, cx| cx.notify()),
            cx.on_focus_out(&focus, window, |_, _, _, cx| cx.notify()),
        ];
        Self { title, focus, buttons: std::array::from_fn(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
            hovered: false, reveal: ScalarMotion::default(), _focus: subscriptions }
    }

    pub(crate) fn set_title(&mut self, title: MiniTitle, cx: &mut Context<Self>) {
        if self.title != title { self.title = title; cx.notify(); }
    }
}

impl Render for MiniBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::ui(cx);
        let reduced = cx.global::<MotionPreference>().reduced;
        self.reveal.set(self.hovered || self.focus.contains_focused(window, cx), reduced, Duration::from_millis(140));
        let reveal = self.reveal.value();
        // 只重绘这个小视图；动画结束后不再请求帧，也不通知文档宿主。
        if self.reveal.running() || Theme::ui_transition_active(cx) { window.request_animation_frame(); }
        let pin = if self.title.pinned { IconName::Pin } else { IconName::PinOff };
        let pin_label = if self.title.pinned { "取消置顶" } else { "置顶小窗" };
        let button = |id, index: usize, glyph, label, action| {
            div().id(id).relative().flex_none().w(px(if index == 0 { 26.0 } else { 34.0 })).h(px(26.0))
                .rounded(px(5.0)).flex().items_center().justify_center().cursor_pointer().occlude()
                .track_focus(&self.buttons[index]).tab_index(0)
                .when(index == 0 && self.title.pinned, |el| el.bg(theme.hover))
                .when(self.buttons[index].is_focused(window), |el| el.border_1().border_color(theme.accent))
                .hover(|style| style.bg(theme.hover)).active(|style| style.bg(theme.border))
                .tooltip(tooltip(label)).child(icon(glyph, if index == 0 { 14.0 } else { 15.0 }, if index == 0 { theme.muted } else { theme.accent }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); cx.emit(action); }))
        };
        let identity = div().relative().flex_1().min_w_0().flex().items_center().gap(px(7.0))
            .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(self.title.name.clone()))
            .when(self.title.dirty, |el| el.child(div().size(px(7.0)).flex_none().rounded_full().bg(theme.accent)))
            .when(self.title.external, |el| el.child(div().id("mini-external-change").size(px(7.0)).flex_none().rounded_full()
                .bg(theme.warning).tooltip(tooltip("文件已在外部更改，返回主窗口后处理"))));
        let mut row = div().relative().size_full().flex().items_center().pl(px(11.0)).pr(px(5.0)).gap(px(8.0))
            .border_1().border_color(theme.border.opacity(0.78)).rounded(px(9.0)).bg(theme.rail_bg)
            .text_size(px(12.0)).text_color(theme.muted)
            .shadow(vec![BoxShadow { inset: false, color: theme.workspace_shadow_near, offset: point(px(0.0), px(3.0)),
                blur_radius: px(12.0), spread_radius: px(0.0) }]);
        #[cfg(windows)]
        { row = row.child(super::window_chrome::drag_surface("mini-drag")); }
        row = row.child(identity).child(div().relative().flex().items_center().gap(px(2.0))
            .child(button("mini-pin", 0, pin, pin_label, MiniEvent::Pin))
            .child(button("mini-return", 1, IconName::ArrowDownLeft, "返回主窗口 (Ctrl+Alt+M)", MiniEvent::Return)));
        let hover_view = cx.entity().downgrade();
        // 原生拖动区与按钮会遮挡父 hitbox，不能依靠父层 on_hover。
        // 捕获鼠标经过的几何边界；只有进出时通知控制条，正文不会随鼠标移动重绘。
        let hover = gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let view = hover_view.clone();
            window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture { return; }
                let hovered = bounds.contains(&event.position) && !window.has_active_prompt();
                let _ = view.update(cx, |bar, cx| {
                    if bar.hovered != hovered { bar.hovered = hovered; cx.notify(); }
                });
            });
        }).absolute().top_0().left_0().size_full();
        div().id("markdown-mini-titlebar").absolute().top(px(8.0)).left(px(8.0)).right(px(8.0)).h(px(34.0))
            .track_focus(&self.focus).tab_group().tab_index(0)
            .child(hover)
            .child(div().relative().size_full().top(px(if reduced { 0.0 } else { -5.0 * (1.0 - reveal) })).opacity(reveal).child(row))
    }
}
