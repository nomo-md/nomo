//! 小窗仅改变当前文档的呈现；会话、保存、历史和后台标签保持原有归属。
use super::*;
use crate::ui::mini::{MiniBar, MiniEvent, MiniFrame, MiniGeometry, MiniPin, MiniTarget, MiniTitle, MiniWindow};

pub(super) struct MiniState {
    session: Option<MiniSession>,
    pub(super) main_window_placement: Option<WindowPlacement>,
    geometry: Option<MiniGeometry>,
    pinned: bool,
    frame_pending: bool,
    // 返回主界面后仍可确认原生恢复，不持有旧 DocumentHost 或控制条。
    restoring: Option<Box<MiniWindow>>,
    #[cfg(target_os = "linux")]
    confirmation_task: Option<Task<()>>,
    #[cfg(target_os = "macos")]
    native_task: Option<Task<()>>,
    #[cfg(target_os = "macos")]
    native_task_epoch: u64,
}

struct MiniSession {
    window: MiniWindow,
    previous_mode: EditorMode,
    paper_mini: bool,
    main_paper: gpui::Bounds<gpui::Pixels>,
    main_viewport: gpui::Size<gpui::Pixels>,
    bar: Entity<MiniBar>,
    _subscription: Subscription,
}

struct PaperFrame {
    progress: f32,
    insets: gpui::Edges<gpui::Pixels>,
    opacity: f32,
    source_size: Option<gpui::Size<gpui::Pixels>>,
}

#[derive(Clone, Copy)]
pub(super) struct ShellFrame {
    pub visibility: f32,
    pub sidebar_width: gpui::Pixels,
    pub toolbar_height: gpui::Pixels,
    pub mode: EditorMode,
}

/// 外壳占位和内容共用窗框进度；内部保持原尺寸，避免文字被挤压后在末帧重新排版。
pub(super) fn shell_row(content: AnyElement, height: gpui::Pixels, visibility: f32) -> AnyElement {
    div().relative().w_full().h(height * visibility).flex_none()
        .when(visibility < 1.0, |row| row.overflow_hidden())
        .child(div().relative().w_full().h(height).top(-height * (1.0 - visibility)).opacity(visibility).child(content))
        .into_any_element()
}

/// 平移已有轮廓，使固定高度的标签与收放中的标签栏对齐，复用原有阴影缓存。
pub(super) fn surface_layer(content: impl IntoElement, visibility: f32) -> AnyElement {
    div().absolute().left_0().right_0().bottom_0().top(px(-TAB_RAIL_HEIGHT * (1.0 - visibility)))
        .opacity(visibility).child(content).into_any_element()
}

impl MiniSession {
    fn paper_frame(&self) -> Option<PaperFrame> {
        let progress = self.window.progress()?;
        let remaining = 1.0 - progress;
        let inset = |value: gpui::Pixels| value.max(px(0.0)) * remaining;
        let insets = gpui::Edges { left: inset(self.main_paper.left()), top: inset(self.main_paper.top()),
            right: inset(self.main_viewport.width - self.main_paper.right()),
            bottom: inset(self.main_viewport.height - self.main_paper.bottom()) };
        // 单栏语义正文全程可读。只有两端表示法不同时才在交接处淡出、换版、淡入。
        let changing_mode = self.previous_mode != EditorMode::Semantic;
        let opacity = if changing_mode {
            let t = ((progress - 0.5).abs() * 3.0).min(1.0);
            t * t * (3.0 - 2.0 * t)
        } else { 1.0 };
        let source_size = (changing_mode && !self.paper_mini).then_some(self.main_paper.size);
        Some(PaperFrame { progress, insets, opacity, source_size })
    }
}

