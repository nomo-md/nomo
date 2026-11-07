//! SNI 与固定 D-Bus 菜单按需注册；只有可恢复的 X11 窗口可以隐藏。
#[path = "linux_menu.rs"]
mod menu;

use super::events::{TrayEvent, TrayEvents};
use crate::ui::toolbar_i18n::ToolbarLocale;
use ashpd::zbus::export::futures_core::Stream;
use ashpd::zbus::{self, Connection, MatchRule, MessageStream, Proxy, message::Type};
use ashpd::zvariant::{OwnedObjectPath, OwnedValue};
use gpui::{App, BackgroundExecutor, Task, Window};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    future::{Future, poll_fn},
    pin::{Pin, pin},
    sync::Arc,
    task::Poll,
    time::Duration,
};
use x11rb::{
    connection::Connection as _,
    protocol::xproto::{ConnectionExt, EventMask, UNMAP_NOTIFY_EVENT, UnmapNotifyEvent},
    xcb_ffi::XCBConnection,
};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_PATH: &str = "/StatusNotifierItem";
const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct TrayIcon {
    // 先取消监视，再释放最后的连接；zbus 的 socket/分发任务随 ConnectionInner 释放。
    _monitor: Task<()>,
    locale_update: Option<Task<()>>,
    _connection: Connection,
    events: Arc<TrayEvents>,
    window_id: u32,
}

pub(super) fn unavailable_reason_key(window: &Window) -> Option<&'static str> {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Xcb(_)) => {
            match HasDisplayHandle::display_handle(window).map(|handle| handle.as_raw()) {
                Ok(RawDisplayHandle::Xcb(display)) if display.connection.is_some() => None,
                _ => Some("tray.x11Unavailable"),
            }
        }
        Ok(RawWindowHandle::Wayland(_)) => {
            Some("tray.waylandUnavailable")
        }
        _ => Some("tray.linuxUnavailable"),
    }
}

impl TrayIcon {
    pub(crate) fn start(window: &Window, cx: &App) -> Result<Task<Result<Self, String>>, String> {
        if let Some(reason) = unavailable_reason_key(window) {
            return Err(ToolbarLocale::current(cx).app_text(reason).into());
        }
        let RawWindowHandle::Xcb(handle) = HasWindowHandle::window_handle(window)
            .map_err(error)?
            .as_raw()
        else {
            return Err("窗口不是 X11 窗口".into());
        };
        let window_id = handle.window.get();
        let locale = ToolbarLocale::current(cx);
        let executor = cx.background_executor().clone();
        Ok(cx.background_executor().spawn(async move {
            timed(Self::connect(window_id, locale, executor.clone()), &executor).await
        }))
    }

    async fn connect(window_id: u32, locale: ToolbarLocale, executor: BackgroundExecutor) -> Result<Self, String> {
        let events = Arc::new(TrayEvents::default());
        events.set_locale(locale);
        // 固定内嵌 32px 图标，只有 4096 字节 ARGB；不创建图片 Store 或文档缓存。
        let rgba = image::load_from_memory_with_format(
            include_bytes!("../../../assets/app/32x32.png"),
            image::ImageFormat::Png,
        )
        .map_err(error)?
        .to_rgba8();
        if rgba.dimensions() != (32, 32) {
            return Err("托盘图标尺寸不符合 32px 预算".into());
        }
        let mut pixels = rgba.into_raw();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        let service_name = format!(
            "org.kde.StatusNotifierItem-{}-{window_id}",
            std::process::id()
        );
        let connection = zbus::connection::Builder::session()
            .map_err(error)?
            .name(service_name.as_str())
            .map_err(error)?
            .max_queued(16)
            .method_timeout(TIMEOUT)
            .serve_at(
                ITEM_PATH,
                StatusItem {
                    events: events.clone(),
                    pixels,
                    window_id,
                },
            )
            .map_err(error)?
            .serve_at(menu::PATH, menu::TrayMenu(events.clone()))
            .map_err(error)?
            .build()
            .await
            .map_err(error)?;

        // 在首次检查和注册前订阅，覆盖初始化期间宿主退出／替换的竞争。
        let owner_rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")
            .map_err(error)?
            .interface("org.freedesktop.DBus")
            .map_err(error)?
            .member("NameOwnerChanged")
            .map_err(error)?
            .add_arg(WATCHER)
            .map_err(error)?
            .build();
        let owner_changes = MessageStream::for_match_rule(owner_rule, &connection, Some(8))
            .await
            .map_err(error)?;
        let watcher_rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(WATCHER)
            .map_err(error)?
            .path(WATCHER_PATH)
            .map_err(error)?
            .build();
        let watcher_changes = MessageStream::for_match_rule(watcher_rule, &connection, Some(8))
            .await
            .map_err(error)?;
        require_host(&connection).await?;
        let watcher = Proxy::new(&connection, WATCHER, WATCHER_PATH, WATCHER)
            .await
            .map_err(error)?;
        let _: () = watcher
            .call("RegisterStatusNotifierItem", &(service_name.as_str(),))
            .await
            .map_err(error)?;
        require_host(&connection).await?;
        events.set_online();
        let monitor_connection = connection.clone();
        let monitor_events = events.clone();
        let monitor_executor = executor.clone();
        let monitor = executor.spawn(async move {
            monitor_host(
                monitor_connection,
                owner_changes,
                watcher_changes,
                &monitor_executor,
            )
            .await;
            monitor_events.send(TrayEvent::Unavailable);
        });
        Ok(Self {
            _monitor: monitor,
            locale_update: None,
            _connection: connection,
            events,
            window_id,
        })
    }

