//! Windows 使用应用级托盘，macOS 和 Linux 的托盘随隐藏的工作区管理。
#[cfg(windows)]
#[path = "tray/windows.rs"]
pub(crate) mod windows;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod events;
#[cfg(target_os = "macos")]
#[path = "tray/macos.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "tray/linux.rs"]
mod native;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use events::TrayEvent;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use native::{TrayIcon, restore_window};

/// 判断当前窗口后端，不以桌面环境变量推测 X11／Wayland。
pub(crate) fn unavailable_reason(window: &gpui::Window) -> Option<&'static str> {
    unavailable_reason_key(window).map(|key| crate::ui::toolbar_i18n::ToolbarLocale::fallback().app_text(key))
}

pub(crate) fn unavailable_reason_key(window: &gpui::Window) -> Option<&'static str> {
    #[cfg(windows)]
    { let _ = window; None }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    { native::unavailable_reason_key(window) }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    { let _ = window; Some("tray.platformUnavailable") }
}

pub(crate) fn supported(window: &gpui::Window) -> bool {
    unavailable_reason(window).is_none()
}
