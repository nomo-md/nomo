//! 状态栏随工作区释放；窗口引用仅在可取消的主线程操作内暂存，不隐藏整个应用。
use super::events::{TrayEvent, TrayEvents};
use crate::ui::toolbar_i18n::ToolbarLocale;
use gpui::{App, Task, Window};
use objc2::rc::Retained;
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSImage, NSMenu, NSMenuItem, NSSquareStatusItemLength, NSStatusBar,
    NSStatusItem, NSView, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Arc;

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "NomoTrayActions"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Arc<TrayEvents>]
    struct TrayActions;

    unsafe impl NSObjectProtocol for TrayActions {}

    impl TrayActions {
        #[unsafe(method(restoreWindow:))]
        fn restore_window(&self, _sender: Option<&NSObject>) {
            self.ivars().send(TrayEvent::Restore);
        }

        #[unsafe(method(quitApplication:))]
        fn quit_application(&self, _sender: Option<&NSObject>) {
            self.ivars().send(TrayEvent::Quit);
        }
    }
);

pub(crate) struct TrayIcon {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    // NSMenuItem 不 retain target；必须比菜单回调活得更久。
    _actions: Retained<TrayActions>,
    events: Arc<TrayEvents>,
}

pub(super) fn unavailable_reason_key(window: &Window) -> Option<&'static str> {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::AppKit(_)) => None,
        _ => Some("tray.macosUnavailable"),
    }
}

fn native_window(window: &Window) -> Result<Retained<NSWindow>, String> {
    MainThreadMarker::new().ok_or("托盘窗口操作必须在主线程执行")?;
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)
        .map_err(|error| error.to_string())?
        .as_raw()
    else {
        return Err("当前窗口没有可用的 macOS 托盘接口".into());
    };
    // GPUI 保证借用 Window 期间该 NSView 存活；不把裸指针保存到托盘或回调中。
    let view = unsafe { &*handle.ns_view.cast::<NSView>().as_ptr() };
    view.window()
        .ok_or_else(|| "无法取得当前 macOS 窗口".into())
}

impl TrayIcon {
    pub(crate) fn start(window: &Window, cx: &App) -> Result<Task<Result<Self, String>>, String> {
        let _ = native_window(window)?;
        Ok(Task::ready(Self::create(ToolbarLocale::current(cx))))
    }

    fn create(locale: ToolbarLocale) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("托盘必须在主线程创建")?;
        let events = Arc::new(TrayEvents::default());
        let actions = TrayActions::alloc(mtm).set_ivars(events.clone());
        let actions: Retained<TrayActions> = unsafe { msg_send![super(actions), init] };
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        for (label, action) in [
            (locale.app_text("tray.open"), sel!(restoreWindow:)),
            (locale.app_text("tray.quit"), sel!(quitApplication:)),
        ] {
            // selector 与 target 均由本模块固定；target 由 TrayIcon 保持到菜单解除之后。
            let entry = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(label),
                    Some(action),
                    &NSString::from_str(""),
                )
            };
            unsafe {
                entry.setTarget(Some(&actions));
            }
            menu.addItem(&entry);
        }
        let data = NSData::with_bytes(include_bytes!("../../../assets/app/32x32.png"));
        let image =
            NSImage::initWithData(NSImage::alloc(), &data).ok_or("无法读取 Nomo 托盘图标")?;
        image.setSize(NSSize::new(18.0, 18.0));
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
        let Some(button) = item.button(mtm) else {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
            return Err("无法创建 macOS 状态栏按钮；窗口保持打开".into());
        };
        button.setImage(Some(&image));
        button.setToolTip(Some(&NSString::from_str("Nomo")));
        item.setMenu(Some(&menu));
        events.set_online();
        Ok(Self {
            item,
            menu,
            _actions: actions,
            events,
        })
    }

    pub(crate) fn set_locale(&mut self, locale: ToolbarLocale, _: &App) -> Result<(), String> {
        for (index, key) in ["tray.open", "tray.quit"].into_iter().enumerate() {
            if let Some(item) = self.menu.itemAtIndex(index as isize) { item.setTitle(&NSString::from_str(locale.app_text(key))); }
        }
        Ok(())
    }

    pub(crate) fn events(&self) -> Arc<TrayEvents> {
        self.events.clone()
    }

    pub(crate) fn hide_again(&self, window: &Window, cx: &App) -> Result<Option<Task<()>>, String> {
        if !self.events.is_online() {
            return Err("托盘不可用；窗口保持打开".into());
        }
        let native = native_window(window)?;
        // orderOut 也可能让其他窗口成为 key window，同步触发 GPUI 绘制回调。
        // 任务只保留到下一次主线程调度；工作区在恢复／关闭时取消尚未执行的操作。
        Ok(Some(cx.foreground_executor().spawn(async move {
            native.orderOut(None);
        })))
    }
}

pub(crate) fn restore_window(window: &Window, cx: &App) -> Result<Option<Task<()>>, String> {
    let mtm = MainThreadMarker::new().ok_or("托盘窗口操作必须在主线程执行")?;
    let native = native_window(window)?;
    // AppKit 的 key-window 通知会同步请求绘制，不能在 Workspace 更新借用中调用。
    Ok(Some(cx.foreground_executor().spawn(async move {
        if native.isMiniaturized() {
            native.deminiaturize(None);
        }
        if let Some(view) = native.contentView() {
            view.setNeedsDisplay(true);
        }
        native.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    })))
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.item.setMenu(None);
        self.menu.removeAllItems();
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}
