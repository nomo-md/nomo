//! 底部短提示只保存最新一条有界消息；替换或关闭窗口会取消旧计时任务。
use super::{motion, popup::reduced_motion};
use crate::theme::Theme;
use gpui::{Context, IntoElement, Render, SharedString, Task, Window, div, prelude::*, px, relative};
use std::time::Duration;

const FADE: Duration = Duration::from_millis(200);

pub(crate) struct Toast {
    message: Option<SharedString>,
    visibility: motion::ScalarMotion,
    timer: Option<Task<()>>,
    generation: u64,
}

impl Toast {
    pub(crate) fn new(message: SharedString, duration: Duration, cx: &mut Context<Self>) -> Self {
        let mut toast = Self { message: None, visibility: motion::ScalarMotion::default(), timer: None, generation: 0 };
        toast.show(message, duration, cx);
        toast
    }

    pub(crate) fn show(&mut self, message: SharedString, duration: Duration, cx: &mut Context<Self>) {
        // 对齐旧版 showToast：新提示替换旧提示并重新计时，不保留可交互的错误卡片。
        self.timer = None;
        self.generation = self.generation.wrapping_add(1);
        if message.is_empty() {
            self.message = None;
            self.visibility.reset();
            cx.notify();
            return;
        }
        let mut end = if message.len() > 8 * 1024 { 8 * 1024 - '…'.len_utf8() } else { message.len() };
        while !message.is_char_boundary(end) { end -= 1; }
        let message: SharedString = if end < message.len() { format!("{}…", &message[..end]).into() } else { message };
        // GPUI 的 nowrap 仍保留硬换行；先折叠控制空白，避免错误详情撑成多行面板。
        self.message = Some(if message.contains(['\r', '\n', '\t']) { message.replace(['\r', '\n', '\t'], " ").into() } else { message });
        self.visibility.set(true, reduced_motion(cx), FADE);
        let generation = self.generation;
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            let fade = this.update(cx, |toast, cx| {
                if toast.generation != generation { return None; }
                let reduced = reduced_motion(cx);
                toast.visibility.set(false, reduced, FADE);
                cx.notify();
                Some(if reduced { Duration::from_millis(90) } else { FADE })
            }).ok().flatten();
            let Some(fade) = fade else { return; };
            cx.background_executor().timer(fade).await;
            let _ = this.update(cx, |toast, cx| {
                if toast.generation != generation { return; }
                toast.message = None; toast.visibility.reset(); cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for Toast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(message) = self.message.clone() else { return div().absolute().into_any_element(); };
        let theme = Theme::ui(cx);
        let reduced = reduced_motion(cx);
        self.visibility.retime(reduced, FADE);
        let progress = self.visibility.value();
        if self.visibility.running() || Theme::ui_transition_active(cx) { window.request_animation_frame(); }
        // 旧版 .app-toast：底部 44px、单行、8×20px 内边距，不注册鼠标或焦点处理。
        div().absolute().left_0().bottom(px(44.0 - if reduced { 0.0 } else { 6.0 * (1.0 - progress) }))
            .w_full().flex().justify_center().px(px(16.0))
            .child(div().min_w_0().max_w((window.viewport_size().width - px(32.0)).max(px(0.0))).truncate()
                .px(px(20.0)).py(px(8.0)).rounded(px(8.0)).bg(theme.fg).text_color(theme.app_bg)
                .text_size(px(13.0)).line_height(relative(1.5)).opacity(progress)
                .child(message))
            .into_any_element()
    }
}