    pub(crate) fn set_locale(&mut self, locale: ToolbarLocale, cx: &App) -> Result<(), String> {
        let Some(revision) = self.events.set_locale(locale) else { return Ok(()); };
        let connection = self._connection.clone();
        let executor = cx.background_executor().clone();
        // 每个托盘最多一个有期限的通知；替换语言或释放托盘时取消旧任务。
        self.locale_update = Some(cx.background_executor().spawn(async move {
            let result = timed(async {
                connection.emit_signal(None::<&str>, menu::PATH, "com.canonical.dbusmenu", "LayoutUpdated", &(revision, 0_i32)).await.map_err(error)?;
                connection.emit_signal(None::<&str>, ITEM_PATH, "org.kde.StatusNotifierItem", "NewToolTip", &()).await.map_err(error)
            }, &executor).await;
            if let Err(error) = result { eprintln!("[Tray locale] {error}"); }
        }));
        Ok(())
    }

    pub(crate) fn events(&self) -> Arc<TrayEvents> {
        self.events.clone()
    }

    pub(crate) fn hide_again(
        &self,
        window: &Window,
        _cx: &App,
    ) -> Result<Option<Task<()>>, String> {
        if !self.events.is_online() {
            return Err("桌面托盘宿主已退出；窗口保持打开".into());
        }
        change_visibility(window, self.window_id, false)?;
        Ok(None)
    }
}

// 恢复不依赖仍存活的托盘服务；宿主掉线注销后，外部打开／重新激活仍可找回窗口。
pub(crate) fn restore_window(window: &Window, _cx: &App) -> Result<Option<Task<()>>, String> {
    let RawWindowHandle::Xcb(handle) = HasWindowHandle::window_handle(window)
        .map_err(error)?
        .as_raw()
    else {
        return Err("当前窗口不是 X11 窗口".into());
    };
    change_visibility(window, handle.window.get(), true)?;
    Ok(None)
}

fn error(error: impl std::fmt::Display) -> String {
    format!("托盘操作失败：{error}")
}

async fn timed<T>(
    future: impl Future<Output = Result<T, String>>,
    executor: &BackgroundExecutor,
) -> Result<T, String> {
    let mut future = pin!(future);
    let mut timeout = pin!(executor.timer(TIMEOUT));
    poll_fn(|cx| {
        if let Poll::Ready(result) = future.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        if timeout.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err("桌面托盘响应超时；窗口保持打开".into()));
        }
        Poll::Pending
    })
    .await
}

async fn require_host(connection: &Connection) -> Result<(), String> {
    // 直接读取 Properties.Get，避免以缓存的 true 隐藏窗口。
    let proxy = Proxy::new(
        connection,
        WATCHER,
        WATCHER_PATH,
        "org.freedesktop.DBus.Properties",
    )
    .await
    .map_err(error)?;
    let value: OwnedValue = proxy
        .call("Get", &(WATCHER, "IsStatusNotifierHostRegistered"))
        .await
        .map_err(|_| "当前桌面没有可用的托盘宿主；窗口保持打开".to_owned())?;
    if !bool::try_from(value).map_err(error)? {
        return Err("当前桌面没有可用的托盘宿主；窗口保持打开".into());
    }
    Ok(())
}

