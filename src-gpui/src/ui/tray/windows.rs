//! 一个通知区域图标服务整个应用，窗口打开或关闭时保持同一原生消息入口。
use std::{mem::size_of, ptr::null_mut};

use gpui::{App, Global, Subscription, Task, Window, WindowHandle};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

use crate::theme::Theme;
use crate::ui::{toolbar_i18n::ToolbarLocale, workspace::Workspace};

const CLASS_NAME: *const u16 = windows_sys::core::w!("Nomo.Tray");
const CALLBACK: u32 = WM_APP + 0x4e;
const OPEN: u32 = 1;
const QUIT: u32 = 2;
const FIRST_WINDOW: u32 = 3;

enum Event {
    Open,
    Menu,
    TaskbarCreated,
}

struct CallbackState {
    events: mpsc::Sender<Event>,
    taskbar_created: u32,
}

struct Tray {
    hwnd: HWND,
    module: HINSTANCE,
    icon: HICON,
    variant: u16,
    _callback: Box<CallbackState>,
    _events: Task<()>,
    _theme: Subscription,
}

impl Global for Tray {}

pub(crate) fn install(cx: &mut App) -> Result<(), String> {
    if cx.try_global::<Tray>().is_some() {
        return Ok(());
    }

    let (events, mut receiver) = mpsc::channel(8);
    let callback = Box::new(CallbackState {
        events,
        taskbar_created: unsafe {
            RegisterWindowMessageW(windows_sys::core::w!("TaskbarCreated"))
        },
    });
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: module,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // TaskbarCreated 广播只发给顶层窗口。这个隐藏窗口没有任务栏按钮，也不依附文档窗口。
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            CLASS_NAME,
            WS_POPUP,
            0, 0, 0, 0,
            null_mut(),
            null_mut(),
            module,
            (&*callback as *const CallbackState).cast(),
        )
    };
    if hwnd.is_null() {
        let error = std::io::Error::last_os_error().to_string();
        unsafe { UnregisterClassW(CLASS_NAME, module) };
        return Err(error);
    }

    let mut tray = Tray {
        hwnd,
        module,
        icon: null_mut(),
        variant: 0,
        _callback: callback,
        _events: cx.spawn(async move |cx| {
            while let Some(event) = receiver.recv().await {
                match event {
                    Event::Open => cx.update(|cx| reveal(None, cx)),
                    Event::Menu => {
                        let (locale, windows) = cx.update(|cx| (ToolbarLocale::current(cx), workspaces(cx)));
                        // 原生菜单有嵌套消息循环。显示期间不借用 GPUI 或托盘状态。
                        let choice = show_menu(hwnd, locale, &windows);
                        cx.update(|cx| {
                            if cx.try_global::<Tray>().is_none() {
                                return;
                            }
                            match choice {
                                OPEN => reveal(None, cx),
                                QUIT => crate::quit_workspaces(cx),
                                choice if choice >= FIRST_WINDOW => {
                                    if let Some(window) = windows.get((choice - FIRST_WINDOW) as usize) {
                                        reveal(Some(window.handle), cx);
                                    }
                                }
                                _ => {}
                            }
                        });
                    }
                    Event::TaskbarCreated => cx.update(|cx| {
                        if let Err(error) = sync_icon(cx, true) {
                            crate::app_logger::warn("Tray", &error);
                        }
                    }),
                }
            }
        }),
        _theme: cx.observe_global::<Theme>(refresh_icon),
    };
    let variant = icon_variant(cx.global::<Theme>().dark, any_window_shown(cx));
    let result = tray.sync(variant, ToolbarLocale::current(cx), true);
    cx.set_global(tray);
    result
}

pub(crate) fn shutdown(cx: &mut App) {
    if cx.try_global::<Tray>().is_some() {
        cx.remove_global::<Tray>();
    }
}

pub(crate) fn refresh_icon(cx: &mut App) {
    if let Err(error) = sync_icon(cx, false) {
        crate::app_logger::warn("Tray", &error);
    }
}

pub(crate) fn refresh_locale(cx: &mut App) -> Result<(), String> {
    let locale = ToolbarLocale::current(cx);
    if let Some(tray) = cx.try_global::<Tray>() && !tray.icon.is_null() {
        tray.register(tray.icon, locale, NIM_MODIFY)?;
    }
    Ok(())
}

fn sync_icon(cx: &mut App, readd: bool) -> Result<(), String> {
    let Some(previous) = cx.try_global::<Tray>().map(|tray| tray.variant) else {
        return Ok(());
    };
    let variant = icon_variant(cx.global::<Theme>().dark, any_window_shown(cx));
    if !readd && variant == previous {
        return Ok(());
    }
    let locale = ToolbarLocale::current(cx);
    cx.global_mut::<Tray>().sync(variant, locale, readd || previous == 0)
}

fn icon_variant(dark: bool, shown: bool) -> u16 {
    // 对应 build.rs 中四个 v0.6.2 托盘图标资源。
    2 + 2 * u16::from(dark) + u16::from(!shown)
}

impl Tray {
    fn sync(&mut self, variant: u16, locale: ToolbarLocale, readd: bool) -> Result<(), String> {
        let icon = unsafe {
            LoadImageW(
                self.module,
                variant as usize as *const u16,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            ) as HICON
        };
        if icon.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let result = if readd {
            self.register(icon, locale, NIM_ADD)
                .or_else(|_| self.register(icon, locale, NIM_MODIFY))
        } else {
            self.register(icon, locale, NIM_MODIFY)
        };
        if let Err(error) = result {
            unsafe { DestroyIcon(icon) };
            return Err(error);
        }
        if !self.icon.is_null() {
            unsafe { DestroyIcon(self.icon) };
        }
        self.icon = icon;
        self.variant = variant;
        Ok(())
    }

