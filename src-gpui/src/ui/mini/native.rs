//! 原地切换同一个原生窗口；只保存几何和窗口属性，不创建第二个窗口或正文快照。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MiniTarget { Mini, Main }

pub(crate) enum MiniFrame { More, Settled(MiniTarget) }

#[cfg(windows)]
mod platform {
    use super::{MiniFrame, MiniTarget};
    use super::super::bounds_motion::{BoundsMotion, same_rect};
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{Foundation::HWND as ComHwnd, System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED}, UI::Shell::{ITaskbarList, TaskbarList}};
    use windows_sys::Win32::{Foundation::*, Graphics::{Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED},
        Gdi::{GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL}},
        UI::{HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI},
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass}, WindowsAndMessaging::*}};

    const SUBCLASS: usize = 0x4e4d494e;

    #[derive(Clone, Copy)]
    pub(crate) struct MiniGeometry { rect: RECT, scale: f32 }

    pub(crate) struct MiniWindow {
        hwnd: HWND,
        placement: WINDOWPLACEMENT,
        style: isize,
        topmost: bool,
        restored: bool,
        main_rect: RECT,
        mini: MiniGeometry,
        motion: Option<BoundsMotion>,
    }

    fn failure(action: &str) -> String { format!("{action}：{}", std::io::Error::last_os_error()) }

    fn window_rect(hwnd: HWND) -> Result<RECT, String> {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 { return Err(failure("读取窗口尺寸失败")); }
        Ok(rect)
    }

    fn same_screen(a: RECT, b: RECT) -> bool {
        let monitor = unsafe { MonitorFromRect(&a, MONITOR_DEFAULTTONULL) };
        !monitor.is_null() && monitor == unsafe { MonitorFromRect(&b, MONITOR_DEFAULTTONULL) }
    }

    fn place_bounds(hwnd: HWND, rect: RECT) -> Result<(), String> {
        if unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), rect.left, rect.top, rect.right - rect.left,
            rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER) } == 0 {
            return Err(failure("调整小窗过渡位置失败"));
        }
        Ok(())
    }

    fn drag_epoch(hwnd: HWND) -> Option<usize> {
        let mut epoch = 0;
        (unsafe { GetWindowSubclass(hwnd, Some(subclass), SUBCLASS, &mut epoch) } != 0).then_some(epoch)
    }

    fn controls_rect(hwnd: HWND, motion: &BoundsMotion, current: RECT) -> bool {
        drag_epoch(hwnd) == Some(motion.drag_epoch) && same_rect(current, motion.last)
    }

    fn set_transition_disabled(hwnd: HWND, disabled: bool) -> Result<(), String> {
        let value = i32::from(disabled);
        let result = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            (&value as *const i32).cast(), std::mem::size_of::<i32>() as u32) };
        if result < 0 { return Err(format!("更新小窗系统过渡失败：HRESULT 0x{:08X}", result as u32)); }
        Ok(())
    }

    struct WindowTransition(HWND);
    impl WindowTransition {
        fn suspend(hwnd: HWND) -> Result<Self, String> { set_transition_disabled(hwnd, true)?; Ok(Self(hwnd)) }
    }
    impl Drop for WindowTransition {
        fn drop(&mut self) {
            // Nomo/GPUI 不设置该属性；仅本次最大化切换关闭系统位图拉伸，随后恢复默认策略。
            if let Err(error) = set_transition_disabled(self.0, false) { eprintln!("{error}"); }
        }
    }

    fn taskbar(hwnd: HWND, visible: bool) -> Result<(), String> {
        // GPUI 的 UI 线程使用 COM；每次成功初始化都有对应释放，接口不跨线程保留。
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|error| format!("初始化任务栏接口失败：{error}"))?;
            let result = (|| {
                let taskbar: ITaskbarList = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)?;
                taskbar.HrInit()?;
                if visible { taskbar.AddTab(ComHwnd(hwnd)) } else { taskbar.DeleteTab(ComHwnd(hwnd)) }
            })();
            CoUninitialize();
            result.map_err(|error| format!("更新小窗任务栏状态失败：{error}"))
        }
    }

    fn set_style(hwnd: HWND, style: isize) -> Result<(), String> {
        unsafe {
            SetLastError(0);
            if SetWindowLongPtrW(hwnd, GWL_STYLE, style) == 0 && GetLastError() != 0 { return Err(failure("更新小窗样式失败")); }
        }
        Ok(())
    }

    fn pin(hwnd: HWND, pinned: bool) -> Result<(), String> {
        if unsafe { SetWindowPos(hwnd, if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST }, 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER) } == 0 { return Err(failure("更新小窗置顶失败")); }
        Ok(())
    }

    impl MiniWindow {
        pub(crate) fn enter(window: &Window, previous: Option<MiniGeometry>, pinned: bool, animate: bool) -> Result<Self, String> {
            if window.is_fullscreen() { return Err("请先退出全屏，再打开 Markdown Mini".into()); }
            let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?.as_raw()
                else { return Err("当前窗口不支持 Markdown Mini".into()); };
            let hwnd = handle.hwnd.get() as HWND;
            let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 { return Err(failure("读取主窗口位置失败")); }
            let main_rect = window_rect(hwnd)?;
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
            let topmost = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize != 0;
            let _transition = (placement.showCmd == SW_SHOWMAXIMIZED as u32).then(|| WindowTransition::suspend(hwnd)).transpose()?;
            // 先确认 Shell 可用；失败时还没有改动原窗口的尺寸、样式和置顶。
            taskbar(hwnd, false)?;
            let mut state = Self { hwnd, placement, style, topmost, restored: false, main_rect, motion: None,
                mini: MiniGeometry { rect: main_rect, scale: window.scale_factor() } };
            let result = (|| {
                let monitor = unsafe { previous.map_or_else(|| MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), |geometry| MonitorFromRect(&geometry.rect, MONITOR_DEFAULTTONEAREST)) };
                let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 { return Err(failure("读取小窗屏幕范围失败")); }
                let (mut dpi_x, mut dpi_y) = (0, 0);
                let scale = if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } >= 0 && dpi_x > 0 {
                    dpi_x as f32 / 96.0
                } else { window.scale_factor().max(0.1) };
                // GPUI 自绘标题栏只保留左右和底部的系统缩放边框。
                let dpi = (scale * 96.0).round() as u32;
                let frame = unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
                let area = info.rcWork;
                let width = previous.map_or((460.0 * scale).round() as i32 + frame * 2, |g| ((g.rect.right - g.rect.left) as f32 * scale / g.scale).round() as i32)
                    .max((320.0 * scale).round() as i32 + frame * 2).min(area.right - area.left);
                let height = previous.map_or((560.0 * scale).round() as i32 + frame, |g| ((g.rect.bottom - g.rect.top) as f32 * scale / g.scale).round() as i32)
                    .max((240.0 * scale).round() as i32 + frame).min(area.bottom - area.top);
                let x = previous.map_or(area.left + (area.right - area.left - width) / 2, |g| g.rect.left).clamp(area.left, area.right - width);
                let y = previous.map_or(area.top + (area.bottom - area.top - height) / 2, |g| g.rect.top).clamp(area.top, area.bottom - height);
                let target_rect = RECT { left: x, top: y, right: x + width, bottom: y + height };
                state.mini = MiniGeometry { rect: target_rect, scale };
                let moving = animate && !same_rect(main_rect, target_rect) && same_screen(main_rect, target_rect);
                let first = if moving { main_rect } else { target_rect };
                // subclass 不引用 Rust 堆对象；只在小窗存在，销毁窗口时也会自行注销。
                if unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0) } == 0 { return Err(failure("设置小窗尺寸约束失败")); }
                set_style(hwnd, style & !((WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize))?;
                if unsafe { IsZoomed(hwnd) } != 0 {
                    // 在原来的屏幕矩形解除最大化，避免先闪过最大化前的普通窗口。
                    let offset_x = info.rcWork.left - info.rcMonitor.left;
                    let offset_y = info.rcWork.top - info.rcMonitor.top;
                    let target = WINDOWPLACEMENT { showCmd: SW_SHOWNORMAL as u32, flags: 0,
                        rcNormalPosition: RECT { left: first.left - offset_x, top: first.top - offset_y,
                            right: first.right - offset_x, bottom: first.bottom - offset_y },
                        ..placement };
                    if unsafe { SetWindowPlacement(hwnd, &target) } == 0 { return Err(failure("从最大化切换小窗失败")); }
                }
                if unsafe { SetWindowPos(hwnd, if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST },
                    first.left, first.top, first.right - first.left, first.bottom - first.top,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_FRAMECHANGED) } == 0 { return Err(failure("切换小窗尺寸失败")); }
                if moving {
                    state.motion = Some(BoundsMotion::new(window_rect(hwnd)?, target_rect, MiniTarget::Mini, 0));
                }
                Ok(())
            })();
            if let Err(error) = result {
                return Err(match state.restore() { Ok(()) => error, Err(restore) => format!("{error}；恢复窗口失败：{restore}") });
            }
            Ok(state)
        }

        pub(crate) fn geometry(&self) -> MiniGeometry {
            // 返回和快速折返都记住真正的小窗尺寸，不能把中间帧当成下次进入的位置。
            match self.live_geometry() {
                Ok(current) if self.motion.as_ref().is_some_and(|motion| controls_rect(self.hwnd, motion, current.rect)) => self.mini,
                Ok(current) => current,
                Err(error) => { eprintln!("{error}"); self.mini }
            }
        }

        fn live_geometry(&self) -> Result<MiniGeometry, String> {
            let rect = window_rect(self.hwnd)?;
            let scale = (unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0).max(0.1);
            Ok(MiniGeometry { rect, scale })
        }

        pub(crate) fn set_pinned(&self, pinned: bool) -> Result<(), String> { pin(self.hwnd, pinned) }

        pub(crate) fn moving_to(&self) -> Option<MiniTarget> { self.motion.as_ref().map(|m| m.target) }
        pub(crate) fn progress(&self) -> Option<f32> { self.motion.as_ref().map(BoundsMotion::progress) }
        pub(crate) fn resume_after_handoff(&mut self) {
            if let Some(motion) = &mut self.motion { motion.resume_after_handoff(); }
        }

        pub(crate) fn transition_to(&mut self, target: MiniTarget, animate: bool) -> Result<bool, String> {
            if let Some(motion) = &mut self.motion {
                motion.redirect(target);
                if animate { return Ok(true); }
                if target == MiniTarget::Mini { self.settle_mini()?; }
                return Ok(false);
            }
            if target == MiniTarget::Mini || !animate { return Ok(false); }
            self.mini = self.live_geometry()?;
            if same_rect(self.main_rect, self.mini.rect) || !same_screen(self.main_rect, self.mini.rect) { return Ok(false); }
            let epoch = drag_epoch(self.hwnd).ok_or_else(|| failure("读取小窗拖动状态失败"))?;
            self.motion = Some(BoundsMotion::new(self.main_rect, self.mini.rect, target, epoch));
            Ok(true)
        }

        pub(crate) fn advance(&mut self, reduced: bool, handoff: bool) -> Result<(MiniFrame, bool), String> {
            let Some(motion) = &mut self.motion else { return Ok((MiniFrame::Settled(MiniTarget::Mini), false)); };
            let current = window_rect(self.hwnd)?;
            // 用户拖动、系统贴靠或 DPI 调整优先，后续帧不再覆盖新的窗口位置。
            if !controls_rect(self.hwnd, motion, current) {
                self.motion = None;
                return Ok((MiniFrame::Settled(MiniTarget::Mini), true));
            }
            let (rect, done) = motion.next(reduced, handoff);
            if done && motion.target == MiniTarget::Main {
                // 最后一帧统一恢复 WINDOWPLACEMENT（含最大化）、原编辑模式和焦点。
                return Ok((MiniFrame::Settled(MiniTarget::Main), false));
            }
            let changed = !same_rect(rect, current);
            if changed {
                place_bounds(self.hwnd, rect)?;
                motion.last = window_rect(self.hwnd)?;
            }
            if done {
                self.motion = None;
                Ok((MiniFrame::Settled(MiniTarget::Mini), changed))
            } else { Ok((MiniFrame::More, changed)) }
        }

        pub(crate) fn settle_mini(&mut self) -> Result<(), String> {
            self.motion = None;
            place_bounds(self.hwnd, self.mini.rect)
        }

        pub(crate) fn restore(&mut self) -> Result<(), String> {
            if self.restored { return Ok(()); }
            let _transition = (self.placement.showCmd == SW_SHOWMAXIMIZED as u32).then(|| WindowTransition::suspend(self.hwnd)).transpose()?;
            taskbar(self.hwnd, true)?;
            // 只还原本功能改动的按钮位；WS_MAXIMIZE 等运行态交给 WINDOWPLACEMENT 恢复。
            let buttons = (WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize;
            let current = unsafe { GetWindowLongPtrW(self.hwnd, GWL_STYLE) };
            set_style(self.hwnd, (current & !buttons) | (self.style & buttons))?;
            pin(self.hwnd, self.topmost)?;
            if unsafe { SetWindowPlacement(self.hwnd, &self.placement) } == 0 { return Err(failure("恢复主窗口位置失败")); }
            if unsafe { SetWindowPos(self.hwnd, std::ptr::null_mut(), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED) } == 0 { return Err(failure("恢复主窗口边框失败")); }
            unsafe { RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS); }
            self.restored = true;
            Ok(())
        }
    }

    impl Drop for MiniWindow {
        fn drop(&mut self) {
            let mut data = 0;
            if !self.restored && unsafe { GetWindowSubclass(self.hwnd, Some(subclass), SUBCLASS, &mut data) } != 0 {
                if let Err(error) = self.restore() { eprintln!("{error}"); }
                unsafe { RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS); }
            }
        }
    }

    unsafe extern "system" fn subclass(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, epoch: usize) -> LRESULT {
        if message == WM_NCDESTROY { unsafe { RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS); } }
        if message == WM_ENTERSIZEMOVE && unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, epoch.wrapping_add(1)) } == 0 {
            eprintln!("{}", failure("记录小窗拖动状态失败"));
        }
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if message == WM_GETMINMAXINFO && lparam != 0 {
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let mut outer = RECT::default();
            let mut inner = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut outer) } != 0 && unsafe { GetClientRect(hwnd, &mut inner) } != 0 {
                let minmax = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                minmax.ptMinTrackSize.x = (320.0 * scale).round() as i32 + (outer.right - outer.left - inner.right).max(0);
                minmax.ptMinTrackSize.y = (240.0 * scale).round() as i32 + (outer.bottom - outer.top - inner.bottom).max(0);
            }
        }
        result
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
use platform as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(target_os = "linux")]
use linux as backend;

