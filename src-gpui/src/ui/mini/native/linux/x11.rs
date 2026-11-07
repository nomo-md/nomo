//! 借用 GPUI 的 XCB 连接发送自己的请求，不接管事件队列、不关闭连接。
use super::{CONFIRM_TIMEOUT, MiniFrame, MiniPin, MiniTarget};
use gpui::Window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::time::Instant;
use x11rb::{
    connection::Connection,
    properties::WmSizeHints,
    protocol::{
        randr::ConnectionExt as _,
        xproto::{AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, EventMask},
    },
    xcb_ffi::XCBConnection,
};

#[derive(Clone, Copy)]
pub(crate) struct Geometry {
    rect: Rect,
    scale: f32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

struct Atoms {
    state: u32,
    above: u32,
    below: u32,
    skip: u32,
    max_h: u32,
    max_v: u32,
    moveresize: u32,
    workarea: u32,
    desktop: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Flags {
    above: bool,
    below: bool,
    skip: bool,
    max_h: bool,
    max_v: bool,
}
impl Flags {
    fn maximized(self) -> bool {
        self.max_h || self.max_v
    }
}
#[derive(Clone, Copy)]
enum Step {
    Start,
    Unmaximize,
    Geometry,
    Maximize,
    Pin,
}

pub(in super::super) struct X11Mini {
    window: u32,
    root: u32,
    atoms: Atoms,
    original: Flags,
    hints: Option<WmSizeHints>,
    main: Rect,
    captured_normal: bool,
    unmaximized_sample: Option<Rect>,
    mini: Geometry,
    requested_rect: Rect,
    pin_supported: bool,
    skip_supported: bool,
    position_supported: bool,
    pinned: bool,
    desired_pin: bool,
    pending_pin: bool,
    target: MiniTarget,
    pending: Option<Step>,
    deadline: Instant,
    restored: bool,
    entered: bool,
    notice: Vec<&'static str>,
}

fn error(e: impl std::fmt::Display) -> String {
    format!("Mini X11 窗口操作失败：{e}")
}

fn connection(window: &Window) -> Result<(XCBConnection, u32), String> {
    let RawWindowHandle::Xcb(handle) = HasWindowHandle::window_handle(window)
        .map_err(error)?
        .as_raw()
    else {
        return Err("Mini 所属窗口不是 X11 窗口".into());
    };
    let RawDisplayHandle::Xcb(display) = HasDisplayHandle::display_handle(window)
        .map_err(error)?
        .as_raw()
    else {
        return Err("Mini 所属显示连接不是 XCB".into());
    };
    let pointer = display.connection.ok_or("XCB 连接已经关闭")?;
    // SAFETY: 包装器只在当前借用 Window 的同步调用内使用；false 保证不释放 GPUI 的连接。
    let connection = unsafe { XCBConnection::from_raw_xcb_connection(pointer.as_ptr(), false) }
        .map_err(error)?;
    Ok((connection, handle.window.get()))
}

fn atom(connection: &XCBConnection, name: &[u8]) -> Result<u32, String> {
    Ok(connection
        .intern_atom(false, name)
        .map_err(error)?
        .reply()
        .map_err(error)?
        .atom)
}

fn property(
    connection: &XCBConnection,
    window: u32,
    name: u32,
    kind: AtomEnum,
    limit: u32,
) -> Result<Vec<u32>, String> {
    let reply = connection
        .get_property(false, window, name, kind, 0, limit)
        .map_err(error)?
        .reply()
        .map_err(error)?;
    if reply.bytes_after != 0 {
        return Err("Mini 窗口属性超过读取预算".into());
    }
    if reply.type_ == u32::from(AtomEnum::NONE) {
        return Ok(Vec::new());
    }
    let values = reply.value32().ok_or("Mini 窗口属性格式无效")?;
    Ok(values.collect())
}

fn rect(connection: &XCBConnection, window: u32, root: u32) -> Result<Rect, String> {
    let geometry = connection
        .get_geometry(window)
        .map_err(error)?
        .reply()
        .map_err(error)?;
    let origin = connection
        .translate_coordinates(window, root, 0, 0)
        .map_err(error)?
        .reply()
        .map_err(error)?;
    Ok(Rect {
        x: origin.dst_x.into(),
        y: origin.dst_y.into(),
        width: geometry.width.into(),
        height: geometry.height.into(),
    })
}

impl Atoms {
    fn flags(&self, connection: &XCBConnection, window: u32) -> Result<Flags, String> {
        let values = property(connection, window, self.state, AtomEnum::ATOM, 64)?;
        Ok(Flags {
            above: values.contains(&self.above),
            below: values.contains(&self.below),
            skip: values.contains(&self.skip),
            max_h: values.contains(&self.max_h),
            max_v: values.contains(&self.max_v),
        })
    }
}

impl X11Mini {
    pub(in super::super) fn enter(
        window: &Window,
        previous: Option<Geometry>,
        pinned: bool,
    ) -> Result<Self, String> {
        let (connection, xid) = connection(window)?;
        let root = connection
            .get_geometry(xid)
            .map_err(error)?
            .reply()
            .map_err(error)?
            .root;
        let atoms = Atoms {
            state: atom(&connection, b"_NET_WM_STATE")?,
            above: atom(&connection, b"_NET_WM_STATE_ABOVE")?,
            below: atom(&connection, b"_NET_WM_STATE_BELOW")?,
            skip: atom(&connection, b"_NET_WM_STATE_SKIP_TASKBAR")?,
            max_h: atom(&connection, b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            max_v: atom(&connection, b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            moveresize: atom(&connection, b"_NET_MOVERESIZE_WINDOW")?,
            workarea: atom(&connection, b"_NET_WORKAREA")?,
            desktop: atom(&connection, b"_NET_CURRENT_DESKTOP")?,
        };
        let supported = property(
            &connection,
            root,
            atom(&connection, b"_NET_SUPPORTED")?,
            AtomEnum::ATOM,
            512,
        )?;
        let original = atoms.flags(&connection, xid)?;
        if original.maximized()
            && (!supported.contains(&atoms.state)
                || !supported.contains(&atoms.max_h)
                || !supported.contains(&atoms.max_v))
        {
            return Err("窗口管理器不支持恢复最大化状态，请先手动还原窗口再打开 Mini".into());
        }
        let hints = WmSizeHints::get_normal_hints(&connection, xid)
            .map_err(error)?
            .reply()
            .map_err(error)?;
        let main = rect(&connection, xid, root)?;
        let pin_supported = supported.contains(&atoms.state)
            && supported.contains(&atoms.above)
            && supported.contains(&atoms.below);
        let skip_supported = supported.contains(&atoms.state) && supported.contains(&atoms.skip);
        let position_supported = supported.contains(&atoms.moveresize);
        let scale = window.scale_factor().max(0.1);
        let mut state = Self {
            window: xid,
            root,
            atoms,
            original,
            hints,
            main,
            captured_normal: !original.maximized(),
            unmaximized_sample: None,
            mini: previous.unwrap_or(Geometry { rect: main, scale }),
            requested_rect: main,
            pin_supported,
            skip_supported,
            position_supported,
            pinned: original.above,
            desired_pin: pinned,
            pending_pin: pin_supported,
            target: MiniTarget::Mini,
            pending: Some(Step::Start),
            deadline: Instant::now() + CONFIRM_TIMEOUT,
            restored: false,
            entered: false,
            notice: Vec::new(),
        };
        let area = state.work_area(&connection, previous.map_or(main, |g| g.rect))?;
        let desired = previous.map_or(
            Rect {
                x: area.x,
                y: area.y,
                width: (460.0 * scale).round() as u32,
                height: (560.0 * scale).round() as u32,
            },
            |g| Rect {
                width: (g.rect.width as f32 * scale / g.scale.max(0.1)).round() as u32,
                height: (g.rect.height as f32 * scale / g.scale.max(0.1)).round() as u32,
                ..g.rect
            },
        );
        let mut desired = Rect {
            width: desired.width.max((320.0 * scale) as u32),
            height: desired.height.max((240.0 * scale) as u32),
            ..desired
        };
        if previous.is_none() {
            desired.x += (area.width as i32 - desired.width as i32) / 2;
            desired.y += (area.height as i32 - desired.height as i32) / 2;
        }
        state.mini = Geometry {
            rect: clamp(desired, area),
            scale,
        };
        if !pin_supported {
            state.add_notice("mini.noPin");
        }
        if !position_supported {
            state.add_notice("mini.noPosition");
        }
        if !skip_supported {
            state.add_notice("mini.noTaskbarHide");
        }
        Ok(state)
    }

    fn add_notice(&mut self, notice: &'static str) {
        // 六种固定通知按键去重，待显示文案不会随状态确认累积。
        if !self.notice.contains(&notice) { self.notice.push(notice); }
    }
    pub(in super::super) fn take_notice(&mut self, locale: crate::ui::toolbar_i18n::ToolbarLocale) -> Option<String> {
        if self.notice.is_empty() { return None; }
        Some(self.notice.drain(..).map(|key| locale.app_text(key)).collect::<Vec<_>>().join("\n"))
    }
    pub(in super::super) fn pin_state(&self) -> MiniPin {
        if !self.pin_supported {
            MiniPin::Unavailable("mini.noPin")
        } else if self.pending_pin {
            MiniPin::Pending(self.pinned)
        } else {
            MiniPin::Ready(self.pinned)
        }
    }
    pub(in super::super) fn moving_to(&self) -> Option<MiniTarget> {
        self.pending.map(|_| self.target)
    }
    pub(in super::super) fn geometry(&self, window: &Window) -> Geometry {
        if self.target == MiniTarget::Main || !self.entered {
            return self.mini;
        }
        match connection(window).and_then(|(connection, xid)| {
            if self.atoms.flags(&connection, xid)?.maximized() {
                return Ok(self.mini);
            }
            rect(&connection, xid, self.root).map(|rect| Geometry {
                rect,
                scale: window.scale_factor().max(0.1),
            })
        }) {
            Ok(geometry) => geometry,
            Err(error) => {
                eprintln!("{error}");
                self.mini
            }
        }
    }

    fn send_state(
        &self,
        connection: &XCBConnection,
        enabled: bool,
        first: u32,
        second: u32,
    ) -> Result<(), String> {
        let event = ClientMessageEvent::new(
            32,
            self.window,
            self.atoms.state,
            [u32::from(enabled), first, second, 1, 0],
        );
        connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                event,
            )
            .map_err(error)?
            .check()
            .map_err(error)
    }
    fn request_pin(
        &self,
        connection: &XCBConnection,
        pinned: bool,
        below: bool,
    ) -> Result<(), String> {
        if self.pin_supported {
            self.send_state(connection, pinned, self.atoms.above, 0)?;
            self.send_state(connection, below, self.atoms.below, 0)?;
        }
        Ok(())
    }
    pub(in super::super) fn set_pinned(
        &mut self,
        pinned: bool,
        window: &Window,
    ) -> Result<(), String> {
        if !self.pin_supported {
            return Err("窗口管理器不支持应用内置顶".into());
        }
        if self.pending.is_some() {
            return Err("窗口状态仍在确认，请稍后切换置顶".into());
        }
        let (connection, _) = connection(window)?;
        self.request_pin(&connection, pinned, false)?;
        connection.flush().map_err(error)?;
        self.desired_pin = pinned;
        self.pending_pin = true;
        self.pending = Some(Step::Pin);
        self.deadline = Instant::now() + CONFIRM_TIMEOUT;
        Ok(())
    }

    pub(in super::super) fn transition_to(
        &mut self,
        target: MiniTarget,
        window: &Window,
    ) -> Result<bool, String> {
        if self.target == MiniTarget::Mini
            && self.entered
            && matches!(self.pending, None | Some(Step::Pin))
        {
            self.mini = self.geometry(window);
        }
        self.target = target;
        self.restored = false;
        self.deadline = Instant::now() + CONFIRM_TIMEOUT;
        // 最大化请求仍在路上时先等确认，再应用折返；不用 toggle 猜测窗口管理器状态。
        if !matches!(self.pending, Some(Step::Unmaximize | Step::Maximize)) {
            self.pending = Some(Step::Start);
        }
        Ok(true)
    }
    pub(in super::super) fn restore(&mut self, window: &Window) -> Result<bool, String> {
        if self.restored {
            return Ok(true);
        }
        if self.target != MiniTarget::Main || self.pending.is_none() {
            self.transition_to(MiniTarget::Main, window)?;
        }
        Ok(false)
    }

    fn apply_geometry(&mut self, connection: &XCBConnection, scale: f32) -> Result<(), String> {
        if !self.captured_normal {
            self.main = rect(connection, self.window, self.root)?;
            self.captured_normal = true;
        }
        let mini = self.target == MiniTarget::Mini;
        if mini {
            let mut hints = self.hints.clone().unwrap_or_default();
            hints.min_size = Some((
                (320.0 * scale).round() as i32,
                (240.0 * scale).round() as i32,
            ));
            hints
                .set_normal_hints(connection, self.window)
                .map_err(error)?
                .check()
                .map_err(error)?;
        } else if let Some(hints) = &self.hints {
            hints
                .set_normal_hints(connection, self.window)
                .map_err(error)?
                .check()
                .map_err(error)?;
        } else {
            connection
                .delete_property(self.window, AtomEnum::WM_NORMAL_HINTS.into())
                .map_err(error)?
                .check()
                .map_err(error)?;
        }
        let pin = if mini {
            self.desired_pin
        } else {
            self.original.above
        };
        self.request_pin(connection, pin, !mini && self.original.below)?;
        self.pending_pin = self.pin_supported;
        if self.skip_supported {
            self.send_state(connection, mini || self.original.skip, self.atoms.skip, 0)?;
        }
        let desired = if mini { self.mini.rect } else { self.main };
        self.requested_rect = clamp(desired, self.work_area(connection, desired)?);
        let r = self.requested_rect;
        if self.position_supported {
            // StaticGravity 的根坐标指向客户区；避免把系统标题栏高度误加到下一次恢复。
            let event = ClientMessageEvent::new(
                32,
                self.window,
                self.atoms.moveresize,
                [
                    10 | (15 << 8) | (1 << 12),
                    r.x as u32,
                    r.y as u32,
                    r.width,
                    r.height,
                ],
            );
            connection
                .send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                    event,
                )
                .map_err(error)?
                .check()
                .map_err(error)?;
        } else {
            connection
                .configure_window(
                    self.window,
                    &ConfigureWindowAux::new().width(r.width).height(r.height),
                )
                .map_err(error)?
                .check()
                .map_err(error)?;
        }
        connection.flush().map_err(error)?;
        self.pending = Some(Step::Geometry);
        Ok(())
    }