    fn register(
        &self,
        icon: HICON,
        locale: ToolbarLocale,
        command: NOTIFY_ICON_MESSAGE,
    ) -> Result<(), String> {
        let mut data = self.icon_data();
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.hIcon = icon;
        data.uCallbackMessage = CALLBACK;
        let mut tooltip = [0; 128];
        let tip = locale.app_text("tray.restoreTip");
        for (slot, character) in tooltip.iter_mut().take(127).zip(tip.encode_utf16()) {
            *slot = character;
        }
        data.szTip = tooltip;
        if unsafe { Shell_NotifyIconW(command, &data) } == 0 {
            let key = if command == NIM_ADD { "tray.createFailed" } else { "tray.refreshFailed" };
            return Err(locale.app_text(key).into());
        }
        Ok(())
    }

    fn icon_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // 先注销并销毁原生入口，再让回调数据与事件任务释放。
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.icon_data());
            DestroyWindow(self.hwnd);
            UnregisterClassW(CLASS_NAME, self.module);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 指针由 Tray 持有，和原生窗口在同一线程，销毁窗口后才释放。
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        } else if let Some(state) =
            (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const CallbackState).as_ref()
        {
            let event = match message {
                CALLBACK => match lparam as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => Some(Event::Open),
                    WM_RBUTTONUP => Some(Event::Menu),
                    _ => None,
                },
                message if message == state.taskbar_created && message != 0 => Some(Event::TaskbarCreated),
                _ => None,
            };
            if let Some(event) = event {
                let _ = state.events.try_send(event);
                return 0;
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn window_hwnd(window: &Window) -> Result<HWND, String> {
    match HasWindowHandle::window_handle(window)
        .map_err(|error| error.to_string())?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
        _ => Err("无法取得 Windows 窗口句柄".into()),
    }
}

fn window_shown(window: &Window) -> bool {
    window_hwnd(window).is_ok_and(|hwnd| unsafe { IsWindowVisible(hwnd) != 0 })
}

fn any_window_shown(cx: &mut App) -> bool {
    cx.windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<Workspace>())
        .any(|handle| handle.update(cx, |_, window, _| window_shown(window)).unwrap_or(false))
}

pub(crate) fn hide_window(window: &Window, cx: &mut App) -> Result<(), String> {
    let hwnd = window_hwnd(window)?;
    let locale = ToolbarLocale::current(cx);
    if cx.global::<Tray>().icon.is_null() {
        let variant = icon_variant(cx.global::<Theme>().dark, true);
        cx.global_mut::<Tray>().sync(variant, locale, true)?;
    }
    let tray = cx.global::<Tray>();
    // Shell 可能刚刚重启。确认恢复入口已经注册后再隐藏文档窗口。
    tray.register(tray.icon, locale, NIM_MODIFY)
        .or_else(|_| tray.register(tray.icon, locale, NIM_ADD))?;
    if unsafe { ShowWindowAsync(hwnd, SW_HIDE) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub(crate) fn restore_window(window: &Window) -> Result<(), String> {
    let hwnd = window_hwnd(window)?;
    unsafe {
        let command = if IsIconic(hwnd) != 0 {
            SW_RESTORE
        } else if IsWindowVisible(hwnd) == 0 {
            SW_SHOW
        } else {
            return Ok(());
        };
        // 同步显示会在 Workspace 仍被借用时触发 GPUI 绘制。
        if ShowWindowAsync(hwnd, command) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

struct TrayWindow {
    handle: WindowHandle<Workspace>,
    shown: bool,
    title: String,
}

fn workspaces(cx: &mut App) -> Vec<TrayWindow> {
    let mut windows: Vec<_> = cx.windows()
        .into_iter()
        .filter_map(|handle| {
            let handle = handle.downcast::<Workspace>()?;
            handle.update(cx, |_, window, _| TrayWindow {
                handle,
                shown: window_shown(window),
                title: window.window_title(),
            }).ok()
        })
        .collect();
    windows.sort_by_key(|window| gpui::AnyWindowHandle::from(window.handle).window_id().as_u64());
    windows
}

fn reveal(target: Option<WindowHandle<Workspace>>, cx: &mut App) {
    let handle = target.or_else(|| {
        let preferred = crate::preferred_workspace(cx);
        let windows = workspaces(cx);
        let has_shown = windows.iter().any(|window| window.shown);
        let mut candidates = windows.iter().filter(|window| !has_shown || window.shown);
        candidates.clone().find(|window| Some(window.handle) == preferred)
            .or_else(|| candidates.next()).map(|window| window.handle)
    });
    if let Some(handle) = handle {
        let _ = handle.update(cx, |workspace, window, cx| workspace.reveal_window(window, cx));
    }
}

fn show_menu(hwnd: HWND, locale: ToolbarLocale, windows: &[TrayWindow]) -> u32 {
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(menu, MF_STRING, OPEN as usize, wide(locale.app_text("tray.open")).as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        for (index, window) in windows.iter().enumerate() {
            let marker = if window.shown { "●" } else { "○" };
            // Win32 将 & 解释为助记键，文档标题需要按原样显示。
            let title = format!("{marker} {}", window.title.replace('&', "&&"));
            AppendMenuW(menu, MF_STRING, FIRST_WINDOW as usize + index, wide(&title).as_ptr());
        }
        if !windows.is_empty() {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }
        AppendMenuW(menu, MF_STRING, QUIT as usize, wide(locale.app_text("tray.quit")).as_ptr());
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, hwnd, std::ptr::null());
        // Win32 需要一次消息循环切换，避免下次右键菜单立即关闭。
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        choice as u32
    }
}
