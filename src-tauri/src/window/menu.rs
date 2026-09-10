use std::path::Path;

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder},
    AppHandle, Emitter, Manager, Runtime, WebviewWindow,
};

use crate::i18n::{self, InterfaceLocale};

#[cfg(target_os = "macos")]
static APP_MENU_EVENT_INSTALLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_window_menu<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Result<(), String> {
    let menu = build_window_menu(app).map_err(|error| format!("构建菜单失败：{error}"))?;

    #[cfg(target_os = "macos")]
    {
        let _ = window;
        app.set_menu(menu)
            .map_err(|error| format!("设置 macOS 原生菜单失败：{error}"))?;
        install_app_menu_event(app);
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_menu(menu)
            .map_err(|error| format!("设置菜单失败：{error}"))?;
        window.on_menu_event(|window, event| {
            let command = event.id().as_ref().to_string();
            if command == "quit" {
                let _ = crate::window::commands::emit_exit_request(window.app_handle());
                return;
            }
            if command == "open-settings" {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::window::commands::open_settings_window_for_app(app).await;
                });
                return;
            }

            let _ = window.emit("nomo://menu-command", command);
        });
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn install_app_menu_event<R: Runtime>(app: &AppHandle<R>) {
    if APP_MENU_EVENT_INSTALLED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    app.on_menu_event(|app, event| {
        let command = event.id().as_ref().to_string();
        if command == "quit" {
            let _ = crate::window::commands::emit_exit_request(app);
            return;
        }
        if command == "open-settings" {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::window::commands::open_settings_window_for_app(app).await;
            });
            return;
        }

        if let Some(window) = focused_document_window(app) {
            let _ = window.emit("nomo://menu-command", command);
        }
    });
}

#[cfg(target_os = "macos")]
fn focused_document_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    for (label, window) in app.webview_windows() {
        if is_document_window_label(&label) && window.is_focused().unwrap_or(false) {
            return Some(window);
        }
    }

    app.get_webview_window("main")
}

#[cfg(target_os = "macos")]
fn is_document_window_label(label: &str) -> bool {
    label == "main" || (label.starts_with("window-") && label != "window-settings")
}

pub(crate) fn build_window_menu<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri::menu::Menu<R>, String> {
    let file_menu = build_file_menu(app)?;
    let edit_menu = build_edit_menu(app)?;
    let paragraph_menu = build_paragraph_menu(app)?;
    let format_menu = build_format_menu(app)?;
    let view_menu = build_view_menu(app)?;
    let settings_menu = build_settings_menu(app)?;

    MenuBuilder::new(app)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&paragraph_menu)
        .item(&format_menu)
        .item(&view_menu)
        .item(&settings_menu)
        .build()
        .map_err(|e| e.to_string())
}