impl Default for MiniState {
    fn default() -> Self { Self { session: None, main_window_placement: None, geometry: None, pinned: true, frame_pending: false,
        restoring: None, #[cfg(target_os = "linux")] confirmation_task: None,
        #[cfg(target_os = "macos")] native_task: None,
        #[cfg(target_os = "macos")] native_task_epoch: 0 } }
}

impl MiniState {
    pub(super) fn active(&self) -> bool { self.session.is_some() }
    pub(super) fn has_native_bounds(&self) -> bool { self.active() || self.restoring.is_some() }
    pub(super) fn shell_frame(&self) -> Option<ShellFrame> {
        let session = self.session.as_ref()?;
        Some(ShellFrame {
            visibility: 1.0 - session.window.progress()?,
            sidebar_width: session.main_paper.left().max(px(0.0)),
            toolbar_height: (session.main_paper.top() - px(crate::ui::chrome::TITLEBAR_HEIGHT + TAB_RAIL_HEIGHT)).max(px(0.0)),
            mode: session.previous_mode,
        })
    }

    pub(super) fn render_paper(&self, main: AnyElement, theme: &Theme) -> AnyElement {
        let frame = self.session.as_ref().and_then(MiniSession::paper_frame);
        let paper = div().flex_none().size_full()
            .when_some(frame.as_ref().and_then(|f| f.source_size), |paper, size| paper.w(size.width).h(size.height))
            .child(main);
        let bounds = div().absolute().inset_0().flex().items_center().justify_center().overflow_hidden()
            .when_some(frame, |panel, f| panel.opacity(f.opacity));
        // 编辑区始终挂在同一外壳布局里；文件树与各顶栏的占位已经给出了本帧边界。
        div().relative().size_full().overflow_hidden().bg(theme.surface).child(bounds.child(paper)).into_any_element()
    }
}

fn set_document_layout(document: &Entity<DocumentHost>, previous: EditorMode, mini: bool, cx: &mut Context<Workspace>) -> Result<(), ControlError> {
    document.update(cx, |host, cx| {
        if previous != EditorMode::Semantic { host.change_mode(if mini { EditorMode::Semantic } else { previous }, cx)?; }
        host.set_mini_layout(mini, cx);
        Ok(())
    })
}

fn mini_failure(error: &crate::ui::toolbar_i18n::UiMessage, cx: &App) -> SharedString {
    let locale = ToolbarLocale::current(cx);
    locale.app_format("mini.operationFailed", &[("error", &error.render(locale))]).into()
}

impl Workspace {
    fn mini_title(&self, cx: &App) -> Option<MiniTitle> {
        let document = self.document()?;
        let host = document.read(cx);
        Some(MiniTitle { name: host.display_title(cx).into(), dirty: host.is_dirty(cx),
            external: self.external_save_blocked(&document, cx),
            pin: self.mini.session.as_ref().map_or(MiniPin::Ready(self.mini.pinned), |session| session.window.pin_state()) })
    }

    pub(super) fn toggle_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_pair().is_some() { return; }
        if window.has_active_prompt() || self.closing_window { return; }
        if let Some(restoring) = &mut self.mini.restoring {
            match restoring.restore(window) {
                Ok(true) => self.mini.restoring = None,
                Ok(false) => {
                    self.notice = Some(ToolbarLocale::current(cx).app_text("mini.restoring").into());
                    self.schedule_mini_frame(window, cx); cx.notify(); return;
                }
                Err(error) => { self.failure_notice = Some(mini_failure(&error, cx)); cx.notify(); return; }
            }
        }
        if let Some(session) = &self.mini.session {
            let target = if session.window.moving_to() == Some(MiniTarget::Main) { MiniTarget::Mini } else { MiniTarget::Main };
            self.switch_mini_target(target, window, cx);
            return;
        }
        if let Some(reason) = crate::ui::mini::unavailable_reason_key(window) {
            self.notice = Some(ToolbarLocale::current(cx).app_text(reason).into());
            cx.notify(); return;
        }
        if !self.opening.is_empty() {
            self.notice = Some(ToolbarLocale::current(cx).app_text("mini.opening").into()); cx.notify(); return;
        }
        let Some(document) = self.document() else { return; };
        if let Err(error) = document.read(cx).ready(cx) { self.notice = Some(ToolbarLocale::current(cx).error(&error).into()); cx.notify(); return; }
        let previous_mode = self.tabs[self.active].mode;
        let main_viewport = window.viewport_size();
        let main_paper = self.outline_ui.bounds.get().filter(|bounds| !bounds.is_empty())
            .unwrap_or_else(|| gpui::Bounds::new(gpui::Point::default(), main_viewport));
        let animate = !cx.global::<MotionPreference>().reduced;
        self.mini.main_window_placement = self.window_placement(window, cx);
        let mut native = match MiniWindow::enter(window, self.mini.geometry, self.mini.pinned, animate) {
            Ok(native) => native,
            Err(error) => {
                window.bounds_changed(cx);
                self.failure_notice = Some(mini_failure(&error, cx));
                cx.notify(); return;
            }
        };
        let paper_mini = previous_mode == EditorMode::Semantic || native.progress().is_none();
        if paper_mini && let Err(error) = set_document_layout(&document, previous_mode, true, cx) {
            let rollback = native.restore(window);
            window.bounds_changed(cx);
            self.failure_notice = Some(match &rollback { Ok(_) => ToolbarLocale::current(cx).error(&error), Err(restore) => ToolbarLocale::current(cx).app_format("mini.mainRollbackFailed", &[("error", &ToolbarLocale::current(cx).error(&error)), ("restore", &restore.render(ToolbarLocale::current(cx)))]) }.into());
            #[cfg(windows)]
            let retain_restore = matches!(rollback, Ok(false));
            #[cfg(not(windows))]
            let retain_restore = !matches!(rollback, Ok(true));
            if retain_restore { self.mini.restoring = Some(Box::new(native)); self.schedule_mini_frame(window, cx); }
            cx.notify(); return;
        }
        // Win32 在 SetWindowPos 内同步派发尺寸事件；此时 GPUI 正借出当前 Window，
        // 回调无法再次 update 它。原生操作结束后同步视口，防止正文和命中区沿用旧宽度。
        window.bounds_changed(cx);
        if paper_mini { self.tabs[self.active].mode = EditorMode::Semantic; }
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
        self.explorer.sidebar_motion = None;
        let mut title = self.mini_title(cx).expect("active Mini document");
        title.pin = native.pin_state();
        if let Some(notice) = native.take_notice(ToolbarLocale::current(cx)) { self.notice = Some(notice.into()); }
        let bar = cx.new(|cx| MiniBar::new(title, window, cx));
        let subscription = cx.subscribe_in(&bar, window, |workspace, _, event, window, cx| match event {
            MiniEvent::Return => workspace.return_from_mini(window, cx),
            MiniEvent::Pin => {
                let Some(session) = &mut workspace.mini.session else { return; };
                let MiniPin::Ready(pinned) = session.window.pin_state() else { return; };
                match session.window.set_pinned(!pinned, window) {
                    Ok(()) => {
                        if let MiniPin::Ready(pinned) = session.window.pin_state() { workspace.mini.pinned = pinned; }
                    }
                    Err(error) => workspace.failure_notice = Some(mini_failure(&error, cx)),
                }
                workspace.schedule_mini_frame(window, cx);
                cx.notify();
            }
        });
        self.mini.session = Some(MiniSession { window: native, previous_mode, paper_mini, main_paper, main_viewport, bar, _subscription: subscription });
        self.update_paper_frame(window, cx);
        window.focus(&document.focus_handle(cx), cx);
        self.schedule_mini_frame(window, cx);
        cx.notify();
    }

    fn update_paper_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        let Some(session) = &self.mini.session else { return; };
        let frame = session.paper_frame().and_then(|frame| {
            if !session.paper_mini { return None; }
            let width = (window.viewport_size().width - frame.insets.left - frame.insets.right).max(px(1.0));
            let progress = if session.previous_mode == EditorMode::Semantic { frame.progress } else { 1.0 };
            Some((width, progress))
        });
        document.update(cx, |host, cx| host.set_mini_paper_frame(frame, cx));
    }

