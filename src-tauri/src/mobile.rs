use tauri::Manager;

#[tauri::command]
fn get_mobile_system_theme() -> &'static str {
    "light"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    crate::app_logger::init(&context.config().identifier);

    tauri::Builder::default()
        .manage(crate::mobile_search::MobileSearchState::default())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let config = crate::config::ConfigManager::load_or_default(app.handle())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
            #[cfg(target_os = "android")]
            {
                let cache = app.path().app_cache_dir()?.join(crate::mobile_imports::INCOMING_DIR);
                let durable = app.path().app_data_dir()?.join(crate::mobile_imports::INCOMING_DIR);
                if !matches!(crate::mobile_imports::migrate_legacy(&config, &cache, &durable), Ok(0)) {
                    crate::app_logger::warn("ExternalOpen", "部分旧导入文档未能迁移，已保留原文件与引用");
                    use tauri_plugin_dialog::DialogExt;
                    app.dialog().message("部分旧文档未能迁移。原文件仍保留，请勿清理缓存；检查剩余空间后重启重试。").title("Nomo").show(|_| {});
                }
            }
            app.manage(config);

            let segmented_root = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?
                .join("segmented-documents");
            app.manage(crate::text_document::DocumentSessionManager::new(
                segmented_root,
            )?);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::mobile_search::start_mobile_document_search,
            crate::mobile_search::cancel_mobile_document_search,
            get_mobile_system_theme,
            crate::file_system::read_markdown_file,
            crate::file_system::write_markdown_file,
            crate::file_system::write_markdown_file_with_encoding,
            crate::file_system::install_sample_document,
            crate::file_system::stat_markdown_file,
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
            crate::app_logger::log_message,
            crate::app_logger::set_logger_enabled,
            crate::app_logger::get_logger_enabled,
        ])
        .build(context)
        .expect("error while building Nomo")
        .run(|_app, _event| {
            // 微信等外部应用"用其他应用打开"时，框架把 Intent 转成
            // RunEvent::Opened；这里负责消费并转发给前端
            #[cfg(target_os = "android")]
            crate::android_intent::handle_run_event(_app, &_event);
        });
}
