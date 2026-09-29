use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{
    image::Image,
    menu::{Menu, MenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow,
};

use crate::i18n;

const TRAY_ID: &str = "nomo-main-tray";
const TRAY_OPEN_ID: &str = "tray-open-main-window";
const TRAY_EXIT_ID: &str = "tray-exit-app";
const TRAY_WINDOW_PREFIX: &str = "tray-window:";
const TRAY_DARK_ACTIVE_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/tray/nomo-tray-dark-active-24-preview.png");
const TRAY_DARK_INACTIVE_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/tray/nomo-tray-dark-inactive-24-preview.png");
const TRAY_LIGHT_ACTIVE_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/tray/nomo-tray-light-active-24-preview.png");
const TRAY_LIGHT_INACTIVE_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/tray/nomo-tray-light-inactive-24-preview.png");
#[cfg(target_os = "macos")]
const WINDOW_LIGHT_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/macos/nomo-app-light-256.png");
#[cfg(target_os = "macos")]
const WINDOW_DARK_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/nomo/macos/nomo-app-dark-256.png");

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrayTheme {
    Light,
    Dark,
}

#[derive(Clone, Copy)]
struct TrayVisualState {
    active: bool,
    theme: TrayTheme,
    applied_icon_theme: Option<TrayTheme>,
}

struct DocumentTrayWindow<R: Runtime> {
    label: String,
    window: WebviewWindow<R>,
    visible: bool,
}

impl Default for TrayVisualState {
    fn default() -> Self {
        Self {
            active: true,
            theme: TrayTheme::Light,
            applied_icon_theme: None,
        }
    }
}

static TRAY_STATE: OnceLock<Mutex<TrayVisualState>> = OnceLock::new();
static LAST_ACTIVE_WINDOW: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static WINDOW_TITLES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

pub(crate) fn install_app_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if app.tray_by_id(TRAY_ID).is_some() {
        crate::app_logger::debug("Tray", "托盘已存在，跳过安装");
        return Ok(());
    }

    let menu = build_tray_menu(app)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Nomo")
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                TRAY_OPEN_ID => show_main_window(app),
                TRAY_EXIT_ID => {
                    let _ = crate::window::commands::emit_exit_request(app);
                }
                _ if id.starts_with(TRAY_WINDOW_PREFIX) => {
                    let label = &id[TRAY_WINDOW_PREFIX.len()..];
                    show_window_by_label(app, label);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => show_main_window(tray.app_handle()),
            _ => {}
        });

    if let Ok(icon) = tray_state_icon(current_tray_state()) {
        builder = builder.icon(icon);
    } else if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder
        .build(app)
        .map_err(|error| format!("创建托盘图标失败：{error}"))?;
    crate::app_logger::info("Tray", "托盘图标创建完成");
    Ok(())
}

pub(crate) fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        crate::app_logger::info("Tray", "刷新托盘菜单");
        let menu = build_tray_menu(app)?;
        tray.set_menu(Some(menu))
            .map_err(|error| format!("刷新托盘菜单失败：{error}"))?;
    }
    Ok(())
}

fn build_tray_menu<R: Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, String> {
    let windows = collect_document_windows(app);
    let mut builder = MenuBuilder::new(app).text(TRAY_OPEN_ID, i18n::app_text(app, "tray_open"));

    if !windows.is_empty() {
        builder = builder.separator();
        for info in &windows {
            let prefix = if info.visible { "●" } else { "○" };
            let menu_id = format!("{TRAY_WINDOW_PREFIX}{}", info.label);
            let title = window_display_title(&info.label, &info.window);
            builder = builder.text(menu_id, format!("{prefix} {title}"));
        }
    }

    builder
        .separator()
        .text(TRAY_EXIT_ID, i18n::app_text(app, "tray_exit"))
        .build()
        .map_err(|error| format!("构建托盘菜单失败：{error}"))
}

pub(crate) fn record_last_active_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    crate::app_logger::debug("Tray", &format!("记录最后活跃窗口：{label}"));
    let _ = last_active_window().lock().map(|mut active| {
        *active = Some(label.to_string());
    });
    let _ = refresh_tray_menu(app);
}

pub(crate) fn last_active_document_window_label() -> Option<String> {
    last_active_window()
        .lock()
        .ok()
        .and_then(|active| active.clone())
}

pub(crate) fn record_window_title<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    title: &str,
) -> Result<(), String> {
    window_titles()
        .lock()
        .map(|mut titles| {
            titles.insert(label.to_string(), title.to_string());
        })
        .map_err(|error| format!("记录窗口标题失败：{error}"))?;
    refresh_tray_menu(app)
}

