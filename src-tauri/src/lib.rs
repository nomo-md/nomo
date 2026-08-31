mod app_logger;
mod config;
mod export;
mod external_link;
mod file_system;
mod i18n;
mod models;
mod pdf_outline;
mod software_update;
mod text_document;
mod window;
mod windows_package;

#[cfg(target_os = "windows")]
mod export_windows;

#[cfg(target_os = "macos")]
mod export_macos;

use tauri::{Emitter, Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 先生成 context 以获取应用 identifier，便于启动阶段读取配置
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();

    // 根据配置提前初始化日志开关，确保渲染模式等启动日志能被记录
    crate::app_logger::init(&context.config().identifier);
    let startup_timer = std::time::Instant::now();

    // 步骤1：根据用户偏好设置 WebView2 渲染模式（必须在 WebView2 环境初始化前设置环境变量）
    #[cfg(target_os = "windows")]
    {
        if crate::config::is_software_render_mode(&context.config().identifier) {
            unsafe {
                std::env::set_var(
                    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                    "--disable-gpu --disable-gpu-compositing",
                );
            }
            crate::app_logger::info("App", "已启用软件渲染模式");
        } else {
            crate::app_logger::info("App", "已启用硬件加速渲染模式");
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            crate::app_logger::info("App", "收到单实例启动参数");
            let targets = crate::window::external_open::collect_external_open_targets_from_args(
                args,
                Some(std::path::PathBuf::from(cwd)),
            );
            let should_restore_window =
                targets.markdown_paths.is_empty() && targets.folder_paths.is_empty();
            let _ = crate::window::external_open::route_external_open_targets(app, targets);
            if should_restore_window {
                crate::window::tray::show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| match event {
            _ if crate::export::is_pdf_export_window_label(window.label()) => {}
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                crate::window::state::persist_window_state_after_geometry_change(window);
            }
            WindowEvent::ThemeChanged(theme) => {
                crate::window::commands::broadcast_system_theme_changed(window.app_handle(), *theme);
            }
            WindowEvent::Focused(true) => {
                let label = window.label();
                if crate::window::external_open::is_document_window_label(label) {
                    crate::window::tray::record_last_active_window(window.app_handle(), label);
                }
            }
            WindowEvent::Destroyed => {
                let label = window.label();
                crate::app_logger::debug("Window", &format!("窗口销毁前持久化状态：{label}"));
                crate::window::state::persist_window_state_before_destroy(window);
                if label == crate::window::commands::SETTINGS_WINDOW_LABEL {
                    crate::window::commands::reset_settings_close_handler_ready();
                    crate::window::commands::clear_pending_settings_close_request();
                    crate::window::commands::forget_settings_owner();
                    match crate::window::commands::take_deferred_settings_action() {
                        Some(crate::window::commands::DeferredSettingsAction::CloseOwner(
                            owner_label,
                        )) => {
                            if let Err(error) =
                                crate::window::commands::resume_owner_close_after_settings(
                                    window.app_handle(),
                                    &owner_label,
                                )
                            {
                                crate::app_logger::warn(
                                    "Window",
                                    &format!(
                                        "偏好设置关闭后继续关闭 owner 失败：label={owner_label} error={error}"
                                    ),
                                );
                            }
                        }
                        Some(crate::window::commands::DeferredSettingsAction::ExitApp) => {
                            window.app_handle().exit(0);
                        }
                        None => {}
                    }
                }
                if crate::window::external_open::is_document_window_label(label) {
                    if let Some(registry) = window
                        .app_handle()
                        .try_state::<crate::window::open_targets::OpenTargetRegistry>()
                    {
                        registry.forget_window(label);
                    }
                    crate::window::state::forget_markdown_mini_mode_window(label);
                    crate::window::tray::forget_window(window.app_handle(), label);
                }
            }
            WindowEvent::CloseRequested { api, .. } => {
                let label = window.label();
                crate::app_logger::info("Window", &format!("收到窗口关闭请求：{label}"));
                let returns_from_markdown_mini =
                    crate::window::external_open::is_document_window_label(label)
                        && crate::window::state::is_markdown_mini_mode_window(label);
                if crate::window::commands::consume_next_close(label) {
                    if label == crate::window::commands::SETTINGS_WINDOW_LABEL {
                        crate::window::commands::reset_settings_close_handler_ready();
                        return;
                    }
                    if returns_from_markdown_mini {
                        return;
                    }
                    match crate::window::commands::request_settings_close_before_owner(
                        window.app_handle(),
                        label,
                    ) {
                        Ok(true) => {
                            api.prevent_close();
                            return;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            api.prevent_close();
                            crate::app_logger::warn(
                                "Settings",
                                &format!(
                                    "关闭 owner 前保存偏好设置失败，已取消关闭：label={label} error={error}"
                                ),
                            );
                            return;
                        }
                    }
                    return;
                }
                if label == crate::window::commands::SETTINGS_WINDOW_LABEL {
                    if crate::window::commands::settings_close_handler_ready() {
                        if let Some(request_id) =
                            crate::window::commands::begin_settings_close_request()
                        {
                            match window.emit("nomo://settings-request-close", request_id) {
                                Ok(()) => {
                                    api.prevent_close();
                                    crate::window::commands::schedule_settings_close_fallback(
                                        window.clone(),
                                        request_id,
                                    );
                                }
                                Err(error) => {
                                    crate::window::commands::clear_pending_settings_close_request();
                                    crate::app_logger::warn(
                                        "Settings",
                                        &format!(
                                            "通知前端保存设置失败，直接关闭偏好设置窗口：{error}"
                                        ),
                                    );
                                    crate::window::commands::reset_settings_close_handler_ready();
                                }
                            }
                        } else {
                            api.prevent_close();
                        }
                    }
                    return;
                }
                if !crate::window::external_open::is_document_window_label(label) {
                    return;
                }
                if returns_from_markdown_mini {
                    api.prevent_close();
                    let _ = window.emit("nomo://markdown-mini-request-return", ());
                    return;
                }

                api.prevent_close();
                crate::app_logger::info("Window", &format!("请求前端确认关闭：{label}"));
                let _ = window.emit(
                    "nomo://request-close-window",
                    crate::models::WindowLabelPayload {
                        window_label: label.to_string(),
                    },
                );
            }
            _ => {}
        })
        .setup(move |app| {
            use tauri::Manager;
            let setup_timer = std::time::Instant::now();
            crate::app_logger::info("App", "开始 Tauri setup");
            let config = crate::config::ConfigManager::load_or_default(app.handle())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            app.manage(config);
            app.manage(crate::window::open_targets::OpenTargetRegistry::default());
            let segmented_root = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?
                .join("segmented-documents");
            let segmented_manager =
                crate::text_document::DocumentSessionManager::new(segmented_root)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            app.manage(segmented_manager);

            if let Some(window) = app.get_webview_window("main") {
                crate::app_logger::info("Window", "初始化主窗口系统适配和菜单");
                crate::window::os::setup_window(&window).map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("初始化主窗口 Windows 无边框样式失败：{error}"),
                    )
                })?;
                crate::window::menu::install_window_menu(app.handle(), &window)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            }
            crate::app_logger::info("Tray", "安装应用托盘");
            crate::window::tray::install_app_tray(app.handle())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            crate::window::state::restore_window_state(app.handle(), "main");
            if let Some(window) = app.get_webview_window("main") {
                crate::app_logger::info("Window", "显示主窗口");
                window
                    .show()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
                // macOS 27 会在 Tauri setup 尚未返回、WebView 父视图仍在附着时拒绝设置
                // first responder，并在稍后的视图释放阶段触发 NSView 生命周期断言。
                // 启动时由 AppKit 自然激活首个可见窗口；后续显式唤起仍走平台聚焦逻辑。
                #[cfg(not(target_os = "macos"))]
                window
                    .set_focus()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            }
            let mut startup_targets =
                crate::window::external_open::collect_external_open_targets_from_startup_args();
            let early_open_paths =
                crate::window::external_open::take_early_external_open_paths()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            for path in early_open_paths {
                if !startup_targets.markdown_paths.contains(&path) {
                    startup_targets.markdown_paths.push(path);
                }
            }
            crate::app_logger::info(
                "App",
                &format!(
                    "启动待打开目标：files={} folders={}",
                    startup_targets.markdown_paths.len(),
                    startup_targets.folder_paths.len()
                ),
            );
            crate::window::external_open::persist_pending_external_open(
                app.handle(),
                "main",
                &startup_targets.markdown_paths,
            )
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            if let Some(folder_path) = startup_targets.folder_paths.first() {
                crate::window::external_open::persist_pending_external_folder_open(
                    app.handle(),
                    "main",
                    folder_path,
                )
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            }
            crate::app_logger::perf("App", "Tauri setup", setup_timer.elapsed());
            crate::app_logger::perf("App", "软件打开", startup_timer.elapsed());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::export::export_html,
            crate::export::export_pdf_from_html,
            crate::export::read_file_as_base64,
            crate::file_system::read_markdown_file,
            crate::file_system::write_markdown_file,
            crate::file_system::write_markdown_file_with_encoding,
            crate::text_document::commands::open_segmented_document,
            crate::text_document::commands::reload_segmented_session,
            crate::text_document::commands::read_segmented_window,
            crate::text_document::commands::apply_segmented_edits,
            crate::text_document::commands::undo_segmented_revision,
            crate::text_document::commands::redo_segmented_revision,
            crate::text_document::commands::flush_segmented_journal,
            crate::text_document::commands::save_segmented_revision,
            crate::text_document::commands::start_segmented_task,
            crate::text_document::commands::cancel_segmented_task,
            crate::text_document::commands::close_segmented_session,
            crate::text_document::commands::check_segmented_external_change,
            crate::text_document::commands::get_segmented_session_status,
            crate::file_system::install_sample_document,
            crate::file_system::stat_markdown_file,
            crate::config::commands::remember_recent_entry,
            crate::config::commands::list_recent_entries,
            crate::config::commands::clear_recent_entries,
            crate::config::commands::create_document_snapshot,
            crate::config::commands::list_document_snapshots,
            crate::config::commands::write_workspace_draft,
            crate::config::commands::read_workspace_draft,
            crate::config::commands::delete_workspace_draft,
            crate::config::commands::update_app_setting,
            crate::config::commands::update_app_settings,
            crate::config::commands::list_app_settings,
            crate::window::commands::update_window_state,
            crate::window::commands::refresh_window_menu,
            crate::window::commands::activate_document_window,
            crate::window::commands::report_window_title,
            crate::window::commands::refresh_interface_language_chrome,
            crate::window::commands::set_desktop_icon_theme,
            crate::window::commands::get_desktop_system_theme,
            crate::file_system::list_folder_markdown_files,
            crate::file_system::create_folder,
            crate::file_system::rename_file,
            crate::file_system::delete_file,
            crate::file_system::image_assets::import_image_asset,
            crate::file_system::image_assets::resolve_image_asset,
            crate::file_system::image_assets::delete_image_asset,
            crate::file_system::image_assets::upload_image_via_picgo_core,
            crate::file_system::image_assets::upload_image_via_picgo_server,
            crate::file_system::image_assets::test_picgo_connection,
            crate::window::commands::create_new_window,
            crate::window::open_targets::sync_window_open_targets,
            crate::window::open_targets::prepare_open_target_window,
            crate::window::open_targets::release_open_target_reservation,
            crate::window::commands::open_settings_window,
            crate::window::commands::mark_settings_close_handler_ready,
            crate::window::commands::cancel_settings_close_request,
            crate::window::commands::acknowledge_settings_close_request,
            crate::window::commands::enter_markdown_mini_mode,
            crate::window::commands::exit_markdown_mini_mode,
            crate::window::commands::set_markdown_mini_mode_pinned,
            crate::window::commands::close_window,
            crate::window::commands::hide_window_to_tray,
            crate::window::commands::request_exit_app,
            crate::window::commands::exit_app,
            crate::window::commands::get_markdown_file_association_status,
            crate::window::commands::register_markdown_file_association,
            crate::window::commands::unregister_markdown_file_association,
            crate::window::commands::get_windows_context_menu_status,
            crate::window::commands::register_windows_context_menu,
            crate::window::commands::unregister_windows_context_menu,
            crate::windows_package::get_legacy_installer_notice,
            crate::windows_package::open_windows_installed_apps,
            crate::windows_package::open_microsoft_store_product,
            crate::app_logger::log_message,
            crate::app_logger::set_logger_enabled,
            crate::app_logger::get_logger_enabled,
            crate::software_update::is_windows_installer_installation,
            crate::software_update::get_cached_software_update,
            crate::software_update::get_software_update_state,
            crate::software_update::check_software_update,
            crate::software_update::download_software_update,
            crate::software_update::install_software_update,
            crate::file_system::get_folder_tree,
            crate::file_system::list_folder_children,
            crate::file_system::check_paths_exist,
            crate::external_link::open_external_link,
            crate::external_link::open_local_attachment,
            crate::external_link::reveal_in_explorer
        ])
        .build(context)
        .expect("error while building Nomo")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            match _event {
                tauri::RunEvent::Opened { urls } => {
                    let paths =
                        crate::window::external_open::collect_markdown_paths_from_urls(urls);
                    crate::app_logger::info(
                        "ExternalOpen",
                        &format!("收到 macOS Opened 事件：files={}", paths.len()),
                    );
                    if _app.try_state::<crate::config::ConfigManager>().is_none() {
                        if let Err(error) =
                            crate::window::external_open::queue_early_external_open(paths)
                        {
                            crate::app_logger::error("ExternalOpen", &error);
                        } else {
                            crate::app_logger::info("ExternalOpen", "已暂存 setup 前文件打开请求");
                        }
                    } else if let Err(error) =
                        crate::window::external_open::route_external_open(_app, paths)
                    {
                        crate::app_logger::error("ExternalOpen", &error);
                    }
                }
                tauri::RunEvent::Reopen {
                    has_visible_windows,
                    ..
                } => {
                    crate::app_logger::info(
                        "Window",
                        &format!("收到 macOS Dock 重开事件：visible={has_visible_windows}"),
                    );
                    if !has_visible_windows {
                        crate::window::tray::show_main_window(_app);
                    }
                }
                _ => {}
            }
        });
}
