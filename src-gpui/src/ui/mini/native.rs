//! 原地切换同一个 HWND；只保存几何和窗口样式，不创建第二个窗口或正文快照。
#[cfg(windows)]
mod platform {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{Foundation::HWND as ComHwnd, System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED}, UI::Shell::{ITaskbarList, TaskbarList}};
    use windows_sys::Win32::{Foundation::*, Graphics::{Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED},
        Gdi::{GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST}},
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
    }

    fn failure(action: &str) -> String { format!("{action}：{}", std::io::Error::last_os_error()) }

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
        pub(crate) fn enter(window: &Window, previous: Option<MiniGeometry>, pinned: bool) -> Result<Self, String> {
            if window.is_fullscreen() { return Err("请先退出全屏，再打开 Markdown Mini".into()); }
            let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?.as_raw()
                else { return Err("当前窗口不支持 Markdown Mini".into()); };
            let hwnd = handle.hwnd.get() as HWND;
            let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 { return Err(failure("读取主窗口位置失败")); }
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
            let topmost = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize != 0;
            let _transition = (placement.showCmd == SW_SHOWMAXIMIZED as u32).then(|| WindowTransition::suspend(hwnd)).transpose()?;
            // 先确认 Shell 可用；失败时还没有改动原窗口的尺寸、样式和置顶。
            taskbar(hwnd, false)?;
            let mut state = Self { hwnd, placement, style, topmost, restored: false };
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
                // subclass 不引用 Rust 堆对象；只在小窗存在，销毁窗口时也会自行注销。
                if unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0) } == 0 { return Err(failure("设置小窗尺寸约束失败")); }
                set_style(hwnd, style & !((WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize))?;
                if unsafe { IsZoomed(hwnd) } != 0 {
                    // 直接从最大化进入目标几何，避免先闪过一次正常窗口的尺寸。
                    let offset_x = info.rcWork.left - info.rcMonitor.left;
                    let offset_y = info.rcWork.top - info.rcMonitor.top;
                    let target = WINDOWPLACEMENT { showCmd: SW_SHOWNORMAL as u32, flags: 0,
                        rcNormalPosition: RECT { left: x - offset_x, top: y - offset_y, right: x + width - offset_x, bottom: y + height - offset_y },
                        ..placement };
                    if unsafe { SetWindowPlacement(hwnd, &target) } == 0 { return Err(failure("从最大化切换小窗失败")); }
                }
                if unsafe { SetWindowPos(hwnd, if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST }, x, y, width, height,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_FRAMECHANGED) } == 0 { return Err(failure("切换小窗尺寸失败")); }
                Ok(())
            })();
            if let Err(error) = result {
                return Err(match state.restore() { Ok(()) => error, Err(restore) => format!("{error}；恢复窗口失败：{restore}") });
            }
            Ok(state)
        }

        pub(crate) fn geometry(&self) -> Option<MiniGeometry> {
            let mut rect = RECT::default();
            (unsafe { GetWindowRect(self.hwnd, &mut rect) } != 0).then(|| MiniGeometry {
                rect, scale: (unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0).max(0.1),
            })
        }

        pub(crate) fn set_pinned(&self, pinned: bool) -> Result<(), String> { pin(self.hwnd, pinned) }

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

    unsafe extern "system" fn subclass(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
        if message == WM_NCDESTROY { unsafe { RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS); } }
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

#[cfg(windows)]
pub(crate) use platform::{MiniGeometry, MiniWindow};

#[cfg(not(windows))]
#[derive(Clone, Copy)]
pub(crate) struct MiniGeometry;
#[cfg(not(windows))]
pub(crate) struct MiniWindow;
#[cfg(not(windows))]
impl MiniWindow {
    pub(crate) fn enter(_: &gpui::Window, _: Option<MiniGeometry>, _: bool) -> Result<Self, String> { Err("当前平台尚未接入 Markdown Mini 原生窗口".into()) }
    pub(crate) fn geometry(&self) -> Option<MiniGeometry> { None }
    pub(crate) fn set_pinned(&self, _: bool) -> Result<(), String> { Err("当前平台不支持小窗置顶".into()) }
    pub(crate) fn restore(&mut self) -> Result<(), String> { Ok(()) }
}
