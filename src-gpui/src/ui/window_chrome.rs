//! 主窗口与设置窗口共用的原生窗口操作和 Windows 标题栏控件。

use gpui::Window;

/// GPUI 0.2.2 的 Windows `zoom_window` 只会最大化，不会还原。
pub(crate) fn toggle_maximize(window: &Window) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindowAsync, SW_MAXIMIZE, SW_RESTORE};

        let handle = match HasWindowHandle::window_handle(window) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("读取窗口句柄失败：{error}");
                return;
            }
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            eprintln!("读取窗口句柄失败：不是 Windows 窗口");
            return;
        };
        let command = if window.is_maximized() { SW_RESTORE } else { SW_MAXIMIZE };
        // SAFETY: 句柄来自当前借用的 GPUI 窗口，异步请求由 Windows 在窗口线程执行。
        if unsafe { ShowWindowAsync(handle.hwnd.get() as _, command) } == 0 {
            eprintln!("切换窗口最大化状态失败");
        }
    }
    #[cfg(not(target_os = "windows"))]
    window.zoom_window();
}

#[cfg(target_os = "windows")]
mod windows {
    use super::super::components::tooltip;
    use crate::theme::Theme;
    use gpui::{
        AnyElement, App, ClickEvent, Div, FocusHandle, InteractiveElement, IntoElement, MouseButton,
        ParentElement, Stateful, StatefulInteractiveElement, Styled, Window, WindowControlArea,
        div, point, prelude::*, px, rgb,
    };
    use std::rc::Rc;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum CaptionAction { Minimize, Maximize, Close }

    pub(crate) struct CaptionFocus {
        minimize: FocusHandle,
        maximize: FocusHandle,
        close: FocusHandle,
    }

    impl CaptionFocus {
        pub(crate) fn new(cx: &mut App) -> Self {
            Self {
                minimize: cx.focus_handle().tab_index(0).tab_stop(true),
                maximize: cx.focus_handle().tab_index(0).tab_stop(true),
                close: cx.focus_handle().tab_index(0).tab_stop(true),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum WindowGlyph { Minimize, Maximize, Restore, Close }

    /// 使用旧版 20×20 路径，按 16px 绘制并继承按钮文字颜色。
    fn window_glyph(glyph: WindowGlyph) -> AnyElement {
        gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let color = window.text_style().color;
            let at = |x: f32, y: f32| bounds.origin + point(px(x * 0.8), px(y * 0.8));
            let mut path = gpui::PathBuilder::stroke(px(1.0));
            let mut line = |points: &[(f32, f32)], closed: bool| {
                if let Some((x, y)) = points.first() { path.move_to(at(*x, *y)); }
                for (x, y) in points.iter().skip(1) { path.line_to(at(*x, *y)); }
                if closed { path.close(); }
            };
            match glyph {
                WindowGlyph::Minimize => line(&[(4.5, 10.5), (15.5, 10.5)], false),
                WindowGlyph::Maximize => line(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)], true),
                WindowGlyph::Restore => {
                    line(&[(6.5, 5.5), (14.5, 5.5), (14.5, 13.5), (6.5, 13.5)], true);
                    line(&[(5.0, 12.0), (5.0, 4.0), (13.0, 4.0)], false);
                }
                WindowGlyph::Close => {
                    line(&[(5.0, 5.0), (15.0, 15.0)], false);
                    line(&[(15.0, 5.0), (5.0, 15.0)], false);
                }
            }
            if let Ok(path) = path.build() { window.paint_path(path, color); }
        }).size(px(16.0)).flex_none().into_any_element()
    }

    /// 将空白标题栏交给 Windows；遮挡父视图避免焦点处理阻止系统拖动。
    pub(crate) fn drag_surface(id: &'static str) -> Stateful<Div> {
        div().id(id).absolute().top_0().right_0().bottom_0().left_0()
            .occlude().window_control_area(WindowControlArea::Drag)
    }

    /// GPUI 的 Windows 命中测试把客户区顶部作为缩放边缘；不能让标题栏焦点吞掉该事件。
    pub(crate) fn top_resize_guard(window: &Window) -> Stateful<Div> {
        use windows_sys::Win32::UI::{HiDpi::GetSystemMetricsForDpi, WindowsAndMessaging::SM_CYFRAME};

        let scale = window.scale_factor().max(0.1);
        let dpi = (96.0 * scale).round() as u32;
        // SAFETY: 查询当前 DPI 的系统指标不持有窗口或其他资源。
        let frame = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) };
        let height = if frame > 0 { (frame as f32 + 1.0) / scale } else { 9.0 };
        div().id("window-top-resize-guard").absolute().top_0().right_0().left_0()
            .h(px(height)).occlude()
    }

    pub(crate) fn caption_controls(
        theme: &Theme,
        window: &Window,
        focus: &CaptionFocus,
        on_action: impl Fn(CaptionAction, &mut Window, &mut App) + 'static,
    ) -> Div {
        let maximized = window.is_maximized();
        let on_action = Rc::new(on_action);
        let button = |id, action, glyph, area, label, focus: &FocusHandle| {
            let on_action = on_action.clone();
            let foreground = theme.fg;
            div().id(id).relative().w(px(46.0)).h_full().flex_none()
                .flex().items_center().justify_center().text_color(foreground).occlude().track_focus(focus).tab_index(0)
                .when(focus.is_focused(window), |el| el.border_2().border_color(theme.accent))
                .hover(move |style| if action == CaptionAction::Close {
                    style.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff))
                } else { style.bg(foreground.opacity(0.12)) })
                .active(move |style| if action == CaptionAction::Close {
                    style.bg(rgb(0xa3261b)).text_color(rgb(0xffffff))
                } else { style.bg(foreground.opacity(0.18)) })
                .child(window_glyph(glyph))
                // 保留 Windows 对最大化按钮的命中信息；动作仍只由下方点击处理一次。
                .child(div().absolute().top_0().right_0().bottom_0().left_0().window_control_area(area))
                .tooltip(tooltip(label))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_click(move |event: &ClickEvent, window, cx| {
                    if action == CaptionAction::Maximize
                        && matches!(event, ClickEvent::Mouse(click) if click.down.click_count > 1)
                    { return; }
                    on_action(action, window, cx);
                })
        };
        div().flex().h_full().flex_none().tab_group()
            .child(button("window-minimize", CaptionAction::Minimize, WindowGlyph::Minimize, WindowControlArea::Min, "最小化", &focus.minimize))
            .child(button("window-maximize", CaptionAction::Maximize,
                if maximized { WindowGlyph::Restore } else { WindowGlyph::Maximize },
                WindowControlArea::Max, if maximized { "还原" } else { "最大化" }, &focus.maximize))
            .child(button("window-close", CaptionAction::Close, WindowGlyph::Close, WindowControlArea::Close, "关闭", &focus.close))
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows::{CaptionAction, CaptionFocus, caption_controls, drag_surface, top_resize_guard};
