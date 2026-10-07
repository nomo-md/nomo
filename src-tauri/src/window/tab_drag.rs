//! Windows 标签 Pointer 手势的屏幕命中。独立于 WebView 原生文件拖入通道。
use super::tab_transfer::{DropPlacement, ScreenPoint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

static STATE: OnceLock<Mutex<DragState>> = OnceLock::new();

#[derive(Default)]
struct DragState {
    windows: HashMap<String, RegisteredZones>,
    active: Option<(String, String)>,
}
struct RegisteredZones {
    ready: bool,
    zones: Vec<DropZone>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropZone {
    id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    placement: DropPlacement,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DragInput {
    drag_id: String,
    item_id: String,
    tab_id: Option<String>,
    can_combine: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DragEvent {
    #[serde(flatten)]
    input: DragInput,
    phase: String,
    source_window_label: String,
    target_window_label: Option<String>,
    placement: Option<DropPlacement>,
    screen_position: ScreenPoint,
    source_client_position: Option<tauri::LogicalPosition<f64>>,
}

fn state() -> &'static Mutex<DragState> {
    STATE.get_or_init(Default::default)
}

pub(crate) fn window_ready(label: &str) -> bool {
    state()
        .lock()
        .ok()
        .and_then(|state| state.windows.get(label).map(|zones| zones.ready))
        .unwrap_or(false)
}

pub(crate) fn forget_window(label: &str) {
    if let Ok(mut state) = state().lock() {
        state.windows.remove(label);
        if state
            .active
            .as_ref()
            .is_some_and(|(source, _)| source == label)
        {
            state.active = None;
        }
    }
}

#[tauri::command]
pub(crate) fn register_tab_drop_zones(
    window: WebviewWindow,
    zones: Vec<DropZone>,
    ready: bool,
) -> Result<(), String> {
    if !super::external_open::is_document_window_label(window.label()) {
        return Err("非文档窗口不能注册标签落点".into());
    }
    if zones.len() > 1000
        || zones.iter().any(|zone| {
            zone.id.is_empty()
                || ![zone.x, zone.y, zone.width, zone.height]
                    .iter()
                    .all(|value| value.is_finite())
                || zone.width < 0.0
                || zone.height < 0.0
        })
    {
        return Err("标签落点矩形无效".into());
    }
    state()
        .lock()
        .map_err(|_| "锁定标签拖动状态失败")?
        .windows
        .insert(window.label().into(), RegisteredZones { zones, ready });
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_native_tab_drag(window: WebviewWindow, drag_id: String) -> Result<(), String> {
    let mut state = state().lock().map_err(|_| "锁定标签拖动状态失败")?;
    if state
        .active
        .as_ref()
        .is_some_and(|(source, id)| source == window.label() && id == &drag_id)
    {
        state.active = None;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn begin_native_tab_drag(
    app: AppHandle,
    window: WebviewWindow,
    input: DragInput,
) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("跨窗口标签拖动仅支持 Windows".into());
    }
    if !super::external_open::is_document_window_label(window.label())
        || super::state::is_markdown_mini_mode_window(window.label())
    {
        return Err("当前窗口不支持标签拖动".into());
    }
    if input.drag_id.is_empty() || input.item_id.is_empty() {
        return Err("拖动项无效".into());
    }
    let source = window.label().to_string();
    {
        let mut state = state().lock().map_err(|_| "锁定标签拖动状态失败")?;
        if state.active.is_some() {
            return Err("已有标签拖动正在进行".into());
        }
        state.active = Some((source.clone(), input.drag_id.clone()));
    }
    #[cfg(target_os = "windows")]
    std::thread::spawn(move || run_drag(app, source, input));
    #[cfg(not(target_os = "windows"))]
    let _ = (app, source, input);
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_drag(app: AppHandle, source: String, input: DragInput) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut last_location = None;
    loop {
        let alive = state().lock().ok().is_some_and(|state| {
            state
                .active
                .as_ref()
                .is_some_and(|(label, id)| label == &source && id == &input.drag_id)
        });
        let mut point = POINT { x: 0, y: 0 };
        let cursor_ok = unsafe { GetCursorPos(&mut point) } != 0;
        let escape = unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } < 0;
        let pressed = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0;
        let cancelled = !alive || !cursor_ok || escape || app.get_webview_window(&source).is_none();
        let (target_window_label, placement) = if cancelled {
            (None, None)
        } else {
            hit_test(&app, &input, point.x, point.y)
        };
        let phase = if cancelled {
            "cancel"
        } else if !pressed {
            "drop"
        } else {
            "move"
        };
        let location = (
            point.x,
            point.y,
            target_window_label.clone(),
            serde_json::to_string(&placement).unwrap_or_default(),
        );
        if phase != "move" || last_location.as_ref() != Some(&location) {
            let event = DragEvent {
                input: input.clone(),
                phase: phase.into(),
                source_window_label: source.clone(),
                target_window_label,
                placement,
                screen_position: ScreenPoint {
                    x: point.x,
                    y: point.y,
                },
                // 拖影使用源 WebView 的逻辑坐标，捕获丢失后也能隐藏或继续跟随。
                source_client_position: app.get_webview_window(&source).and_then(|window| {
                    let origin = window.inner_position().ok()?;
                    let scale = window.scale_factor().ok()?;
                    Some(tauri::LogicalPosition::new(
                        (point.x - origin.x) as f64 / scale,
                        (point.y - origin.y) as f64 / scale,
                    ))
                }),
            };
            for (label, _) in app.webview_windows() {
                if super::external_open::is_document_window_label(&label) {
                    let _ = app.emit_to(label, "nomo://native-tab-drag", event.clone());
                }
            }
            last_location = Some(location);
        }
        if phase != "move" {
            if let Ok(mut state) = state().lock() {
                if state
                    .active
                    .as_ref()
                    .is_some_and(|(label, id)| label == &source && id == &input.drag_id)
                {
                    state.active = None;
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

#[cfg(target_os = "windows")]
fn hit_test(
    app: &AppHandle,
    input: &DragInput,
    x: i32,
    y: i32,
) -> (Option<String>, Option<DropPlacement>) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT};
    // 根 HWND 排除被其他程序遮挡的文档窗口，窗口枚举顺序不能代表真实 Z-order。
    let top = unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) };
    if top.is_null() {
        return (None, None);
    }
    let Ok(state) = state().lock() else {
        return (None, None);
    };
    for (label, zones) in &state.windows {
        if !zones.ready
            || !super::external_open::is_document_window_label(label)
            || super::state::is_markdown_mini_mode_window(label)
        {
            continue;
        }
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(true) {
            continue;
        }
        let Ok(hwnd) = window.hwnd() else {
            continue;
        };
        if hwnd.0 != top {
            continue;
        }
        let Ok(origin) = window.inner_position() else {
            continue;
        };
        let Ok(scale) = window.scale_factor() else {
            continue;
        };
        let client_x = (x - origin.x) as f64 / scale;
        let client_y = (y - origin.y) as f64 / scale;
        for zone in &zones.zones {
            if matches!(&zone.placement, DropPlacement::Combine { .. }) && !input.can_combine {
                continue;
            }
            if client_x >= zone.x
                && client_x <= zone.x + zone.width
                && client_y >= zone.y
                && client_y <= zone.y + zone.height
            {
                return (Some(label.clone()), Some(zone.placement.clone()));
            }
        }
        // 文档窗口内未命中标签栏时取消，不把正文区误判为创建新窗口。
        return (Some(label.clone()), None);
    }
    (None, None)
}
