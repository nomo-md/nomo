//! 小窗仅改变当前文档的呈现；会话、保存、历史和后台标签保持原有归属。
use super::*;
use crate::ui::mini::{MiniBar, MiniEvent, MiniGeometry, MiniTitle, MiniWindow};

pub(super) struct MiniState {
    session: Option<MiniSession>,
    geometry: Option<MiniGeometry>,
    pinned: bool,
}

struct MiniSession {
    window: MiniWindow,
    previous_mode: EditorMode,
    bar: Entity<MiniBar>,
    _subscription: Subscription,
}

impl Default for MiniState {
    fn default() -> Self { Self { session: None, geometry: None, pinned: true } }
}

impl MiniState {
    pub(super) fn active(&self) -> bool { self.session.is_some() }
}

impl Workspace {
    fn mini_title(&self, cx: &App) -> Option<MiniTitle> {
        let document = self.document()?;
        let host = document.read(cx);
        Some(MiniTitle { name: host.title().into(), dirty: host.is_dirty(cx),
            external: self.external_save_blocked(&document, cx), pinned: self.mini.pinned })
    }

    pub(super) fn toggle_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() || self.closing_window { return; }
        if self.mini.active() {
            if self.leave_mini(window, cx) {
                if let Some(document) = self.document() { self.resolve_external_before_save(&document, window, cx); }
            }
            return;
        }
        if !cfg!(windows) || window.is_fullscreen() {
            self.notice = Some(if cfg!(windows) { "请先退出全屏，再打开 Markdown Mini" } else { "当前平台尚未接入 Markdown Mini" }.into());
            cx.notify(); return;
        }
        if !self.opening.is_empty() {
            self.notice = Some("文档仍在打开，请稍后切换小窗".into()); cx.notify(); return;
        }
        let Some(document) = self.document() else { return; };
        if let Err(error) = document.read(cx).ready(cx) { self.notice = Some(error.to_string().into()); cx.notify(); return; }
        let previous_mode = self.tabs[self.active].mode;
        // 同一轮更新里切换布局和原生尺寸；不经过 Settings::set，避免全局设置与磁盘写入。
        let layout = document.update(cx, |host, cx| {
            if previous_mode != EditorMode::Semantic { host.change_mode(EditorMode::Semantic, cx)?; }
            host.set_mini_layout(true, cx);
            Ok::<_, ControlError>(())
        });
        if let Err(error) = layout { self.notice = Some(error.to_string().into()); cx.notify(); return; }
        let native = match MiniWindow::enter(window, self.mini.geometry, self.mini.pinned) {
            Ok(native) => native,
            Err(error) => {
                window.bounds_changed(cx);
                let rollback = document.update(cx, |host, cx| {
                    host.set_mini_layout(false, cx);
                    if previous_mode != EditorMode::Semantic { host.change_mode(previous_mode, cx)?; }
                    Ok::<_, ControlError>(())
                });
                self.failure_notice = Some(match rollback { Ok(()) => error, Err(restore) => format!("{error}；恢复编辑模式失败：{restore}") }.into());
                cx.notify(); return;
            }
        };
        // Win32 在 SetWindowPos 内同步派发尺寸事件；此时 GPUI 正借出当前 Window，
        // 回调无法再次 update 它。原生操作结束后同步视口，防止正文和命中区沿用旧宽度。
        window.bounds_changed(cx);
        self.tabs[self.active].mode = EditorMode::Semantic;
        self.pending_tree_click.take();
        self.popover = None;
        self.toolbar_popup = None;
        self.toolbar_motion_task.take();
        self.toolbar_motion = None;
        self.tab_menu_open = false;
        self.search = None;
        self.close_status(cx);
        self.outline_ui.suspend_for_mini();
        self.suspend_tree(cx);
        self.surface.clear();
        let title = self.mini_title(cx).expect("active Mini document");
        let bar = cx.new(|cx| MiniBar::new(title, window, cx));
        let subscription = cx.subscribe_in(&bar, window, |workspace, _, event, window, cx| match event {
            MiniEvent::Return => workspace.toggle_mini(window, cx),
            MiniEvent::Pin => {
                let Some(session) = &workspace.mini.session else { return; };
                let next = !workspace.mini.pinned;
                match session.window.set_pinned(next) {
                    Ok(()) => workspace.mini.pinned = next,
                    Err(error) => workspace.failure_notice = Some(error.into()),
                }
                cx.notify();
            }
        });
        self.mini.session = Some(MiniSession { window: native, previous_mode, bar, _subscription: subscription });
        window.focus(&document.focus_handle(cx), cx);
        cx.notify();
    }

    /// 其他文档/主页/关闭入口先返回正常窗口；失败时保留当前会话，让用户可以重试。
    pub(super) fn leave_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.mini.session.as_ref() else { return true; };
        let Some(document) = self.document() else { return false; };
        let previous_mode = session.previous_mode;
        if let Err(error) = document.read(cx).ready(cx) { self.notice = Some(error.to_string().into()); cx.notify(); return false; }
        let result = document.update(cx, |host, cx| {
            if previous_mode != EditorMode::Semantic { host.change_mode(previous_mode, cx)?; }
            host.set_mini_layout(false, cx);
            Ok::<_, ControlError>(())
        });
        if let Err(error) = result { self.failure_notice = Some(error.to_string().into()); cx.notify(); return false; }
        let session = self.mini.session.as_mut().expect("Mini session");
        let geometry = session.window.geometry();
        if let Err(error) = session.window.restore() {
            window.bounds_changed(cx);
            let rollback = document.update(cx, |host, cx| {
                if previous_mode != EditorMode::Semantic { host.change_mode(EditorMode::Semantic, cx)?; }
                host.set_mini_layout(true, cx);
                Ok::<_, ControlError>(())
            });
            self.failure_notice = Some(match rollback { Ok(()) => error, Err(restore) => format!("{error}；恢复小窗编辑模式失败：{restore}") }.into());
            cx.notify(); return false;
        }
        window.bounds_changed(cx);
        self.mini.geometry = geometry;
        self.mini.session = None;
        self.tabs[self.active].mode = previous_mode;
        self.resume_tree(cx);
        window.focus(&document.focus_handle(cx), cx);
        cx.notify();
        true
    }

    pub(super) fn render_mini_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.mini.session.as_ref()?;
        if let Some(title) = self.mini_title(cx) { session.bar.update(cx, |bar, cx| bar.set_title(title, cx)); }
        Some(gpui::deferred(session.bar.clone()).with_priority(2).into_any_element())
    }
}