    pub(in super::super) fn advance(
        &mut self,
        window: &Window,
    ) -> Result<(MiniFrame, bool), String> {
        let result = self.poll(window);
        if result.is_err() {
            self.pending = None;
            self.pending_pin = false;
        }
        result
    }
    fn poll(&mut self, window: &Window) -> Result<(MiniFrame, bool), String> {
        let Some(step) = self.pending else {
            return Ok((MiniFrame::Settled(self.target), false));
        };
        if window.is_fullscreen() {
            return Err("窗口已进入全屏，已停止 Mini 尺寸调整；退出全屏后可重试返回".into());
        }
        let (connection, xid) = connection(window)?;
        if xid != self.window {
            return Err("Mini 所属窗口已经改变".into());
        }
        let flags = self.atoms.flags(&connection, xid)?;
        self.pinned = flags.above;
        let expired = Instant::now() >= self.deadline;
        if expired && matches!(step, Step::Start | Step::Unmaximize | Step::Maximize) {
            return Err(
                "窗口管理器未在期限内确认 Mini 窗口状态；已保留恢复信息，可重试返回主窗口".into(),
            );
        }
        match step {
            Step::Start => {
                if flags.maximized() {
                    self.send_state(&connection, false, self.atoms.max_h, self.atoms.max_v)?;
                    connection.flush().map_err(error)?;
                    self.unmaximized_sample = None;
                    self.pending = Some(Step::Unmaximize);
                } else {
                    self.apply_geometry(&connection, window.scale_factor())?;
                }
            }
            Step::Unmaximize if !flags.maximized() => {
                let current = rect(&connection, xid, self.root)?;
                // WM 可以先清除最大化属性、再提交 ConfigureWindow；避免把最大化尺寸记成普通尺寸。
                if self.captured_normal || self.unmaximized_sample == Some(current) {
                    self.apply_geometry(&connection, window.scale_factor())?;
                } else {
                    self.unmaximized_sample = Some(current);
                }
            }
            Step::Geometry => {
                let current = rect(&connection, xid, self.root)?;
                let r = self.requested_rect;
                let sized =
                    current.width.abs_diff(r.width) <= 2 && current.height.abs_diff(r.height) <= 2;
                let positioned = !self.position_supported
                    || (current.x.abs_diff(r.x) <= 2 && current.y.abs_diff(r.y) <= 2);
                let mini = self.target == MiniTarget::Mini;
                let pinned = !self.pin_supported
                    || (flags.above
                        == if mini {
                            self.desired_pin
                        } else {
                            self.original.above
                        }
                        && flags.below == (!mini && self.original.below));
                let taskbar = !self.skip_supported || flags.skip == (mini || self.original.skip);
                if sized && positioned && pinned && taskbar && !flags.maximized() {
                    self.pending_pin = false;
                    if !mini && self.original.maximized() {
                        self.send_state(&connection, self.original.max_h, self.atoms.max_h, 0)?;
                        self.send_state(&connection, self.original.max_v, self.atoms.max_v, 0)?;
                        connection.flush().map_err(error)?;
                        self.pending = Some(Step::Maximize);
                    } else {
                        return Ok(self.finish(current, window.scale_factor()));
                    }
                } else if expired && mini {
                    if !sized || !positioned {
                        self.add_notice("mini.geometryRejected");
                    }
                    if !pinned {
                        self.add_notice("mini.pinRejected");
                    }
                    if !taskbar {
                        self.add_notice("mini.taskbarRetained");
                    }
                    return Ok(self.finish(current, window.scale_factor()));
                }
            }
            Step::Maximize
                if flags.max_h == self.original.max_h && flags.max_v == self.original.max_v =>
            {
                if self.target == MiniTarget::Main {
                    return Ok(
                        self.finish(rect(&connection, xid, self.root)?, window.scale_factor())
                    );
                }
                self.pending = Some(Step::Start);
            }
            Step::Pin => {
                if flags.above == self.desired_pin && !flags.below {
                    self.pending_pin = false;
                    self.pending = None;
                    return Ok((MiniFrame::Settled(MiniTarget::Mini), false));
                }
            }
            _ => {}
        }
        if expired {
            return Err(
                "窗口管理器未在期限内确认 Mini 窗口状态；已保留恢复信息，可重试返回主窗口".into(),
            );
        }
        Ok((MiniFrame::More, false))
    }

