//! 托盘只在用户选择隐藏时创建；平台回调只发送有界意图，实际退出交回工作区确认。
#[cfg(windows)]
mod platform {
    use gpui::{App, Window};
    use crate::ui::toolbar_i18n::ToolbarLocale;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use windows_sys::Win32::{Foundation::*, UI::{Shell::*, WindowsAndMessaging::*}};
    const CALLBACK: u32 = WM_APP + 0x4e;
    const SUBCLASS: usize = 0x4e4f4d4f;
    struct TrayState { exit: AtomicBool, locale: AtomicU8 }
    pub(crate) struct TrayIcon { hwnd: HWND, data: NOTIFYICONDATAW, state: Box<TrayState> }
    impl TrayIcon {
        pub(crate) fn hide(window: &Window, locale: ToolbarLocale) -> Result<Self,String> {
            let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?.as_raw() else { return Err(locale.app_text("tray.windowUnavailable").into()); };
            let hwnd = handle.hwnd.get() as HWND;
            let mut data = NOTIFYICONDATAW::default();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32; data.hWnd=hwnd; data.uID=1;
            data.uFlags=NIF_ICON|NIF_MESSAGE|NIF_TIP; data.uCallbackMessage=CALLBACK;
            let mut state=Box::new(TrayState { exit: AtomicBool::new(false), locale: AtomicU8::new(locale.id()) });
            // 复用窗口已有的共享图标，所有 Win32 调用都在窗口所属 UI 线程执行。
            unsafe {
                data.hIcon=SendMessageW(hwnd,WM_GETICON,ICON_SMALL as usize,0) as _;
                if data.hIcon.is_null() {data.hIcon=LoadIconW(std::ptr::null_mut(),IDI_APPLICATION);}
                let tip:Vec<u16>=locale.app_text("tray.restoreTip").encode_utf16().chain(Some(0)).collect();
                data.szTip[..tip.len()].copy_from_slice(&tip);
                if SetWindowSubclass(hwnd,Some(callback),SUBCLASS,(&mut *state as *mut TrayState) as usize)==0 {return Err(locale.app_text("tray.callbackFailed").into());}
                if Shell_NotifyIconW(NIM_ADD,&data)==0 {RemoveWindowSubclass(hwnd,Some(callback),SUBCLASS);return Err(locale.app_text("tray.createFailed").into());}
                ShowWindow(hwnd,SW_HIDE);
            }
            Ok(Self {hwnd,data,state})
        }
        pub(crate) fn set_locale(&mut self, locale: ToolbarLocale, _: &App) -> Result<(), String> {
            if self.state.locale.swap(locale.id(), Ordering::AcqRel) == locale.id() { return Ok(()); }
            self.data.szTip.fill(0);
            let tip: Vec<u16> = locale.app_text("tray.restoreTip").encode_utf16().take(127).collect();
            self.data.szTip[..tip.len()].copy_from_slice(&tip);
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) } == 0 { return Err(locale.app_text("tray.refreshFailed").into()); }
            Ok(())
        }
        pub(crate) fn hide_again(&self) {unsafe {ShowWindow(self.hwnd,SW_HIDE);}}
        pub(crate) fn restore(&self) {unsafe {ShowWindow(self.hwnd,SW_SHOW);SetForegroundWindow(self.hwnd);}}
        pub(crate) fn exit_requested(&self)->bool {self.state.exit.swap(false,Ordering::Relaxed)}
    }
    impl Drop for TrayIcon {fn drop(&mut self){unsafe{Shell_NotifyIconW(NIM_DELETE,&self.data);RemoveWindowSubclass(self.hwnd,Some(callback),SUBCLASS);}}}
    unsafe extern "system" fn callback(hwnd:HWND,message:u32,wparam:WPARAM,lparam:LPARAM,_id:usize,data:usize)->LRESULT {
        if message==CALLBACK {
            match lparam as u32 {
                WM_LBUTTONUP|WM_LBUTTONDBLCLK=>unsafe{ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);},
                WM_RBUTTONUP=>unsafe{
                    let menu=CreatePopupMenu();
                    let locale=ToolbarLocale::from_id((*(data as *const TrayState)).locale.load(Ordering::Acquire));
                    let open:Vec<u16>=locale.app_text("tray.open").encode_utf16().chain(Some(0)).collect();
                    let quit:Vec<u16>=locale.app_text("tray.quit").encode_utf16().chain(Some(0)).collect();
                    AppendMenuW(menu,MF_STRING,1,open.as_ptr());AppendMenuW(menu,MF_STRING,2,quit.as_ptr());
                    let mut point=POINT::default();GetCursorPos(&mut point);SetForegroundWindow(hwnd);
                    let choice=TrackPopupMenu(menu,TPM_RETURNCMD|TPM_RIGHTBUTTON,point.x,point.y,0,hwnd,std::ptr::null());DestroyMenu(menu);
                    if choice==1 {ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);}
                    if choice==2 {(*(data as *const TrayState)).exit.store(true,Ordering::Relaxed);ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);PostMessageW(hwnd,WM_CLOSE,0,0);}
                },
                _=>{}
            }
            return 0;
        }
        unsafe{DefSubclassProc(hwnd,message,wparam,lparam)}
    }
}
#[cfg(windows)]
pub(crate) use platform::TrayIcon;
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

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(crate) struct TrayIcon;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl TrayIcon {
    pub(crate) fn hide(_: &gpui::Window, _: crate::ui::toolbar_i18n::ToolbarLocale)->Result<Self,String>{Err("当前平台尚未提供托盘；窗口保持打开".into())}
    pub(crate) fn set_locale(&mut self, _: crate::ui::toolbar_i18n::ToolbarLocale, _: &gpui::App) -> Result<(), String> { Ok(()) }
    pub(crate) fn hide_again(&self){}
    pub(crate) fn restore(&self){}
    pub(crate) fn exit_requested(&self)->bool{false}
}
