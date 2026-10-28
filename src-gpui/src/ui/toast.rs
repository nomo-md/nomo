//! 底部短提示只保存最新一条有界消息；替换或关闭窗口会取消旧计时任务。
use super::{motion, popup::reduced_motion};
use crate::theme::Theme;
use gpui::{Context, FocusHandle, IntoElement, Render, SharedString, Task, Window, div, prelude::*, px, relative};
use std::time::Duration;

pub(crate) struct Toast {
    message: Option<SharedString>,
    visibility: motion::ScalarMotion,
    timer: Option<Task<()>>,
    generation: u64,
    persistent: bool,
    dismiss_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

impl Toast {
    pub(crate) fn new_with(message: SharedString, persistent: bool, cx: &mut Context<Self>) -> Self {
        let mut toast = Self { message: None, visibility: motion::ScalarMotion::default(), timer: None, generation: 0, persistent: false,
            dismiss_focus: cx.focus_handle(), return_focus: None };
        toast.show_with(message, persistent, cx);
        toast
    }

    pub(crate) fn show_with(&mut self, message: SharedString, persistent: bool, cx: &mut Context<Self>) {
        // 固定底栏移除后，失败提示保留到关闭；仍只保存一条、最多 8 KiB。
        // 普通通知不覆盖尚未关闭的失败，避免自动保存消息掩盖错误。
        if self.persistent && self.message.is_some() && !persistent { return; }
        self.timer = None;
        self.persistent = persistent;
        if !persistent { self.return_focus = None; }
        let mut end = if message.len() > 8 * 1024 { 8 * 1024 - '…'.len_utf8() } else { message.len() };
        while !message.is_char_boundary(end) { end -= 1; }
        self.message = Some(if end < message.len() { format!("{}…", &message[..end]).into() } else { message });
        self.visibility.set(true, reduced_motion(cx), Duration::from_millis(200));
        self.generation = self.generation.wrapping_add(1);
        if persistent { cx.notify(); return; }
        let generation = self.generation;
        let fade = Duration::from_millis(200);
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(3500)).await;
            if !this.update(cx, |toast, cx| {
                if toast.generation != generation { return false; }
                toast.visibility.set(false, reduced_motion(cx), Duration::from_millis(200)); cx.notify(); true
            }).unwrap_or(false) { return; }
            cx.background_executor().timer(fade).await;
            let _ = this.update(cx, |toast, cx| {
                if toast.generation != generation { return; }
                toast.message = None; toast.return_focus = None; toast.visibility.reset(); cx.notify();
            });
        }));
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None;
        self.persistent = false;
        self.timer = None;
        self.generation = self.generation.wrapping_add(1);
        self.visibility.reset();
        // 关闭按钮可获得键盘焦点；移除它时恢复之前的焦点，不能让快捷键落到已消失的元素。
        // FocusHandle 不持有编辑器实体，关闭提示后立即释放记录。
        if let Some(focus) = self.return_focus.take() { window.focus(&focus, cx); }
        cx.notify();
        window.refresh();
    }
}

impl Render for Toast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(message) = self.message.clone() else { return div().into_any_element(); };
        if self.persistent && !self.dismiss_focus.is_focused(window) {
            if let Some(focus) = window.focused(cx) { self.return_focus = Some(focus); }
        }
        let theme = Theme::ui(cx);
        let reduced = reduced_motion(cx);
        self.visibility.retime(reduced, Duration::from_millis(200));
        let progress = self.visibility.value();
        if self.visibility.running() || Theme::ui_transition_active(cx) { window.request_animation_frame(); }
        let message = if self.persistent {
            div().flex().items_start().gap(px(12.0))
                .child(div().id("toast-error-text").min_w_0().max_h(px(136.0)).overflow_y_scroll().child(message))
                .child(div().id("toast-dismiss").track_focus(&self.dismiss_focus).tab_index(0).cursor_pointer().flex_none()
                    .tooltip(super::components::tooltip(super::toolbar_i18n::ToolbarLocale::current(cx).text("close")))
                    .child("×")
                    .on_click(cx.listener(|toast, _, window, cx| { toast.dismiss(window, cx); cx.stop_propagation(); }))
                    .on_key_down(cx.listener(|toast, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "escape") {
                            toast.dismiss(window, cx); cx.stop_propagation();
                        }
                    }))).into_any_element()
        } else { div().child(message).into_any_element() };
        // 仅持久错误卡片接受点击；空白区域仍交给编辑器。
        div().absolute().top_0().left_0().size_full().flex().items_end().justify_center().pb(px(44.0)).px(px(16.0))
            .child(div().relative().top(px(if reduced { 0.0 } else { 6.0 * (1.0 - progress) }))
                .max_w((window.viewport_size().width - px(32.0)).max(px(0.0))).max_h(px(160.0)).overflow_hidden()
                .px(px(20.0)).py(px(8.0)).rounded(px(8.0)).bg(theme.fg).text_color(theme.app_bg)
                .text_size(px(13.0)).line_height(relative(1.5)).opacity(progress)
                .when(self.persistent, |el| el.occlude().on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation()))
                .child(message))
            .into_any_element()
    }
}