pub(crate) use backend::MiniGeometry;
#[cfg(target_os = "macos")]
pub(crate) use macos::NativeOperation as MacMiniOperation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MiniPin {
    Ready(bool),
    Pending(bool),
    Unavailable(&'static str),
}

impl MiniPin {
    pub(crate) fn pinned(self) -> bool { matches!(self, Self::Ready(true) | Self::Pending(true)) }
}

pub(crate) fn unavailable_reason(window: &gpui::Window) -> Option<&'static str> {
    unavailable_reason_key(window).map(|key| crate::ui::toolbar_i18n::ToolbarLocale::fallback().app_text(key))
}

pub(crate) fn unavailable_reason_key(window: &gpui::Window) -> Option<&'static str> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if window.is_fullscreen() || window.is_simple_fullscreen() {
        return Some("mini.fullscreen");
    }
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return Some("mini.windowUnavailable"); };
    let supported = match handle.as_raw() {
        #[cfg(windows)]
        RawWindowHandle::Win32(_) => true,
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(_) => true,
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xcb(_) | RawWindowHandle::Wayland(_) => true,
        _ => false,
    };
    (!supported).then_some("mini.backendUnavailable")
}

/// 非 Windows 快照不持有原生对象或旧视图；macOS 任务只临时保留经过校验的原生对象。
pub(crate) struct MiniWindow {
    inner: backend::MiniWindow,
    #[cfg(windows)]
    pinned: bool,
}

