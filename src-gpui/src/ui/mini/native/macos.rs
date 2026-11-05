//! Mini 快照只保存数值；AppKit 变更由更新借用之外的短生命周期主线程任务执行。
use super::{MiniFrame, MiniPin, MiniTarget};
use gpui::Window;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSFloatingWindowLevel, NSNormalWindowLevel, NSScreen, NSView, NSWindow, NSWindowButton,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy)]
pub(crate) struct MiniGeometry {
    frame: NSRect,
}

pub(super) struct MiniWindow {
    state: Rc<RefCell<State>>,
}

#[derive(Clone, Copy)]
struct Original {
    normal: NSRect,
    normal_captured: bool,
    zoomed: bool,
    min_size: NSSize,
    level: isize,
    miniaturizable: bool,
    buttons: [Option<(bool, bool)>; 3],
}

struct State {
    original: Original,
    mini: MiniGeometry,
    pinned: bool,
    desired_pin: bool,
    restored: bool,
    entered: bool,
    alive: bool,
    view_id: usize,
    window_id: usize,
    generation: u64,
    pending: Option<Command>,
    result: Option<Result<(MiniFrame, bool), String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Mini,
    Main,
    Pin,
}

impl Command {
    fn target(self) -> MiniTarget {
        if self == Self::Main {
            MiniTarget::Main
        } else {
            MiniTarget::Mini
        }
    }
}

/// 只在一次主线程任务中持有原生对象；不持有 Workspace、Window 或文档。
pub(crate) struct NativeOperation {
    state: Weak<RefCell<State>>,
    generation: u64,
    command: Command,
    view: Retained<NSView>,
    native: Retained<NSWindow>,
}

enum OperationFailure {
    Stale,
    Native(String),
}

impl From<String> for OperationFailure {
    fn from(error: String) -> Self {
        Self::Native(error)
    }
}

impl State {
    fn queue(&mut self, command: Command) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = Some(command);
        self.result = None;
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.result = None;
    }
}

const BUTTONS: [NSWindowButton; 3] = [
    NSWindowButton::CloseButton,
    NSWindowButton::MiniaturizeButton,
    NSWindowButton::ZoomButton,
];

fn native_view(window: &Window) -> Result<Retained<NSView>, String> {
    MainThreadMarker::new().ok_or("Mini 窗口操作必须在主线程执行")?;
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)
        .map_err(|e| e.to_string())?
        .as_raw()
    else {
        return Err("当前窗口不是 AppKit 窗口".into());
    };
    // SAFETY: GPUI 提供当前借用窗口的有效 NSView；仅为当前操作临时 retain。
    unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
        .ok_or("Mini 所属窗口已经关闭".into())
}

fn native(window: &Window) -> Result<Retained<NSWindow>, String> {
    native_view(window)?
        .window()
        .ok_or("Mini 所属窗口已经关闭".into())
}

fn overlap(a: NSRect, b: NSRect) -> f64 {
    ((a.origin.x + a.size.width).min(b.origin.x + b.size.width) - a.origin.x.max(b.origin.x))
        .max(0.0)
        * ((a.origin.y + a.size.height).min(b.origin.y + b.size.height)
            - a.origin.y.max(b.origin.y))
        .max(0.0)
}

fn visible_area(native: &NSWindow, desired: NSRect) -> Result<NSRect, String> {
    let mtm = MainThreadMarker::new().ok_or("无法读取小窗屏幕")?;
    let screens = NSScreen::screens(mtm);
    let best = screens.iter().max_by(|a, b| {
        overlap(desired, a.visibleFrame()).total_cmp(&overlap(desired, b.visibleFrame()))
    });
    let area = best
        .filter(|s| overlap(desired, s.visibleFrame()) > 0.0)
        .map(|s| s.visibleFrame())
        .or_else(|| native.screen().map(|s| s.visibleFrame()))
        .ok_or("无法读取小窗可见屏幕范围")?;
    if area.size.width <= 0.0 || area.size.height <= 0.0 {
        return Err("小窗屏幕范围无效".into());
    }
    Ok(area)
}

fn clamp(mut frame: NSRect, area: NSRect) -> NSRect {
    frame.size.width = frame.size.width.min(area.size.width).max(1.0);
    frame.size.height = frame.size.height.min(area.size.height).max(1.0);
    frame.origin.x = frame.origin.x.clamp(
        area.origin.x,
        area.origin.x + area.size.width - frame.size.width,
    );
    frame.origin.y = frame.origin.y.clamp(
        area.origin.y,
        area.origin.y + area.size.height - frame.size.height,
    );
    frame
}