    fn finish(&mut self, rect: Rect, scale: f32) -> (MiniFrame, bool) {
        self.pending = None;
        self.pending_pin = false;
        self.restored = self.target == MiniTarget::Main;
        if !self.restored {
            self.entered = true;
            self.mini = Geometry {
                rect,
                scale: scale.max(0.1),
            };
        }
        (MiniFrame::Settled(self.target), true)
    }

    fn work_area(&self, connection: &XCBConnection, desired: Rect) -> Result<Rect, String> {
        let root = connection
            .get_geometry(self.root)
            .map_err(error)?
            .reply()
            .map_err(error)?;
        let screen = Rect {
            x: 0,
            y: 0,
            width: root.width.into(),
            height: root.height.into(),
        };
        // RandR 1.5 不是所有 X Server 都有；不可用时使用根窗口工作区，不假设连接的 screen_id 等于显示器编号。
        let monitors = connection
            .randr_get_monitors(self.root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let monitor = monitors
            .as_ref()
            .and_then(|reply| {
                reply
                    .monitors
                    .iter()
                    .take(64)
                    .filter(|m| m.width > 0 && m.height > 0)
                    .max_by_key(|m| {
                        let area = Rect {
                            x: m.x.into(),
                            y: m.y.into(),
                            width: m.width.into(),
                            height: m.height.into(),
                        };
                        (overlap(desired, area), overlap(self.main, area), m.primary)
                    })
            })
            .map(|m| Rect {
                x: m.x.into(),
                y: m.y.into(),
                width: m.width.into(),
                height: m.height.into(),
            })
            .unwrap_or(screen);
        let desktop = property(
            connection,
            self.root,
            self.atoms.desktop,
            AtomEnum::CARDINAL,
            1,
        )?
        .first()
        .copied()
        .unwrap_or(0) as usize;
        let areas = property(
            connection,
            self.root,
            self.atoms.workarea,
            AtomEnum::CARDINAL,
            256,
        )?;
        let work = desktop
            .checked_mul(4)
            .and_then(|start| areas.get(start..start + 4))
            .filter(|a| a[2] > 0 && a[3] > 0)
            .map(|a| Rect {
                x: a[0] as i32,
                y: a[1] as i32,
                width: a[2],
                height: a[3],
            })
            .unwrap_or(screen);
        Ok(intersect(monitor, work).unwrap_or(monitor))
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    (right > i64::from(x) && bottom > i64::from(y)).then(|| Rect {
        x,
        y,
        width: (right - i64::from(x)) as u32,
        height: (bottom - i64::from(y)) as u32,
    })
}
fn overlap(a: Rect, b: Rect) -> u64 {
    intersect(a, b).map_or(0, |r| u64::from(r.width) * u64::from(r.height))
}
fn clamp(mut rect: Rect, area: Rect) -> Rect {
    rect.width = rect.width.max(1).min(area.width.max(1));
    rect.height = rect.height.max(1).min(area.height.max(1));
    rect.x = rect
        .x
        .clamp(area.x, area.x + (area.width - rect.width) as i32);
    rect.y = rect
        .y
        .clamp(area.y, area.y + (area.height - rect.height) as i32);
    rect
}
