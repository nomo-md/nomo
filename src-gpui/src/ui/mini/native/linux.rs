//! Linux 窗口状态由窗口管理器异步确认；只在一次切换期间调度检查。
use super::{MiniFrame, MiniPin, MiniTarget};
use gpui::{Decorations, Pixels, Size, Window, px, size};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};

mod x11;

pub(super) const CONFIRM_TIMEOUT: Duration = Duration::from_secs(2);
const WAYLAND_LIMITS: &str = "mini.waylandLimits";

#[derive(Clone, Copy)]
pub(crate) enum MiniGeometry {
    Wayland(Size<Pixels>),
    X11(x11::Geometry),
}

pub(super) enum MiniWindow {
    Wayland(WaylandMini),
    X11(x11::X11Mini),
}

impl MiniWindow {
    pub(super) fn enter(
        window: &mut Window,
        previous: Option<MiniGeometry>,
        pinned: bool,
        _: bool,
    ) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|e| e.to_string())?
            .as_raw();
        match handle {
            RawWindowHandle::Wayland(_) => {
                let prior = if let Some(MiniGeometry::Wayland(size)) = previous {
                    Some(size)
                } else {
                    None
                };
                Ok(Self::Wayland(WaylandMini::enter(window, prior)))
            }
            RawWindowHandle::Xcb(_) => {
                let prior = if let Some(MiniGeometry::X11(geometry)) = previous {
                    Some(geometry)
                } else {
                    None
                };
                x11::X11Mini::enter(window, prior, pinned).map(Self::X11)
            }
            _ => Err("当前 Linux 窗口后端不支持 Markdown Mini".into()),
        }
    }
    pub(super) fn geometry(&self, window: &Window) -> MiniGeometry {
        match self {
            Self::Wayland(state) => MiniGeometry::Wayland(
                if state.pending.is_some() || state.target == MiniTarget::Main {
                    state.mini
                } else {
                    window.viewport_size()
                },
            ),
            Self::X11(state) => MiniGeometry::X11(state.geometry(window)),
        }
    }
    pub(super) fn pin_state(&self) -> MiniPin {
        match self {
            Self::Wayland(_) => {
                MiniPin::Unavailable("mini.waylandPin")
            }
            Self::X11(state) => state.pin_state(),
        }
    }
    pub(super) fn set_pinned(&mut self, pinned: bool, window: &mut Window) -> Result<(), String> {
        match self {
            Self::Wayland(_) => Err("Wayland 不支持应用内置顶".into()),
            Self::X11(state) => state.set_pinned(pinned, window),
        }
    }
    pub(super) fn take_notice(&mut self, locale: crate::ui::toolbar_i18n::ToolbarLocale) -> Option<String> {
        match self {
            Self::Wayland(state) => state.notice.take().map(|key| locale.app_text(key).to_owned()),
            Self::X11(state) => state.take_notice(locale),
        }
    }
    pub(super) fn moving_to(&self) -> Option<MiniTarget> {
        match self {
            Self::Wayland(state) => state.pending.map(|_| state.target),
            Self::X11(state) => state.moving_to(),
        }
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
        match self {
            Self::Wayland(state) => {
                state.begin(target, window);
                Ok(state.pending.is_some())
            }
            Self::X11(state) => state.transition_to(target, window),
        }
    }
    pub(super) fn advance(
        &mut self,
        _: bool,
        _: bool,
        window: &mut Window,
    ) -> Result<(MiniFrame, bool), String> {
        match self {
            Self::Wayland(state) => state.advance(window),
            Self::X11(state) => state.advance(window),
        }
    }
    pub(super) fn settle_mini(&mut self, window: &mut Window) -> Result<(), String> {
        self.transition_to(MiniTarget::Mini, false, window)
            .map(|_| ())
    }
    pub(super) fn restore(&mut self, window: &mut Window) -> Result<bool, String> {
        match self {
            Self::Wayland(state) => {
                if state.restored {
                    return Ok(true);
                }
                if state.target != MiniTarget::Main || state.pending.is_none() {
                    state.begin(MiniTarget::Main, window);
                }
                Ok(false)
            }
            Self::X11(state) => state.restore(window),
        }
    }
}

#[derive(Clone, Copy)]
enum WaylandStep {
    Unmaximize,
    Size(Size<Pixels>),
    Maximize,
}

