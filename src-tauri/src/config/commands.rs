use crate::config::{
    self, content_store_dir, markdown_store_path, now_ts, validate_content_id, DRAFTS_DIR_NAME,
    SNAPSHOTS_DIR_NAME,
};
use crate::models::{
    RecentEntry, RecentEntryInput, RecentEntryType, SettingInput, SettingRecord, SnapshotInput,
    SnapshotRecord, StoredSnapshotRecord, WorkspaceDraftInput, WorkspaceDraftPayload,
};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    time::UNIX_EPOCH,
};
use tauri::{AppHandle, Runtime};

#[tauri::command]
pub(crate) fn remember_recent_entry(app: AppHandle, input: RecentEntryInput) -> Result<(), String> {
    let now = now_ts();
    let modified_at = if input.entry_type == RecentEntryType::File {
        crate::file_system::file_modified_at(&input.path)
    } else {
        0
    };
    let entry = RecentEntry {
        path: input.path,
        entry_type: input.entry_type.as_str().to_string(),
        title: input.title,
        modified_at,
        word_count: input.word_count,
        opened_at: now,
    };

    config::with_manager(&app, |manager| {
        manager.update(|config| {
            config.recent.entries.retain(|item| item.path != entry.path);
            config.recent.entries.insert(0, entry);
        })
    })
}

#[tauri::command]
pub(crate) fn list_recent_entries(app: AppHandle) -> Result<Vec<RecentEntry>, String> {
    query_recent_entries_with_limit(&app, 200)
}

#[tauri::command]
pub(crate) fn clear_recent_entries(app: AppHandle) -> Result<(), String> {
    config::with_manager(&app, |manager| {
        manager.update(|config| config.recent.entries.clear())
    })
}

#[tauri::command]
pub(crate) fn create_document_snapshot(app: AppHandle, input: SnapshotInput) -> Result<(), String> {
    let now = now_ts();
    let content_hash = hash_text(&input.markdown);
    let snapshot_dir = content_store_dir(&app, SNAPSHOTS_DIR_NAME)?;
    write_markdown_store_file(
        &snapshot_dir,
        &content_hash,
        &input.markdown,
        "写入快照正文失败",
    )?;

    let snapshot = StoredSnapshotRecord {
        id: format!("{now}-{content_hash}"),
        document_path: input.path,
        content_hash,
        created_at: now,
        reason: input.reason,
        markdown: None,
    };

    config::with_manager(&app, |manager| {
        manager.update(|config| {
            let snapshots = config
                .snapshots
                .documents
                .entry(snapshot.document_path.clone())
                .or_default();
            snapshots.insert(0, snapshot);
            snapshots.truncate(20);
        })
    })
}

#[tauri::command]
pub(crate) fn list_document_snapshots(
    app: AppHandle,
    path: String,
) -> Result<Vec<SnapshotRecord>, String> {
    let snapshot_dir = content_store_dir(&app, SNAPSHOTS_DIR_NAME)?;
    config::with_manager(&app, |manager| {
        let config = manager.get_config()?;
        let mut records = Vec::new();
        for snapshot in config
            .snapshots
            .documents
            .get(&path)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(20)
        {
            let markdown = read_markdown_store_file(
                &snapshot_dir,
                &snapshot.content_hash,
                "读取快照正文失败",
            )?;
            records.push(SnapshotRecord {
                id: snapshot.id,
                document_path: snapshot.document_path,
                content_hash: snapshot.content_hash,
                markdown,
                created_at: snapshot.created_at,
                reason: snapshot.reason,
            });
        }
        Ok(records)
    })
}

#[tauri::command]
pub(crate) fn write_workspace_draft(
    app: AppHandle,
    input: WorkspaceDraftInput,
) -> Result<WorkspaceDraftPayload, String> {
    let draft_id = input
        .draft_id
        .filter(|value| validate_content_id(value).is_ok())
        .unwrap_or_else(|| format!("draft-{}-{}", now_ts(), hash_text(&input.markdown)));
    let draft_dir = content_store_dir(&app, DRAFTS_DIR_NAME)?;
    write_markdown_store_file(&draft_dir, &draft_id, &input.markdown, "写入草稿失败")?;
    let updated_at = markdown_store_updated_at(&draft_dir, &draft_id)?;
    Ok(WorkspaceDraftPayload {
        draft_id,
        markdown: None,
        updated_at,
    })
}

#[tauri::command]
pub(crate) fn read_workspace_draft(
    app: AppHandle,
    draft_id: String,
) -> Result<WorkspaceDraftPayload, String> {
    let draft_dir = content_store_dir(&app, DRAFTS_DIR_NAME)?;
    let markdown = read_markdown_store_file(&draft_dir, &draft_id, "读取草稿失败")?;
    let updated_at = markdown_store_updated_at(&draft_dir, &draft_id)?;
    Ok(WorkspaceDraftPayload {
        draft_id,
        markdown: Some(markdown),
        updated_at,
    })
}