impl MiniWindow {
    pub(super) fn enter(
        window: &mut Window,
        previous: Option<MiniGeometry>,
        pinned: bool,
        _: bool,
    ) -> Result<Self, String> {
        let view = native_view(window)?;
        let native = view.window().ok_or("Mini 所属窗口已经关闭")?;
        let before = native.frame();
        let area = visible_area(&native, previous.map_or(before, |g| g.frame))?;
        let zoomed = native.isZoomed();
        let min_size = native.contentMinSize();
        let level = native.level();
        let miniaturizable = native
            .styleMask()
            .contains(NSWindowStyleMask::Miniaturizable);
        let buttons = BUTTONS.map(|kind| {
            native
                .standardWindowButton(kind)
                .map(|button| (button.isHidden(), button.isEnabled()))
        });
        let desired = previous.map_or_else(
            || {
                NSRect::new(
                    NSPoint::new(
                        area.origin.x + (area.size.width - 460.0) / 2.0,
                        area.origin.y + (area.size.height - 560.0) / 2.0,
                    ),
                    NSSize::new(460.0, 560.0),
                )
            },
            |g| g.frame,
        );
        let mini = MiniGeometry {
            frame: clamp(
                NSRect::new(
                    desired.origin,
                    NSSize::new(
                        desired.size.width.max(320.0),
                        desired.size.height.max(240.0),
                    ),
                ),
                area,
            ),
        };
        // 此处只读。普通几何在任务解除 zoom 后捕获，不能在 Workspace 借用中 zoom。
        Ok(Self {
            state: Rc::new(RefCell::new(State {
                original: Original {
                    normal: before,
                    normal_captured: !zoomed,
                    zoomed,
                    min_size,
                    level,
                    miniaturizable,
                    buttons,
                },
                mini,
                pinned: level == NSFloatingWindowLevel,
                desired_pin: pinned,
                restored: true,
                entered: false,
                alive: true,
                view_id: Retained::as_ptr(&view) as usize,
                window_id: Retained::as_ptr(&native) as usize,
                generation: 1,
                pending: Some(Command::Mini),
                result: None,
            })),
        })
    }