pub(crate) fn forget_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    crate::app_logger::debug("Tray", &format!("清理托盘窗口记录：{label}"));
    let _ = window_titles().lock().map(|mut titles| {
        titles.remove(label);
    });
    let _ = last_active_window().lock().map(|mut active| {
        if active.as_deref() == Some(label) {
            *active = None;
        }
    });
    let _ = refresh_tray_menu(app);
}

pub(crate) fn set_tray_active<R: Runtime>(app: &AppHandle<R>, active: bool) {
    crate::app_logger::debug("Tray", &format!("设置托盘激活状态：{active}"));
    let Ok(state) = update_tray_state(|state| state.active = active) else {
        return;
    };
    let _ = apply_tray_icon(app, state);
}

pub(crate) fn sync_tray_active_with_window_visibility<R: Runtime>(app: &AppHandle<R>) {
    let active = collect_document_windows(app)
        .iter()
        .any(|info| info.visible);
    set_tray_active(app, active);
}

pub(crate) fn set_desktop_icon_theme<R: Runtime>(
    app: &AppHandle<R>,
    theme: &str,
) -> Result<(), String> {
    let next_theme = match theme {
        "light" => TrayTheme::Light,
        "dark" => TrayTheme::Dark,
        _ => return Err(format!("未知托盘主题：{theme}")),
    };

    let (state, should_apply) = prepare_desktop_icon_theme(next_theme)?;
    if !should_apply {
        crate::app_logger::debug("Tray", "桌面图标主题未变化，跳过重复同步");
        return Ok(());
    }

    if let Err(error) =
        apply_window_icons(app, next_theme).and_then(|_| apply_tray_icon(app, state))
    {
        rollback_desktop_icon_theme(next_theme);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    crate::app_logger::info("Tray", "从托盘打开文档窗口");
    let windows = collect_document_windows(app);
    let has_visible_window = windows.iter().any(|info| info.visible);
    let target = if has_visible_window {
        choose_last_active_window(windows.iter().filter(|info| info.visible))
    } else {
        choose_last_active_window(windows.iter())
    };

    if let Some(window) = target {
        show_document_window(app, &window);
    }
}

fn show_window_by_label<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    if !crate::window::external_open::is_document_window_label(window.label()) {
        return;
    }
    show_document_window(app, &window);
}

fn show_document_window<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    crate::window::os::bring_window_to_front(window);
    set_tray_active(app, true);
    let _ = refresh_tray_menu(app);
}

fn collect_document_windows<R: Runtime>(app: &AppHandle<R>) -> Vec<DocumentTrayWindow<R>> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _window)| crate::window::external_open::is_document_window_label(label))
        .map(|(label, window)| {
            let visible = window.is_visible().unwrap_or(false);
            DocumentTrayWindow {
                label,
                window,
                visible,
            }
        })
        .collect()
}

fn choose_last_active_window<'a, R: Runtime>(
    windows: impl Iterator<Item = &'a DocumentTrayWindow<R>>,
) -> Option<WebviewWindow<R>> {
    let windows = windows.collect::<Vec<_>>();
    let active_label = current_last_active_window();
    if let Some(active_label) = active_label {
        if let Some(info) = windows.iter().find(|info| info.label == active_label) {
            return Some(info.window.clone());
        }
    }

    windows.first().map(|info| info.window.clone())
}

fn window_display_title<R: Runtime>(label: &str, window: &WebviewWindow<R>) -> String {
    if let Some(title) = window_titles()
        .lock()
        .ok()
        .and_then(|titles| titles.get(label).cloned())
        .filter(|title| !title.trim().is_empty())
    {
        return title;
    }

    window
        .title()
        .ok()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| label.to_string())
}

fn tray_state() -> &'static Mutex<TrayVisualState> {
    TRAY_STATE.get_or_init(|| Mutex::new(TrayVisualState::default()))
}

fn last_active_window() -> &'static Mutex<Option<String>> {
    LAST_ACTIVE_WINDOW.get_or_init(|| Mutex::new(None))
}

fn current_last_active_window() -> Option<String> {
    last_active_window()
        .lock()
        .ok()
        .and_then(|active| active.clone())
}

fn window_titles() -> &'static Mutex<HashMap<String, String>> {
    WINDOW_TITLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_tray_state() -> TrayVisualState {
    tray_state()
        .lock()
        .map(|state| *state)
        .unwrap_or_else(|_| TrayVisualState::default())
}

fn update_tray_state(update: impl FnOnce(&mut TrayVisualState)) -> Result<TrayVisualState, String> {
    let mut state = tray_state()
        .lock()
        .map_err(|error| format!("读取 Nomo 托盘状态失败：{error}"))?;
    update(&mut state);
    Ok(*state)
}

fn prepare_desktop_icon_theme(next_theme: TrayTheme) -> Result<(TrayVisualState, bool), String> {
    let mut state = tray_state()
        .lock()
        .map_err(|error| format!("读取 Nomo 托盘状态失败：{error}"))?;
    let should_apply = stage_desktop_icon_theme(&mut state, next_theme);
    Ok((*state, should_apply))
}