    fn sync_paper_layout(&mut self, window: &Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        let Some(document) = self.document() else { return Ok(()); };
        let Some(session) = &mut self.mini.session else { return Ok(()); };
        let mini = session.previous_mode == EditorMode::Semantic || session.window.progress().is_none_or(|t| {
            t > 0.5 || (t == 0.5 && session.window.moving_to() == Some(MiniTarget::Mini))
        });
        if session.paper_mini != mini {
            set_document_layout(&document, session.previous_mode, mini, cx)?;
            session.paper_mini = mini;
            self.tabs[self.active].mode = if mini { EditorMode::Semantic } else { session.previous_mode };
            // 重建双栏可能占用一帧。把剩余时间留给新正文的淡入。
            session.window.resume_after_handoff();
        }
        self.update_paper_frame(window, cx);
        Ok(())
    }

    pub(super) fn return_from_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_mini_target(MiniTarget::Main, window, cx);
    }

    fn switch_mini_target(&mut self, target: MiniTarget, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() || self.closing_window { return; }
        if target == MiniTarget::Main && let Some(document) = self.document() && let Err(error) = document.read(cx).ready(cx) {
            self.notice = Some(ToolbarLocale::current(cx).error(&error).into()); cx.notify(); return;
        }
        let Some(session) = &mut self.mini.session else { return; };
        match session.window.transition_to(target, !cx.global::<MotionPreference>().reduced, window) {
            Ok(true) => {
                self.update_paper_frame(window, cx);
                self.schedule_mini_frame(window, cx);
            }
            Ok(false) if target == MiniTarget::Main => self.finish_mini_return(window, cx),
            Ok(false) => {
                window.bounds_changed(cx);
                self.present_mini_frame(window, cx);
            }
            Err(error) => {
                window.bounds_changed(cx);
                self.failure_notice = Some(mini_failure(&error, cx));
                if target == MiniTarget::Main { self.finish_mini_return(window, cx); }
            }
        }
        cx.notify();
    }

    fn schedule_mini_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.mini.restoring.as_deref().or_else(|| self.mini.session.as_ref().map(|s| &s.window))
            .is_some_and(|native| native.moving_to().is_some());
        #[cfg(target_os = "macos")]
        if !pending {
            self.mini.native_task_epoch = self.mini.native_task_epoch.wrapping_add(1);
            self.mini.native_task.take();
            self.mini.frame_pending = false;
        }
        if !pending { return; }
        #[cfg(not(target_os = "macos"))]
        if self.mini.frame_pending { return; }
        #[cfg(target_os = "macos")]
        self.mini.native_task.take();
        self.mini.frame_pending = true;
        #[cfg(target_os = "linux")]
        {
            // 每次仅一个可取消任务；只确认当前快照，未完成请求在平台层有总时限。
            self.mini.confirmation_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(40)).await;
                let _ = this.update_in(cx, |workspace, window, cx| workspace.advance_mini_frame(window, cx));
            }));
        }
        #[cfg(target_os = "macos")]
        {
            self.mini.native_task_epoch = self.mini.native_task_epoch.wrapping_add(1);
            let epoch = self.mini.native_task_epoch;
            // prepare / complete 的 update_in 只读原生属性、更新 Rust 状态。
            // run 在两次借用之间执行，AppKit 的同步 resize / display 回调可安全进入 GPUI。
            self.mini.native_task = Some(cx.spawn_in(window, async move |this, cx| {
                let operation = this.update_in(cx, |workspace, window, cx| {
                    if workspace.mini.native_task_epoch != epoch { return None; }
                    let operation = workspace.mini.restoring.as_deref()
                        .or_else(|| workspace.mini.session.as_ref().map(|s| &s.window))
                        .and_then(|native| native.prepare_native_operation(window));
                    if operation.is_none() {
                        workspace.mini.native_task.take();
                        workspace.advance_mini_frame(window, cx);
                    }
                    operation
                }).ok().flatten();
                let Some(operation) = operation else { return; };
                operation.run();
                let _ = this.update_in(cx, |workspace, window, cx| {
                    // 原生回调可能已关闭旧会话并创建新请求；旧任务不能取消新任务。
                    if workspace.mini.native_task_epoch != epoch { return; }
                    workspace.mini.native_task.take();
                    workspace.mini.frame_pending = false;
                    let current = workspace.mini.restoring.as_deref()
                        .or_else(|| workspace.mini.session.as_ref().map(|s| &s.window))
                        .is_some_and(|native| native.accepts_native_operation(&operation, window));
                    if current { workspace.advance_mini_frame(window, cx); }
                    else { workspace.schedule_mini_frame(window, cx); }
                });
            }));
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
        // 只排一帧；回调不持有 HWND、会话或视图，退出或折返后仅查看当前状态。
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |workspace, cx| workspace.advance_mini_frame(window, cx));
        });
        }
    }

    fn advance_mini_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mini.frame_pending = false;
        #[cfg(target_os = "linux")]
        self.mini.confirmation_task.take();
        if let Some(restoring) = &mut self.mini.restoring {
            let result = restoring.advance(true, false, window);
            let pending_only = matches!(&result, Ok((MiniFrame::More, false)));
            if let Some(notice) = restoring.take_notice(ToolbarLocale::current(cx)) { self.notice = Some(notice.into()); }
            match result {
                Ok((MiniFrame::More, _)) => self.schedule_mini_frame(window, cx),
                Ok((MiniFrame::Settled(MiniTarget::Main), _)) => { self.mini.restoring = None; }
                Ok((MiniFrame::Settled(MiniTarget::Mini), _)) => {
                    self.failure_notice = Some(ToolbarLocale::current(cx).app_text("mini.restorePending").into());
                }
                Err(error) => { self.failure_notice = Some(mini_failure(&error, cx)); }
            }
            if !pending_only { window.bounds_changed(cx); cx.notify(); }
            return;
        }
        let Some(session) = &mut self.mini.session else { return; };
        let Some(target) = session.window.moving_to() else { return; };
        let handoff = session.previous_mode != EditorMode::Semantic;
        let result = session.window.advance(cx.global::<MotionPreference>().reduced, handoff, window);
        if target == MiniTarget::Mini && (!cfg!(target_os = "macos") || result.is_ok())
            && let MiniPin::Ready(pinned) = session.window.pin_state() { self.mini.pinned = pinned; }
        if let Some(notice) = session.window.take_notice(ToolbarLocale::current(cx)) { self.notice = Some(notice.into()); }
        match result {
            Ok((frame, changed)) => {
                if changed { window.bounds_changed(cx); }
                match frame {
                    MiniFrame::More => {
                        #[cfg(target_os = "linux")]
                        self.schedule_mini_frame(window, cx);
                        #[cfg(not(target_os = "linux"))]
                        if self.present_mini_frame(window, cx) { self.schedule_mini_frame(window, cx); }
                    }
                    MiniFrame::Settled(MiniTarget::Main) => self.finish_mini_return(window, cx),
                    MiniFrame::Settled(MiniTarget::Mini) => { self.present_mini_frame(window, cx); }
                }
            }
            Err(error) => {
                self.failure_notice = Some(mini_failure(&error, cx));
                #[cfg(windows)]
                {
                if target == MiniTarget::Main { self.finish_mini_return(window, cx); }
                else { self.recover_mini_bounds(window, cx); }
                }
                #[cfg(not(windows))]
                { let _ = target; window.bounds_changed(cx); }
                #[cfg(target_os = "macos")]
                if self.mini.session.as_ref().is_some_and(|session| session.window.native_is_restored()) {
                    // 进入失败且原生回滚成功时，也退回原文档布局；失败快照仍由恢复路径保留。
                    self.leave_mini(window, cx);
                }
                cx.notify();
            }
        }
    }

    fn present_mini_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Err(error) = self.sync_paper_layout(window, cx) {
            self.failure_notice = Some(ToolbarLocale::current(cx).error(&error).into());
            // 换版被输入事务拒绝时，退回仍能编辑的那一侧，不逐帧重试。
            if self.mini.session.as_ref().is_some_and(|session| session.paper_mini) {
                self.recover_mini_bounds(window, cx);
            } else { self.leave_mini(window, cx); }
            cx.notify();
            return false;
        }
        // 窗框舍入后可能仍在同一像素，纸面的位置和透明度仍须绘制。
        cx.notify();
        true
    }

    fn finish_mini_return(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() {
            if self.mini.session.as_ref().is_some_and(|s| s.paper_mini) { self.recover_mini_bounds(window, cx); }
            else { self.leave_mini(window, cx); }
            return;
        }
        if self.leave_mini(window, cx) {
            if let Some(document) = self.document() { self.resolve_external_before_save(&document, window, cx); }
        } else { self.recover_mini_bounds(window, cx); }
    }

    fn recover_mini_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &mut self.mini.session else { return; };
        if (session.window.moving_to().is_some() || cfg!(target_os = "macos")) && let Err(error) = session.window.settle_mini(window) {
            let reason = self.failure_notice.take().or_else(|| self.notice.take());
            let error = error.render(ToolbarLocale::current(cx));
            self.failure_notice = Some(reason.map_or_else(|| error.clone(), |reason| ToolbarLocale::current(cx).app_format("mini.additionalFailure", &[("reason", &reason), ("error", &error)])).into());
        }
        window.bounds_changed(cx);
        if let Err(error) = self.sync_paper_layout(window, cx) {
            let reason = self.failure_notice.take().or_else(|| self.notice.take());
            let error = ToolbarLocale::current(cx).error(&error);
            self.failure_notice = Some(reason.map_or_else(|| error.clone(), |reason| ToolbarLocale::current(cx).app_format("mini.additionalFailure", &[("reason", &reason), ("error", &error)])).into());
        }
        self.schedule_mini_frame(window, cx);
        cx.notify();
    }

    /// 其他文档/主页/关闭入口先返回正常窗口；失败时保留当前会话，让用户可以重试。
    pub(super) fn leave_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.mini.session.as_ref() else { return true; };
        let Some(document) = self.document() else { return false; };
        let previous_mode = session.previous_mode;
        let was_mini = session.paper_mini;
        if was_mini {
            if let Err(error) = document.read(cx).ready(cx) { self.notice = Some(ToolbarLocale::current(cx).error(&error).into()); cx.notify(); return false; }
            if let Err(error) = set_document_layout(&document, previous_mode, false, cx) {
                self.failure_notice = Some(ToolbarLocale::current(cx).error(&error).into()); cx.notify(); return false;
            }
        }
        let session = self.mini.session.as_mut().expect("Mini session");
        let geometry = session.window.geometry(window);
        let restored = match session.window.restore(window) {
            Ok(restored) => restored,
            Err(error) => {
            window.bounds_changed(cx);
            let rollback = if was_mini { set_document_layout(&document, previous_mode, true, cx) } else { Ok(()) };
            self.failure_notice = Some(match rollback { Ok(()) => error.render(ToolbarLocale::current(cx)), Err(restore) => ToolbarLocale::current(cx).app_format("mini.editorRollbackFailed", &[("error", &error.render(ToolbarLocale::current(cx))), ("restore", &ToolbarLocale::current(cx).error(&restore))]) }.into());
            cx.notify(); return false;
            }
        };
        window.bounds_changed(cx);
        self.mini.geometry = Some(geometry);
        let session = self.mini.session.take().expect("Mini session");
        if !restored {
            self.mini.restoring = Some(Box::new(session.window));
            self.notice = Some(ToolbarLocale::current(cx).app_text("mini.confirmRestore").into());
        }
        self.tabs[self.active].mode = previous_mode;
        self.resume_tree(cx);
        if !window.has_active_prompt() { window.focus(&document.focus_handle(cx), cx); }
        self.schedule_mini_frame(window, cx);
        cx.notify();
        true
    }

    pub(super) fn render_mini_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.mini.session.as_ref()?;
        if let Some(title) = self.mini_title(cx) { session.bar.update(cx, |bar, cx| bar.set_title(title, cx)); }
        let opacity = session.window.progress().unwrap_or(1.0);
        Some(gpui::deferred(div().absolute().inset_0().opacity(opacity).child(session.bar.clone()))
            .with_priority(2).into_any_element())
    }
}