    pub(super) fn geometry(&self, window: &Window) -> MiniGeometry {
        let state = self.state.borrow();
        if matches!(state.pending, Some(Command::Mini | Command::Main))
            || state.restored
            || !state.entered
        {
            return state.mini;
        }
        let fallback = state.mini;
        drop(state);
        match native(window) {
            Ok(native) => MiniGeometry {
                frame: native.frame(),
            },
            Err(error) => {
                eprintln!("{error}");
                fallback
            }
        }
    }
    pub(super) fn pin_state(&self) -> MiniPin {
        let state = self.state.borrow();
        if state.pending.is_some() {
            MiniPin::Pending(state.pinned)
        } else {
            MiniPin::Ready(state.pinned)
        }
    }
    pub(super) fn set_pinned(&mut self, pinned: bool, _: &mut Window) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        state.desired_pin = pinned;
        state.queue(Command::Pin);
        Ok(())
    }
    pub(super) fn take_notice(&mut self) -> Option<String> {
        None
    }
    pub(super) fn moving_to(&self) -> Option<MiniTarget> {
        self.state.borrow().pending.map(Command::target)
    }
    pub(super) fn progress(&self) -> Option<f32> {
        None
    }
    pub(super) fn resume_after_handoff(&mut self) {}
    pub(super) fn transition_to(
        &mut self,
        target: MiniTarget,
        _: bool,
        window: &mut Window,
    ) -> Result<bool, String> {
        let geometry = self.geometry(window);
        let mut state = self.state.borrow_mut();
        state.mini = geometry;
        if target == MiniTarget::Main && state.restored {
            state.cancel();
            return Ok(false);
        }
        state.queue(if target == MiniTarget::Main {
            Command::Main
        } else {
            Command::Mini
        });
        Ok(true)
    }
    pub(super) fn advance(
        &mut self,
        _: bool,
        _: bool,
        _: &mut Window,
    ) -> Result<(MiniFrame, bool), String> {
        let mut state = self.state.borrow_mut();
        if let Some(result) = state.result.take() {
            state.pending = None;
            return result;
        }
        Ok((MiniFrame::More, false))
    }
    pub(super) fn settle_mini(&mut self, window: &mut Window) -> Result<(), String> {
        self.transition_to(MiniTarget::Mini, false, window)
            .map(|_| ())
    }
    pub(super) fn restore(&mut self, window: &mut Window) -> Result<bool, String> {
        let state = self.state.borrow();
        if state.restored {
            drop(state);
            self.state.borrow_mut().cancel();
            return Ok(true);
        }
        let pending = state.pending == Some(Command::Main);
        drop(state);
        if !pending {
            self.transition_to(MiniTarget::Main, false, window)?;
        }
        Ok(false)
    }

    pub(super) fn is_restored(&self) -> bool {
        self.state.borrow().restored
    }

    pub(super) fn prepare_operation(&self, window: &Window) -> Option<NativeOperation> {
        let state = self.state.borrow();
        let command = state.pending?;
        if state.result.is_some() {
            return None;
        }
        let generation = state.generation;
        drop(state);
        let operation = (|| {
            let view = native_view(window)?;
            let native = view.window().ok_or("Mini 所属窗口已经关闭")?;
            let state = self.state.borrow();
            if Retained::as_ptr(&view) as usize != state.view_id
                || Retained::as_ptr(&native) as usize != state.window_id
            {
                return Err("Mini 所属窗口已经关闭".into());
            }
            if window.is_fullscreen() || window.is_simple_fullscreen() {
                return Err("请先退出全屏，再切换 Mini 窗口".into());
            }
            Ok(NativeOperation {
                state: Rc::downgrade(&self.state),
                generation,
                command,
                view,
                native,
            })
        })();
        match operation {
            Ok(operation) => Some(operation),
            Err(error) => {
                self.state.borrow_mut().result = Some(Err(error));
                None
            }
        }
    }

    pub(super) fn accepts_operation(&self, operation: &NativeOperation, window: &Window) -> bool {
        let Some(state) = operation.state.upgrade() else {
            return false;
        };
        if !Rc::ptr_eq(&state, &self.state) {
            return false;
        }
        let state = state.borrow();
        if !state.alive || state.generation != operation.generation {
            return false;
        }
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return false;
        };
        matches!(handle.as_raw(), RawWindowHandle::AppKit(handle) if handle.ns_view.as_ptr() as usize == state.view_id)
            && operation.view.window().as_ref().map(Retained::as_ptr)
                == Some(Retained::as_ptr(&operation.native))
    }
}

impl Drop for MiniWindow {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.alive = false;
        state.cancel();
    }
}

impl NativeOperation {
    fn current(&self) -> Result<Rc<RefCell<State>>, OperationFailure> {
        let state = self.state.upgrade().ok_or(OperationFailure::Stale)?;
        {
            let state = state.borrow();
            if !state.alive || state.generation != self.generation {
                return Err(OperationFailure::Stale);
            }
        }
        if self.view.window().as_ref().map(Retained::as_ptr) != Some(Retained::as_ptr(&self.native))
        {
            return Err(OperationFailure::Native("Mini 所属窗口已经关闭".into()));
        }
        if self
            .native
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
        {
            return Err(OperationFailure::Native(
                "请先退出全屏，再切换 Mini 窗口".into(),
            ));
        }
        Ok(state)
    }

    // 不跨越 AppKit 调用持有 RefCell 借用；回调可以绘制、切换目标或关闭窗口。
    fn change(&self, change: impl FnOnce(&NSWindow)) -> Result<(), OperationFailure> {
        self.current()?.borrow_mut().restored = false;
        change(&self.native);
        self.current()?;
        Ok(())
    }

    fn verify(&self, desired: NSRect) -> Result<(), OperationFailure> {
        self.current()?;
        let current = self.native.frame();
        if (current.size.width - desired.size.width).abs() > 2.0
            || (current.size.height - desired.size.height).abs() > 2.0
            || (current.origin.x - desired.origin.x).abs() > 2.0
            || (current.origin.y - desired.origin.y).abs() > 2.0
        {
            return Err(OperationFailure::Native(
                "系统未接受目标窗口的位置和尺寸".into(),
            ));
        }
        Ok(())
    }

    fn pin(&self, pinned: bool) -> Result<(), OperationFailure> {
        let level = if pinned {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        };
        self.change(|native| native.setLevel(level))?;
        self.current()?.borrow_mut().pinned = self.native.level() == NSFloatingWindowLevel;
        if self.native.level() != level {
            return Err(OperationFailure::Native("系统未接受小窗置顶状态".into()));
        }
        Ok(())
    }

