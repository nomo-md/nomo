//! 首个工作区窗口的启动位置，与文档会话和用户偏好分开保存。

use gpui::{App, Bounds, DisplayId, Pixels, PlatformDisplay, Window, WindowBounds, point};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024;

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct WindowPlacement {
    // 普通窗口的还原尺寸与相对显示器工作区的位置。
    bounds: Bounds<Pixels>,
    display: Option<[u8; 16]>,
    maximized: bool,
}

impl WindowPlacement {
    pub(crate) fn capture(
        window: &Window,
        last_windowed: Bounds<Pixels>,
        cx: &App,
    ) -> Option<Self> {
        let window_bounds = window.window_bounds();
        let maximized = match window_bounds {
            WindowBounds::Maximized(_) => true,
            WindowBounds::Fullscreen(_) => false,
            WindowBounds::Windowed(_) => window.is_maximized() || minimized_from_maximized(window),
        };
        // Windows 的 GPUI 后端提供最大化前的还原矩形。其他后端可能只返回当前尺寸。
        let mut bounds = if maximized && !cfg!(windows) {
            last_windowed
        } else {
            window_bounds.get_bounds()
        };
        if !valid_bounds(bounds) {
            return None;
        }
        let display = window.display(cx).and_then(|display| {
            let uuid = display.uuid().ok()?;
            bounds.origin = bounds.origin - work_area(display.as_ref()).origin;
            Some(*uuid.as_bytes())
        });
        Some(Self {
            bounds,
            display,
            maximized,
        })
    }

    pub(crate) fn restore(cx: &App) -> Option<(WindowBounds, DisplayId)> {
        let path = state_path()?;
        let placement = match Self::read(&path) {
            Ok(placement) => placement?,
            Err(error) => {
                eprintln!("读取主窗口位置失败：{error}");
                return None;
            }
        };
        let saved_display = placement.display.and_then(|id| {
            cx.displays()
                .into_iter()
                .find(|display| display.uuid().is_ok_and(|uuid| *uuid.as_bytes() == id))
        });
        let keep_position = saved_display.is_some();
        let display = saved_display.or_else(|| cx.primary_display())?;
        let area = work_area(display.as_ref());
        if !valid_bounds(area) {
            return None;
        }
        let size = placement.bounds.size.min(&area.size);
        let bounds = if keep_position {
            let origin = area.origin + placement.bounds.origin;
            Bounds::new(
                point(
                    origin.x.clamp(area.origin.x, area.origin.x + (area.size.width - size.width)),
                    origin.y.clamp(area.origin.y, area.origin.y + (area.size.height - size.height)),
                ),
                size,
            )
        } else {
            Bounds::centered_at(area.center(), size)
        };
        let window_bounds = if placement.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        Some((window_bounds, display.id()))
    }

    pub(crate) fn save(self) {
        let Some(path) = state_path() else {
            return;
        };
        let result = (|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let bytes = serde_json::to_vec(&self).map_err(|error| error.to_string())?;
            crate::file_system::write_file_atomically(&path, &bytes)
        })();
        if let Err(error) = result {
            eprintln!("保存主窗口位置失败：{error}");
        }
    }

    fn read(path: &Path) -> Result<Option<Self>, String> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("记录超过 1 KiB".into());
        }
        let placement: Self = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if !valid_bounds(placement.bounds) {
            return Err("窗口坐标或尺寸无效".into());
        }
        Ok(Some(placement))
    }
}

fn state_path() -> Option<PathBuf> {
    Some(crate::settings::config_path()?.with_file_name("gpui-window-placement-v1.json"))
}

fn valid_bounds(bounds: Bounds<Pixels>) -> bool {
    [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height]
        .into_iter()
        .all(|value| value.as_f32().is_finite())
        && bounds.size.width.as_f32() > 0.0
        && bounds.size.height.as_f32() > 0.0
}

fn work_area(display: &dyn PlatformDisplay) -> Bounds<Pixels> {
    let visible = display.visible_bounds();
    // Win32 WINDOWPLACEMENT 使用工作区坐标，原点不含顶部或左侧任务栏的偏移。
    if cfg!(windows) {
        Bounds::new(display.bounds().origin, visible.size)
    } else {
        visible
    }
}

#[cfg(windows)]
fn minimized_from_maximized(window: &Window) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, IsIconic, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as _;
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: HWND 来自当前借用的 GPUI 窗口，输出结构及其长度已初始化。
    let minimized = unsafe { IsIconic(hwnd) != 0 && GetWindowPlacement(hwnd, &mut placement) != 0 };
    minimized && placement.flags & WPF_RESTORETOMAXIMIZED != 0
}

#[cfg(not(windows))]
fn minimized_from_maximized(_: &Window) -> bool {
    false
}