impl MiniWindow {
    #[cfg(target_os = "macos")]
    pub(crate) fn prepare_native_operation(&self, window: &gpui::Window) -> Option<MacMiniOperation> {
        self.inner.prepare_operation(window)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn accepts_native_operation(&self, operation: &MacMiniOperation, window: &gpui::Window) -> bool {
        self.inner.accepts_operation(operation, window)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn native_is_restored(&self) -> bool { self.inner.is_restored() }

    pub(crate) fn enter(window: &mut gpui::Window, previous: Option<MiniGeometry>, pinned: bool, animate: bool) -> Result<Self, crate::ui::toolbar_i18n::UiMessage> {
        if let Some(reason) = unavailable_reason_key(window) { return Err(crate::ui::toolbar_i18n::UiMessage::new(reason)); }
        Ok(Self { inner: backend::MiniWindow::enter(window, previous, pinned, animate)?, #[cfg(windows)] pinned })
    }

    pub(crate) fn geometry(&self, window: &gpui::Window) -> MiniGeometry {
        #[cfg(windows)] { let _ = window; self.inner.geometry() }
        #[cfg(not(windows))] { self.inner.geometry(window) }
    }

    pub(crate) fn pin_state(&self) -> MiniPin {
        #[cfg(windows)] { MiniPin::Ready(self.pinned) }
        #[cfg(not(windows))] { self.inner.pin_state() }
    }

    pub(crate) fn set_pinned(&mut self, pinned: bool, window: &mut gpui::Window) -> Result<(), crate::ui::toolbar_i18n::UiMessage> {
        #[cfg(windows)] { let _ = window; self.inner.set_pinned(pinned)?; self.pinned = pinned; Ok(()) }
        #[cfg(not(windows))] { self.inner.set_pinned(pinned, window).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
    }

    pub(crate) fn take_notice(&mut self, locale: crate::ui::toolbar_i18n::ToolbarLocale) -> Option<String> {
        #[cfg(windows)] { let _ = locale; None }
        #[cfg(target_os = "macos")] { let _ = locale; self.inner.take_notice() }
        #[cfg(target_os = "linux")] { self.inner.take_notice(locale) }
    }

    pub(crate) fn moving_to(&self) -> Option<MiniTarget> { self.inner.moving_to() }
    pub(crate) fn progress(&self) -> Option<f32> { self.inner.progress() }
    pub(crate) fn resume_after_handoff(&mut self) { self.inner.resume_after_handoff(); }

    pub(crate) fn transition_to(&mut self, target: MiniTarget, animate: bool, window: &mut gpui::Window) -> Result<bool, crate::ui::toolbar_i18n::UiMessage> {
        #[cfg(windows)] { let _ = window; self.inner.transition_to(target, animate).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
        #[cfg(not(windows))] {
            if window.is_fullscreen() || window.is_simple_fullscreen() { return Err(crate::ui::toolbar_i18n::UiMessage::new("mini.switchFullscreen")); }
            self.inner.transition_to(target, animate, window).map_err(crate::ui::toolbar_i18n::UiMessage::raw)
        }
    }

    pub(crate) fn advance(&mut self, reduced: bool, handoff: bool, window: &mut gpui::Window) -> Result<(MiniFrame, bool), crate::ui::toolbar_i18n::UiMessage> {
        #[cfg(windows)] { let _ = window; self.inner.advance(reduced, handoff).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
        #[cfg(not(windows))] { self.inner.advance(reduced, handoff, window).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
    }

    pub(crate) fn settle_mini(&mut self, window: &mut gpui::Window) -> Result<(), crate::ui::toolbar_i18n::UiMessage> {
        #[cfg(windows)] { let _ = window; self.inner.settle_mini().map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
        #[cfg(not(windows))] { self.inner.settle_mini(window).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
    }

    /// false 表示请求尚待窗口管理器确认，调用方必须保留快照并继续有期限的确认。
    pub(crate) fn restore(&mut self, window: &mut gpui::Window) -> Result<bool, crate::ui::toolbar_i18n::UiMessage> {
        #[cfg(windows)] { let _ = window; self.inner.restore().map(|()| true).map_err(crate::ui::toolbar_i18n::UiMessage::raw) }
        #[cfg(not(windows))] {
            if window.is_fullscreen() || window.is_simple_fullscreen() { return Err(crate::ui::toolbar_i18n::UiMessage::new("mini.restoreFullscreen")); }
            self.inner.restore(window).map_err(crate::ui::toolbar_i18n::UiMessage::raw)
        }
    }
}
