//! 重复点击合并为三个标志；原生／D-Bus 回调不持有 Workspace 或窗口引用。
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use tokio::sync::Notify;

#[derive(Clone, Copy)]
pub(crate) enum TrayEvent {
    Restore = 1,
    Quit = 2,
    Unavailable = 4,
}

#[derive(Default)]
pub(crate) struct TrayEvents {
    pending: AtomicU8,
    locale: AtomicU8,
    menu_revision: AtomicU32,
    online: AtomicBool,
    notify: Notify,
}

impl TrayEvents {
    pub(super) fn set_locale(&self, locale: crate::ui::toolbar_i18n::ToolbarLocale) -> Option<u32> {
        (self.locale.swap(locale.id(), Ordering::AcqRel) != locale.id())
            .then(|| self.menu_revision.fetch_add(1, Ordering::AcqRel).wrapping_add(2))
    }
    pub(super) fn locale(&self) -> crate::ui::toolbar_i18n::ToolbarLocale {
        crate::ui::toolbar_i18n::ToolbarLocale::from_id(self.locale.load(Ordering::Acquire))
    }
    pub(super) fn menu_revision(&self) -> u32 { self.menu_revision.load(Ordering::Acquire).wrapping_add(1) }

    pub(super) fn set_online(&self) {
        self.online.store(true, Ordering::Release);
    }

    pub(super) fn is_online(&self) -> bool {
        self.online.load(Ordering::Acquire)
    }

    pub(super) fn send(&self, event: TrayEvent) {
        if matches!(event, TrayEvent::Unavailable) {
            self.online.store(false, Ordering::Release);
        }
        self.pending.fetch_or(event as u8, Ordering::AcqRel);
        self.notify.notify_one();
    }

    pub(crate) async fn next(&self) -> TrayEvent {
        loop {
            let pending = self.pending.load(Ordering::Acquire);
            // 退出本身也先恢复窗口；不得被掉线或随后到来的重复“打开”覆盖。
            let event = if pending & TrayEvent::Quit as u8 != 0 {
                Some(TrayEvent::Quit)
            } else if pending & TrayEvent::Unavailable as u8 != 0 {
                Some(TrayEvent::Unavailable)
            } else if pending & TrayEvent::Restore as u8 != 0 {
                Some(TrayEvent::Restore)
            } else {
                None
            };
            if let Some(event) = event {
                if self
                    .pending
                    .compare_exchange(
                        pending,
                        pending & !(event as u8),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return event;
                }
            } else {
                // Notify 保留一个许可，覆盖检查标志与等待之间的并发通知。
                self.notify.notified().await;
            }
        }
    }
}