    fn enter(&self) -> Result<(), OperationFailure> {
        if self.native.isZoomed() {
            self.change(|native| native.zoom(None))?;
        }
        if self.native.isZoomed() {
            return Err(OperationFailure::Native(
                "系统未还原窗口，请先手动还原后再打开 Mini".into(),
            ));
        }
        let state = self.current()?;
        let (mini, pinned) = {
            let mut state = state.borrow_mut();
            if !state.original.normal_captured {
                state.original.normal = self.native.frame();
                state.original.normal_captured = true;
            }
            (state.mini, state.desired_pin)
        };
        let area = visible_area(&self.native, mini.frame)?;
        let frame = clamp(mini.frame, area);
        self.change(|native| {
            native.setContentMinSize(NSSize::new(
                320.0_f64.min(area.size.width),
                240.0_f64.min(area.size.height),
            ))
        })?;
        self.change(|native| {
            native.setStyleMask(native.styleMask() & !NSWindowStyleMask::Miniaturizable)
        })?;
        for kind in BUTTONS {
            self.change(|native| {
                if let Some(button) = native.standardWindowButton(kind) {
                    button.setHidden(true);
                }
            })?;
            self.change(|native| {
                if let Some(button) = native.standardWindowButton(kind) {
                    button.setEnabled(false);
                }
            })?;
        }
        self.pin(pinned)?;
        self.change(|native| native.setFrame_display(frame, true))?;
        self.verify(frame)?;
        let mut state = state.borrow_mut();
        state.mini.frame = frame;
        state.entered = true;
        Ok(())
    }

    fn restore(&self) -> Result<(), OperationFailure> {
        let state = self.current()?;
        let original = state.borrow().original;
        // 解除 zoom 失败时尚无普通几何，保持 AppKit 的原缩放状态，不覆盖其恢复边界。
        if original.normal_captured && self.native.isZoomed() {
            self.change(|native| native.zoom(None))?;
        }
        self.change(|native| native.setLevel(original.level))?;
        self.change(|native| {
            let mut style = native.styleMask();
            style.set(NSWindowStyleMask::Miniaturizable, original.miniaturizable);
            native.setStyleMask(style);
        })?;
        for (kind, saved) in BUTTONS.into_iter().zip(original.buttons) {
            if let Some((hidden, enabled)) = saved {
                self.change(|native| {
                    if let Some(button) = native.standardWindowButton(kind) {
                        button.setHidden(hidden);
                    }
                })?;
                self.change(|native| {
                    if let Some(button) = native.standardWindowButton(kind) {
                        button.setEnabled(enabled);
                    }
                })?;
            }
        }
        let normal = clamp(
            original.normal,
            visible_area(&self.native, original.normal)?,
        );
        if original.normal_captured {
            self.change(|native| native.setFrame_display(normal, true))?;
        }
        self.change(|native| native.setContentMinSize(original.min_size))?;
        if original.zoomed && !self.native.isZoomed() {
            self.change(|native| native.zoom(None))?;
        }
        if original.zoomed && !self.native.isZoomed() {
            return Err(OperationFailure::Native(
                "系统未恢复主窗口的缩放状态".into(),
            ));
        }
        if !original.zoomed {
            self.verify(normal)?;
        }
        if self.native.level() != original.level {
            return Err(OperationFailure::Native("系统未恢复主窗口的层级".into()));
        }
        self.current()?.borrow_mut().restored = true;
        Ok(())
    }

    /// 必须在 GPUI update 闭包外调用；每个原生变更前后重新检查请求身份。
    pub(crate) fn run(&self) {
        let result = (|| {
            let state = self.current()?;
            let pinned = state.borrow().desired_pin;
            match self.command {
                Command::Mini => self.enter(),
                Command::Main => self.restore(),
                Command::Pin => self.pin(pinned),
            }
        })();
        let result = match result {
            Err(OperationFailure::Native(error)) if self.command == Command::Mini => {
                match self.restore() {
                    Ok(()) => Err(OperationFailure::Native(error)),
                    Err(OperationFailure::Native(rollback)) => {
                        Err(OperationFailure::Native(format!("{error}；{rollback}")))
                    }
                    Err(OperationFailure::Stale) => return,
                }
            }
            result => result,
        };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let mut state = state.borrow_mut();
        if !state.alive || state.generation != self.generation {
            return;
        }
        let result = match result {
            Ok(()) => Ok((MiniFrame::Settled(self.command.target()), true)),
            Err(OperationFailure::Native(error)) => Err(error),
            Err(OperationFailure::Stale) => return,
        };
        state.result = Some(result);
    }
}