#[tauri::command]
pub(crate) fn delete_workspace_draft(app: AppHandle, draft_id: String) -> Result<(), String> {
    let draft_dir = content_store_dir(&app, DRAFTS_DIR_NAME)?;
    let path = markdown_store_path(&draft_dir, &draft_id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("删除草稿失败：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn update_app_setting(app: AppHandle, input: SettingInput) -> Result<(), String> {
    update_settings(app, vec![input])
}

#[tauri::command]
pub(crate) fn update_app_settings(app: AppHandle, inputs: Vec<SettingInput>) -> Result<(), String> {
    update_settings(app, inputs)
}

#[tauri::command]
pub(crate) fn list_app_settings(app: AppHandle) -> Result<Vec<SettingRecord>, String> {
    config::with_manager(&app, |manager| {
        let config = manager.get_config()?;
        Ok(config.app.settings.values().cloned().collect())
    })
}

pub(crate) fn query_recent_entries<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<RecentEntry>, String> {
    query_recent_entries_with_limit(app, 8)
}

pub(crate) fn get_setting_value<R: Runtime>(
    app: &AppHandle<R>,
    key: &str,
) -> Result<Option<String>, String> {
    config::with_manager(app, |manager| {
        let config = manager.get_config()?;
        Ok(config
            .app
            .settings
            .get(key)
            .map(|record| record.value_json.clone()))
    })
}

fn update_settings<R: Runtime>(app: AppHandle<R>, inputs: Vec<SettingInput>) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }

    let now = now_ts();
    config::with_manager(&app, |manager| {
        manager.update(|config| {
            for input in inputs {
                route_setting_to_section(config, &input.key, &input.value_json);
                config.app.settings.insert(
                    input.key.clone(),
                    SettingRecord {
                        key: input.key,
                        value_json: input.value_json,
                        updated_at: now,
                    },
                );
            }
        })
    })
}

fn route_setting_to_section(config: &mut super::AppConfig, key: &str, value_json: &str) {
    let value =
        serde_json::from_str::<serde_json::Value>(value_json).unwrap_or(serde_json::Value::Null);
    if let Some(label) = key.strip_prefix("windowState:") {
        config.window.settings.insert(label.to_string(), value);
    } else if let Some(label) = key.strip_prefix("workspaceTabs:") {
        config.workspace.settings.insert(label.to_string(), value);
    } else if let Some(label) = key.strip_prefix("pendingFolder:") {
        config
            .window
            .settings
            .insert(format!("pendingFolder:{label}"), value);
    } else if let Some(label) = key.strip_prefix("pendingExternalOpen:") {
        config
            .window
            .settings
            .insert(format!("pendingExternalOpen:{label}"), value);
    } else if is_editor_setting_key(key) {
        config.editor.settings.insert(key.to_string(), value);
    }
}

fn is_editor_setting_key(key: &str) -> bool {
    matches!(
        key,
        "fontSize"
            | "lineHeight"
            | "contentWidthPercent"
            | "blockStyle"
            | "editorMode"
            | "autoSave"
            | "sourceCodeFontSize"
            | "sourceCodeLineHeight"
    )
}

fn query_recent_entries_with_limit<R: Runtime>(
    app: &AppHandle<R>,
    limit: usize,
) -> Result<Vec<RecentEntry>, String> {
    config::with_manager(app, |manager| {
        let mut entries = manager.get_config()?.recent.entries;
        entries.sort_by(|left, right| right.opened_at.cmp(&left.opened_at));
        entries.truncate(limit);
        Ok(entries)
    })
}

fn hash_text(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn write_markdown_store_file(
    directory: &std::path::Path,
    id: &str,
    markdown: &str,
    error_prefix: &str,
) -> Result<(), String> {
    let path = markdown_store_path(directory, id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{error_prefix}：{error}"))?;
    }
    fs::write(path, markdown.as_bytes()).map_err(|error| format!("{error_prefix}：{error}"))
}

fn read_markdown_store_file(
    directory: &std::path::Path,
    id: &str,
    error_prefix: &str,
) -> Result<String, String> {
    let path = markdown_store_path(directory, id)?;
    fs::read_to_string(path).map_err(|error| format!("{error_prefix}：{error}"))
}

fn markdown_store_updated_at(directory: &std::path::Path, id: &str) -> Result<i64, String> {
    let path = markdown_store_path(directory, id)?;
    Ok(fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::hash_text;

    #[test]
    fn hashes_equal_text_consistently() {
        assert_eq!(hash_text("hello"), hash_text("hello"));
        assert_ne!(hash_text("hello"), hash_text("world"));
    }
}