async fn monitor_host(
    connection: Connection,
    mut owners: MessageStream,
    mut watcher: MessageStream,
    executor: &BackgroundExecutor,
) {
    let mut closed = pin!(connection.closed());
    loop {
        let message = poll_fn(|cx| {
            if Pin::new(&mut owners).poll_next(cx).is_ready() || closed.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(None);
            }
            Pin::new(&mut watcher).poll_next(cx)
        })
        .await;
        let Some(Ok(message)) = message else {
            break;
        };
        let header = message.header();
        match header.member().map(|member| member.as_str()) {
            Some("StatusNotifierHostUnregistered") => break,
            Some("PropertiesChanged") => {
                if timed(require_host(&connection), executor).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}

fn change_visibility(window: &Window, expected: u32, show: bool) -> Result<(), String> {
    let RawWindowHandle::Xcb(handle) = HasWindowHandle::window_handle(window)
        .map_err(error)?
        .as_raw()
    else {
        return Err("当前窗口不是 X11 窗口".into());
    };
    let RawDisplayHandle::Xcb(display) = HasDisplayHandle::display_handle(window)
        .map_err(error)?
        .as_raw()
    else {
        return Err("当前窗口没有 X11 连接".into());
    };
    if handle.window.get() != expected {
        return Err("托盘窗口身份已失效".into());
    }
    let pointer = display.connection.ok_or("当前 X11 连接已失效")?;
    // 借用 GPUI 的同一 XCB 连接，不关闭它、不读取事件；Window 借用保证连接仍存活。
    let connection = unsafe { XCBConnection::from_raw_xcb_connection(pointer.as_ptr(), false) }
        .map_err(error)?;
    if show {
        connection
            .map_window(expected)
            .map_err(error)?
            .check()
            .map_err(error)?;
        connection.flush().map_err(error)?;
        return Ok(());
    }
    let root = connection
        .setup()
        .roots
        .get(display.screen as usize)
        .ok_or("X11 屏幕已失效")?
        .root;
    let result = (|| {
        connection
            .unmap_window(expected)
            .map_err(error)?
            .check()
            .map_err(error)?;
        // ICCCM 的 WithdrawWindow：兼容有重定父窗口的 WM，同时保留原窗口及其属性。
        let event = UnmapNotifyEvent {
            response_type: UNMAP_NOTIFY_EVENT,
            sequence: 0,
            event: root,
            window: expected,
            from_configure: false,
        };
        connection
            .send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
        connection.flush().map_err(error)
    })();
    if let Err(failure) = result {
        // 任何后续请求失败都尝试回滚已执行的 unmap，不能报告失败却留下隐藏窗口。
        let rollback = connection
            .map_window(expected)
            .map_err(error)
            .and_then(|cookie| cookie.check().map_err(error))
            .and_then(|()| connection.flush().map_err(error));
        return Err(match rollback {
            Ok(()) => failure,
            Err(restore) => format!("{failure}；恢复窗口失败：{restore}"),
        });
    }
    Ok(())
}

struct StatusItem {
    events: Arc<TrayEvents>,
    pixels: Vec<u8>,
    window_id: u32,
}

// 同一连接串行处理固定菜单／属性，避免每次点击创建后台任务。
#[zbus::interface(
    name = "org.kde.StatusNotifierItem",
    crate = "ashpd::zbus",
    spawn = false
)]
impl StatusItem {
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> String {
        format!("nomo-{}", self.window_id)
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Nomo"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        self.window_id
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(32, 32, self.pixels.clone())]
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (&str, Vec<(i32, i32, Vec<u8>)>, &str, &str) {
        ("", Vec::new(), "Nomo", self.events.locale().app_text("tray.menuTip"))
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(menu::PATH).expect("static menu path")
    }
    fn activate(&self, _x: i32, _y: i32) {
        self.events.send(TrayEvent::Restore);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.events.send(TrayEvent::Restore);
    }
    // 支持 dbusmenu 的宿主按 Menu 属性显示系统菜单，不在应用内伪造屏幕坐标菜单。
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
}