fn stage_desktop_icon_theme(state: &mut TrayVisualState, next_theme: TrayTheme) -> bool {
    state.theme = next_theme;
    let should_apply = state.applied_icon_theme != Some(next_theme);
    if should_apply {
        state.applied_icon_theme = Some(next_theme);
    }
    should_apply
}

fn rollback_desktop_icon_theme(failed_theme: TrayTheme) {
    let _ = tray_state().lock().map(|mut state| {
        if state.applied_icon_theme == Some(failed_theme) {
            state.applied_icon_theme = None;
        }
    });
}

fn apply_tray_icon<R: Runtime>(app: &AppHandle<R>, state: TrayVisualState) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let icon = tray_state_icon(state)?;
    tray.set_icon(Some(icon))
        .map_err(|error| format!("设置 Nomo 托盘图标失败：{error}"))
}

#[cfg(target_os = "macos")]
fn apply_window_icons<R: Runtime>(app: &AppHandle<R>, theme: TrayTheme) -> Result<(), String> {
    let icon = window_theme_icon(theme)?;
    for (_label, window) in app.webview_windows() {
        window
            .set_icon(icon.clone())
            .map_err(|error| format!("设置 Nomo 窗口图标失败：{error}"))?;
    }
    apply_dock_icon(app, theme)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_dock_icon<R: Runtime>(app: &AppHandle<R>, theme: TrayTheme) -> Result<(), String> {
    if objc2::MainThreadMarker::new().is_some() {
        return apply_dock_icon_on_main_thread(theme);
    }

    app.run_on_main_thread(move || {
        if let Err(error) = apply_dock_icon_on_main_thread(theme) {
            crate::app_logger::error("Tray", &error);
        }
    })
    .map_err(|error| format!("同步 Nomo Dock 图标失败：{error}"))
}

#[cfg(target_os = "macos")]
fn apply_dock_icon_on_main_thread(theme: TrayTheme) -> Result<(), String> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let bytes = window_theme_icon_bytes(theme);
    let mtm =
        MainThreadMarker::new().ok_or_else(|| "Nomo Dock 图标必须在主线程设置".to_string())?;
    let app = NSApplication::sharedApplication(mtm);
    let data = NSData::with_bytes(bytes);
    let icon = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "读取 Nomo Dock 图标失败".to_string())?;
    unsafe { app.setApplicationIconImage(Some(&icon)) };
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_window_icons<R: Runtime>(_app: &AppHandle<R>, _theme: TrayTheme) -> Result<(), String> {
    // 其他平台保留打包图标，避免 macOS 留白和主题切换影响 Windows 任务栏。
    // 托盘图标仍由 apply_tray_icon 独立跟随主题更新。
    Ok(())
}

fn tray_state_icon(state: TrayVisualState) -> Result<Image<'static>, String> {
    let bytes = match (state.theme, state.active) {
        (TrayTheme::Light, true) => TRAY_LIGHT_ACTIVE_ICON_BYTES,
        (TrayTheme::Light, false) => TRAY_LIGHT_INACTIVE_ICON_BYTES,
        (TrayTheme::Dark, true) => TRAY_DARK_ACTIVE_ICON_BYTES,
        (TrayTheme::Dark, false) => TRAY_DARK_INACTIVE_ICON_BYTES,
    };

    Image::from_bytes(bytes)
        .map(Image::to_owned)
        .map_err(|error| format!("读取 Nomo 托盘图标失败：{error}"))
}

#[cfg(target_os = "macos")]
fn window_theme_icon(theme: TrayTheme) -> Result<Image<'static>, String> {
    let bytes = window_theme_icon_bytes(theme);

    Image::from_bytes(bytes)
        .map(Image::to_owned)
        .map_err(|error| format!("读取 Nomo 窗口图标失败：{error}"))
}

#[cfg(target_os = "macos")]
fn window_theme_icon_bytes(theme: TrayTheme) -> &'static [u8] {
    match theme {
        TrayTheme::Light => WINDOW_LIGHT_ICON_BYTES,
        TrayTheme::Dark => WINDOW_DARK_ICON_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::{stage_desktop_icon_theme, TrayTheme, TrayVisualState};

    #[test]
    fn skips_reapplying_the_same_desktop_icon_theme() {
        let mut state = TrayVisualState::default();

        assert!(stage_desktop_icon_theme(&mut state, TrayTheme::Light));
        assert!(!stage_desktop_icon_theme(&mut state, TrayTheme::Light));
        assert!(stage_desktop_icon_theme(&mut state, TrayTheme::Dark));
        assert!(!stage_desktop_icon_theme(&mut state, TrayTheme::Dark));
    }
}
