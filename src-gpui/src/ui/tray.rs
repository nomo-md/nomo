//! Windows 托盘图标只在用户选择隐藏时创建，窗口释放时注销；不创建后台线程。
#[cfg(windows)]
mod platform {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::{Foundation::*, UI::{Shell::*, WindowsAndMessaging::*}};
    const CALLBACK: u32 = WM_APP + 0x4e;
    const SUBCLASS: usize = 0x4e4f4d4f;
    pub(crate) struct TrayIcon { hwnd: HWND, data: NOTIFYICONDATAW, exit: Box<AtomicBool> }
    impl TrayIcon {
        pub(crate) fn hide(window: &Window) -> Result<Self,String> {
            let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?.as_raw() else { return Err("窗口类型不支持托盘".into()); };
            let hwnd = handle.hwnd.get() as HWND;
            let mut data = NOTIFYICONDATAW::default();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32; data.hWnd=hwnd; data.uID=1;
            data.uFlags=NIF_ICON|NIF_MESSAGE|NIF_TIP; data.uCallbackMessage=CALLBACK;
            let mut exit=Box::new(AtomicBool::new(false));
            // 复用窗口已有的共享图标，所有 Win32 调用都在窗口所属 UI 线程执行。
            unsafe {
                data.hIcon=SendMessageW(hwnd,WM_GETICON,ICON_SMALL as usize,0) as _;
                if data.hIcon.is_null() {data.hIcon=LoadIconW(std::ptr::null_mut(),IDI_APPLICATION);}
                let tip:Vec<u16>="Nomo — 双击恢复窗口".encode_utf16().chain(Some(0)).collect();
                data.szTip[..tip.len()].copy_from_slice(&tip);
                if SetWindowSubclass(hwnd,Some(callback),SUBCLASS,(&mut *exit as *mut AtomicBool) as usize)==0 {return Err("无法注册托盘窗口回调".into());}
                if Shell_NotifyIconW(NIM_ADD,&data)==0 {RemoveWindowSubclass(hwnd,Some(callback),SUBCLASS);return Err("无法创建托盘图标；窗口保持打开".into());}
                ShowWindow(hwnd,SW_HIDE);
            }
            Ok(Self {hwnd,data,exit})
        }
        pub(crate) fn hide_again(&self) {unsafe {ShowWindow(self.hwnd,SW_HIDE);}}
        pub(crate) fn restore(&self) {unsafe {ShowWindow(self.hwnd,SW_SHOW);SetForegroundWindow(self.hwnd);}}
        pub(crate) fn exit_requested(&self)->bool {self.exit.swap(false,Ordering::Relaxed)}
    }
    impl Drop for TrayIcon {fn drop(&mut self){unsafe{Shell_NotifyIconW(NIM_DELETE,&self.data);RemoveWindowSubclass(self.hwnd,Some(callback),SUBCLASS);}}}
    unsafe extern "system" fn callback(hwnd:HWND,message:u32,wparam:WPARAM,lparam:LPARAM,_id:usize,data:usize)->LRESULT {
        if message==CALLBACK {
            match lparam as u32 {
                WM_LBUTTONUP|WM_LBUTTONDBLCLK=>unsafe{ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);},
                WM_RBUTTONUP=>unsafe{
                    let menu=CreatePopupMenu();
                    let open:Vec<u16>="打开 Nomo".encode_utf16().chain(Some(0)).collect();
                    let quit:Vec<u16>="退出 Nomo".encode_utf16().chain(Some(0)).collect();
                    AppendMenuW(menu,MF_STRING,1,open.as_ptr());AppendMenuW(menu,MF_STRING,2,quit.as_ptr());
                    let mut point=POINT::default();GetCursorPos(&mut point);SetForegroundWindow(hwnd);
                    let choice=TrackPopupMenu(menu,TPM_RETURNCMD|TPM_RIGHTBUTTON,point.x,point.y,0,hwnd,std::ptr::null());DestroyMenu(menu);
                    if choice==1 {ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);}
                    if choice==2 {(*(data as *const AtomicBool)).store(true,Ordering::Relaxed);ShowWindow(hwnd,SW_SHOW);SetForegroundWindow(hwnd);PostMessageW(hwnd,WM_CLOSE,0,0);}
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
#[cfg(not(windows))]
pub(crate) struct TrayIcon;
#[cfg(not(windows))]
impl TrayIcon {
    pub(crate) fn hide(_: &gpui::Window)->Result<Self,String>{Err("当前平台尚未提供托盘；窗口保持打开".into())}
    pub(crate) fn hide_again(&self){}
    pub(crate) fn restore(&self){}
    pub(crate) fn exit_requested(&self)->bool{false}
}