fn build_file_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let recent_entries = crate::config::commands::query_recent_entries(app).unwrap_or_default();
    let locale = i18n::effective_locale(app);

    let mut file_menu_builder = SubmenuBuilder::new(app, tr(locale, "menu_file"))
        .item(&menu_item(
            app,
            "new-file",
            tr(locale, "menu_new"),
            Some("CmdOrCtrl+N"),
        )?)
        .item(&menu_item(
            app,
            "new-window",
            tr(locale, "menu_new_window"),
            Some("CmdOrCtrl+Shift+N"),
        )?)
        .item(&menu_item(
            app,
            "open-file",
            tr(locale, "menu_open"),
            Some("CmdOrCtrl+O"),
        )?)
        .item(&menu_item(
            app,
            "open-directory",
            tr(locale, "menu_open_folder"),
            Some("CmdOrCtrl+Shift+O"),
        )?);

    if !recent_entries.is_empty() {
        let mut recent_submenu_builder = SubmenuBuilder::new(app, tr(locale, "menu_open_recent"));
        for entry in recent_entries.iter().take(8) {
            let label = entry.title.clone().unwrap_or_else(|| {
                Path::new(&entry.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(tr(locale, "menu_untitled"))
                    .to_string()
            });
            let item_id = format!("open-recent:{}:{}", entry.entry_type, entry.path);
            recent_submenu_builder =
                recent_submenu_builder.item(&menu_item(app, item_id, label, None)?);
        }
        let recent_submenu = recent_submenu_builder.build().map_err(|e| e.to_string())?;
        file_menu_builder = file_menu_builder.item(&recent_submenu);
    } else {
        let placeholder = MenuItemBuilder::with_id("no-recent", tr(locale, "menu_no_recent"))
            .enabled(false)
            .build(app)
            .map_err(|e| e.to_string())?;
        file_menu_builder = file_menu_builder.item(&placeholder);
    }

    file_menu_builder = file_menu_builder
        .separator()
        .item(&menu_item(
            app,
            "save-file",
            tr(locale, "menu_save"),
            Some("CmdOrCtrl+S"),
        )?)
        .item(&menu_item(
            app,
            "save-file-as",
            tr(locale, "menu_save_as"),
            Some("CmdOrCtrl+Shift+S"),
        )?)
        .separator();

    let export_submenu = SubmenuBuilder::new(app, tr(locale, "menu_export"))
        .item(&menu_item(
            app,
            "export-html",
            tr(locale, "menu_export_html"),
            None,
        )?)
        .item(&menu_item(
            app,
            "export-pdf",
            tr(locale, "menu_export_pdf"),
            None,
        )?)
        .build()
        .map_err(|e| e.to_string())?;

    file_menu_builder
        .item(&export_submenu)
        .separator()
        .item(&menu_item(
            app,
            "close-current-file",
            tr(locale, "menu_close_file"),
            Some("CmdOrCtrl+W"),
        )?)
        .item(&menu_item(
            app,
            "close-current-window",
            tr(locale, "menu_close_window"),
            close_window_accelerator(),
        )?)
        .separator()
        .item(&menu_item(
            app,
            "quit",
            tr(locale, "menu_quit"),
            Some(quit_accelerator()),
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn build_edit_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    SubmenuBuilder::new(app, tr(locale, "menu_edit"))
        .item(&menu_item(
            app,
            "undo",
            tr(locale, "menu_undo"),
            Some("CmdOrCtrl+Z"),
        )?)
        .item(&menu_item(
            app,
            "redo",
            tr(locale, "menu_redo"),
            redo_accelerator(),
        )?)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()
        .map_err(|e| e.to_string())
}

fn build_paragraph_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    let heading_menu = SubmenuBuilder::new(app, tr(locale, "menu_heading"))
        .item(&menu_item(
            app,
            "set-heading-1",
            tr(locale, "menu_heading_1"),
            Some("CmdOrCtrl+1"),
        )?)
        .item(&menu_item(
            app,
            "set-heading-2",
            tr(locale, "menu_heading_2"),
            Some("CmdOrCtrl+2"),
        )?)
        .item(&menu_item(
            app,
            "set-heading-3",
            tr(locale, "menu_heading_3"),
            Some("CmdOrCtrl+3"),
        )?)
        .item(&menu_item(
            app,
            "set-heading-4",
            tr(locale, "menu_heading_4"),
            Some("CmdOrCtrl+4"),
        )?)
        .item(&menu_item(
            app,
            "set-heading-5",
            tr(locale, "menu_heading_5"),
            Some("CmdOrCtrl+5"),
        )?)
        .item(&menu_item(
            app,
            "set-heading-6",
            tr(locale, "menu_heading_6"),
            Some("CmdOrCtrl+6"),
        )?)
        .build()
        .map_err(|e| e.to_string())?;

    let diagram_menu = build_diagram_menu(app)?;

    SubmenuBuilder::new(app, tr(locale, "menu_paragraph"))
        .item(&heading_menu)
        .item(&menu_item(
            app,
            "set-paragraph",
            tr(locale, "menu_paragraph"),
            Some("CmdOrCtrl+0"),
        )?)
        .item(&menu_item(
            app,
            "menu-heading-up",
            tr(locale, "menu_lift_heading"),
            Some("CmdOrCtrl+="),
        )?)
        .item(&menu_item(
            app,
            "menu-heading-down",
            tr(locale, "menu_sink_heading"),
            Some("CmdOrCtrl+-"),
        )?)
        .separator()
        .item(&menu_item(
            app,
            "insert-table",
            tr(locale, "menu_table"),
            Some("CmdOrCtrl+Shift+T"),
        )?)
        .item(&menu_item(
            app,
            "insert-code-block",
            tr(locale, "menu_code_block"),
            Some("CmdOrCtrl+Shift+K"),
        )?)
        .item(&menu_item(
            app,
            "insert-math-block",
            tr(locale, "menu_math_block"),
            Some("CmdOrCtrl+Shift+M"),
        )?)
        .separator()
        .item(&menu_item(
            app,
            "toggle-blockquote",
            tr(locale, "menu_blockquote"),
            Some("CmdOrCtrl+Shift+Q"),
        )?)
        .item(&menu_item(
            app,
            "insert-callout",
            tr(locale, "menu_callout"),
            Some("CmdOrCtrl+Shift+A"),
        )?)
        .item(&menu_item(
            app,
            "menu-comment-block",
            tr(locale, "menu_comment_block"),
            None,
        )?)
        .item(&menu_item(
            app,
            "toggle-ordered-list",
            tr(locale, "menu_ordered_list"),
            Some("CmdOrCtrl+Shift+["),
        )?)
        .item(&menu_item(
            app,
            "toggle-bullet-list",
            tr(locale, "menu_bullet_list"),
            Some("CmdOrCtrl+Shift+]"),
        )?)
        .item(&menu_item(
            app,
            "toggle-task-list",
            tr(locale, "menu_task_list"),
            Some("CmdOrCtrl+Shift+X"),
        )?)
        .separator()
        .item(&menu_item(
            app,
            "menu-insert-paragraph-before",
            tr(locale, "menu_insert_before"),
            Some("CmdOrCtrl+Shift+Enter"),
        )?)
        .item(&menu_item(
            app,
            "menu-insert-paragraph-after",
            tr(locale, "menu_insert_after"),
            Some("CmdOrCtrl+Enter"),
        )?)
        .separator()
        .item(&diagram_menu)
        .item(&menu_item(
            app,
            "menu-footnote",
            tr(locale, "menu_footnote"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-horizontal-rule",
            tr(locale, "menu_horizontal_rule"),
            Some("CmdOrCtrl+Shift+H"),
        )?)
        .item(&menu_item(
            app,
            "menu-content-directory",
            tr(locale, "menu_toc"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-yaml-front-matter",
            tr(locale, "menu_front_matter"),
            None,
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn build_format_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    SubmenuBuilder::new(app, tr(locale, "menu_format"))
        .item(&menu_item(
            app,
            "toggle-bold",
            tr(locale, "menu_bold"),
            Some("CmdOrCtrl+B"),
        )?)
        .item(&menu_item(
            app,
            "toggle-italic",
            tr(locale, "menu_italic"),
            Some("CmdOrCtrl+I"),
        )?)
        .item(&menu_item(
            app,
            "toggle-underline",
            tr(locale, "menu_underline"),
            Some("CmdOrCtrl+U"),
        )?)
        .item(&menu_item(
            app,
            "toggle-inline-code",
            tr(locale, "menu_inline_code"),
            Some("CmdOrCtrl+`"),
        )?)
        .item(&menu_item(
            app,
            "menu-inline-math",
            tr(locale, "menu_inline_math"),
            None,
        )?)
        .separator()
        .item(&menu_item(
            app,
            "toggle-strikethrough",
            tr(locale, "menu_strike"),
            Some("Alt+Shift+5"),
        )?)
        .item(&menu_item(
            app,
            "menu-highlight",
            tr(locale, "menu_highlight"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-comment",
            tr(locale, "menu_comment"),
            None,
        )?)
        .separator()
        .item(&menu_item(
            app,
            "menu-link",
            tr(locale, "menu_link"),
            Some("CmdOrCtrl+K"),
        )?)
        .item(&menu_item(
            app,
            "menu-image",
            tr(locale, "menu_image"),
            None,
        )?)
        .separator()
        .item(&menu_item(
            app,
            "menu-clear-format",
            tr(locale, "menu_clear_format"),
            Some("CmdOrCtrl+\\"),
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn build_view_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    let toolbar_accelerator = toolbar_accelerator(app);
    SubmenuBuilder::new(app, tr(locale, "menu_view"))
        .item(&menu_item(
            app,
            "toggle-source",
            tr(locale, "menu_toggle_source"),
            Some("CmdOrCtrl+E"),
        )?)
        .item(&menu_item(
            app,
            "toggle-outline",
            tr(locale, "menu_toggle_outline"),
            None,
        )?)
        .item(&menu_item(
            app,
            "toggle-toolbar",
            tr(locale, "menu_toggle_toolbar"),
            Some(toolbar_accelerator.as_str()),
        )?)
        .item(&menu_item(
            app,
            "toggle-theme",
            tr(locale, "menu_toggle_theme"),
            Some("CmdOrCtrl+Shift+L"),
        )?)
        .item(&menu_item(
            app,
            "toggle-focus",
            tr(locale, "menu_toggle_explorer"),
            Some("CmdOrCtrl+Shift+F"),
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn toolbar_accelerator<R: Runtime>(app: &AppHandle<R>) -> String {
    crate::config::commands::get_setting_value(app, "shortcutPreferences")
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| {
            value
                .get("toggle-toolbar")
                .and_then(serde_json::Value::as_str)
                .and_then(to_native_accelerator)
        })
        .unwrap_or_else(|| "CmdOrCtrl+Shift+B".to_string())
}

fn to_native_accelerator(shortcut: &str) -> Option<String> {
    let mut parts = shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key = parts.pop()?;
    let normalized_key = match key.to_ascii_lowercase().as_str() {
        "space" => "Space".to_string(),
        "\\" => "Backslash".to_string(),
        "[" => "BracketLeft".to_string(),
        "]" => "BracketRight".to_string(),
        "`" => "Backquote".to_string(),
        "-" => "Minus".to_string(),
        value
            if value.len() == 1
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()) =>
        {
            value.to_ascii_uppercase()
        }
        _ => return None,
    };

    let mut normalized = Vec::new();
    for modifier in parts {
        let value = if modifier.eq_ignore_ascii_case("ctrl") {
            "CmdOrCtrl"
        } else if modifier.eq_ignore_ascii_case("shift") {
            "Shift"
        } else if modifier.eq_ignore_ascii_case("alt") {
            "Alt"
        } else {
            return None;
        };
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    normalized.push(normalized_key.as_str());
    Some(normalized.join("+"))
}

fn build_settings_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    SubmenuBuilder::new(app, tr(locale, "menu_settings"))
        .item(&menu_item(
            app,
            "open-settings",
            tr(locale, "menu_preferences"),
            Some("CmdOrCtrl+,"),
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn build_diagram_menu<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::menu::Submenu<R>, String> {
    let locale = i18n::effective_locale(app);
    SubmenuBuilder::new(app, tr(locale, "menu_chart"))
        .item(&menu_item(
            app,
            "menu-chart",
            tr(locale, "menu_chart_blank"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:flowchart",
            tr(locale, "menu_chart_flowchart"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:sequenceDiagram",
            tr(locale, "menu_chart_sequence"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:classDiagram",
            tr(locale, "menu_chart_class"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:stateDiagram",
            tr(locale, "menu_chart_state"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:pie",
            tr(locale, "menu_chart_pie"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:gantt",
            tr(locale, "menu_chart_gantt"),
            None,
        )?)
        .item(&menu_item(
            app,
            "menu-chart:erDiagram",
            tr(locale, "menu_chart_er"),
            None,
        )?)
        .build()
        .map_err(|e| e.to_string())
}

fn menu_item<R: Runtime>(
    app: &AppHandle<R>,
    id: impl Into<String>,
    text: impl Into<String>,
    accelerator: Option<&str>,
) -> Result<MenuItem<R>, String> {
    let mut builder = MenuItemBuilder::with_id(id.into(), text.into());
    if let Some(accelerator) = accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app).map_err(|e| e.to_string())
}

fn tr(locale: InterfaceLocale, key: &str) -> &'static str {
    i18n::text(locale, key)
}

#[cfg(target_os = "macos")]
fn quit_accelerator() -> &'static str {
    "Cmd+Q"
}

#[cfg(not(target_os = "macos"))]
fn quit_accelerator() -> &'static str {
    "Alt+F4"
}

#[cfg(target_os = "macos")]
fn close_window_accelerator() -> Option<&'static str> {
    None
}

#[cfg(not(target_os = "macos"))]
fn close_window_accelerator() -> Option<&'static str> {
    Some("Alt+F4")
}

#[cfg(target_os = "macos")]
fn redo_accelerator() -> Option<&'static str> {
    Some("CmdOrCtrl+Shift+Z")
}

#[cfg(not(target_os = "macos"))]
fn redo_accelerator() -> Option<&'static str> {
    Some("CmdOrCtrl+Y")
}