pub(super) struct WaylandMini {
    main: Size<Pixels>,
    captured_normal: bool,
    was_maximized: bool,
    mini: Size<Pixels>,
    target: MiniTarget,
    pending: Option<WaylandStep>,
    deadline: Instant,
    restored: bool,
    entered: bool,
    notice: Option<&'static str>,
}

impl WaylandMini {
    fn enter(window: &mut Window, previous: Option<Size<Pixels>>) -> Self {
        let desired = previous.unwrap_or(size(px(640.0), px(560.0)));
        let mut state = Self {
            main: window.viewport_size(),
            captured_normal: !window.is_maximized(),
            was_maximized: window.is_maximized(),
            mini: size(desired.width.max(px(640.0)), desired.height.max(px(420.0))),
            target: MiniTarget::Mini,
            pending: None,
            deadline: Instant::now() + CONFIRM_TIMEOUT,
            restored: false,
            entered: false,
            notice: Some(WAYLAND_LIMITS),
        };
        state.begin(MiniTarget::Mini, window);
        state
    }

    fn begin(&mut self, target: MiniTarget, window: &mut Window) {
        if self.target == MiniTarget::Mini
            && self.entered
            && self.pending.is_none()
            && !window.is_maximized()
        {
            self.mini = window.viewport_size();
        }
        self.target = target;
        self.deadline = Instant::now() + CONFIRM_TIMEOUT;
        self.restored = false;
        // GPUI 的 zoom 是切换请求；前一次解除最大化尚未确认时不能再次盲目切换。
        if matches!(
            self.pending,
            Some(WaylandStep::Unmaximize | WaylandStep::Maximize)
        ) {
            return;
        }
        if window.is_maximized() {
            window.zoom_window();
            self.pending = Some(WaylandStep::Unmaximize);
        } else {
            self.resize(window);
        }
    }

    fn resize(&mut self, window: &mut Window) {
        if !self.captured_normal {
            self.main = window.viewport_size();
            self.captured_normal = true;
        }
        let tiled = matches!(window.window_decorations(), Decorations::Client { tiling } if tiling.top || tiling.bottom || tiling.left || tiling.right);
        let desired = if tiled {
            self.notice =
                Some("mini.tiled");
            window.viewport_size()
        } else if self.target == MiniTarget::Mini {
            self.mini
        } else {
            self.main
        };
        // 只有解除最大化后的普通窗口才能自行提交较小的 surface，避免违反 xdg-shell 配置。
        window.resize(desired);
        self.pending = Some(WaylandStep::Size(desired));
    }

    fn advance(&mut self, window: &mut Window) -> Result<(MiniFrame, bool), String> {
        let Some(step) = self.pending else {
            return Ok((MiniFrame::Settled(self.target), false));
        };
        if window.is_fullscreen() {
            self.pending = None;
            return Err("窗口已进入全屏，已停止 Mini 尺寸调整；退出全屏后可重试返回".into());
        }
        if Instant::now() >= self.deadline {
            self.pending = None;
            return Err("窗口管理器未在期限内确认 Mini 窗口状态；可重试返回主窗口".into());
        }
        match step {
            WaylandStep::Unmaximize if !window.is_maximized() => self.resize(window),
            WaylandStep::Size(desired) if same_size(window.viewport_size(), desired) => {
                if self.target == MiniTarget::Main && self.was_maximized {
                    if !window.is_maximized() {
                        window.zoom_window();
                    }
                    self.pending = Some(WaylandStep::Maximize);
                } else {
                    return Ok(self.finish());
                }
            }
            WaylandStep::Maximize if window.is_maximized() => {
                if self.target == MiniTarget::Main {
                    return Ok(self.finish());
                }
                window.zoom_window();
                self.pending = Some(WaylandStep::Unmaximize);
            }
            _ => {}
        }
        Ok((MiniFrame::More, false))
    }

    fn finish(&mut self) -> (MiniFrame, bool) {
        self.pending = None;
        self.restored = self.target == MiniTarget::Main;
        if !self.restored {
            self.entered = true;
        }
        (MiniFrame::Settled(self.target), true)
    }
}

fn same_size(a: Size<Pixels>, b: Size<Pixels>) -> bool {
    (f32::from(a.width - b.width)).abs() <= 1.0 && (f32::from(a.height - b.height)).abs() <= 1.0
}
