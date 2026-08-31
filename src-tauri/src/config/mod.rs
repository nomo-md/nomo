use crate::models::{RecentEntry, SettingRecord, StoredSnapshotRecord};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, Runtime};

pub(crate) mod commands;

const CONFIG_FILE_NAME: &str = "config.json";
pub(crate) const DRAFTS_DIR_NAME: &str = "drafts";
pub(crate) const SNAPSHOTS_DIR_NAME: &str = "snapshots";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    pub(crate) app: AppSection,
    pub(crate) editor: EditorSection,
    pub(crate) window: WindowSection,
    pub(crate) recent: RecentSection,
    pub(crate) workspace: WorkspaceSection,
    pub(crate) snapshots: SnapshotSection,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppSection {
    pub(crate) settings: BTreeMap<String, SettingRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct EditorSection {
    pub(crate) settings: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WindowSection {
    pub(crate) settings: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RecentSection {
    pub(crate) entries: Vec<RecentEntry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceSection {
    pub(crate) settings: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SnapshotSection {
    pub(crate) documents: BTreeMap<String, Vec<StoredSnapshotRecord>>,
}

#[derive(Clone)]
pub(crate) struct ConfigManager {
    config_path: PathBuf,
    config: Arc<RwLock<AppConfig>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app: AppSection::default(),
            editor: EditorSection::default(),
            window: WindowSection::default(),
            recent: RecentSection::default(),
            workspace: WorkspaceSection::default(),
            snapshots: SnapshotSection::default(),
        }
    }
}

impl ConfigManager {
    pub(crate) fn load_or_default<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        let config_path = config_path(app)?;
        Self::load_or_default_from_path(config_path)
    }

    pub(crate) fn load_or_default_from_path(config_path: PathBuf) -> Result<Self, String> {
        let mut config = match fs::read_to_string(&config_path) {
            Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
                Ok(config) => config,
                Err(error) => {
                    backup_broken_config(&config_path)?;
                    crate::app_logger::error(
                        "Config",
                        &format!("配置文件损坏，已备份并使用默认配置：{error}"),
                    );
                    AppConfig::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppConfig::default(),
            Err(error) => return Err(format!("读取配置文件失败：{error}")),
        };

        migrate_legacy_snapshot_markdown(&config_path, &mut config)?;

        let manager = Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
        };
        manager.save()?;
        Ok(manager)
    }

    pub(crate) fn get_config(&self) -> Result<AppConfig, String> {
        self.config
            .read()
            .map(|config| config.clone())
            .map_err(|error| format!("读取配置状态失败：{error}"))
    }

    pub(crate) fn update<F>(&self, updater: F) -> Result<(), String>
    where
        F: FnOnce(&mut AppConfig),
    {
        let next_config = {
            let mut config = self
                .config
                .write()
                .map_err(|error| format!("更新配置状态失败：{error}"))?;
            updater(&mut config);
            config.clone()
        };
        self.save_config(&next_config)
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        let config = self.get_config()?;
        self.save_config(&config)
    }

    #[cfg(any(target_os = "android", test))]
    pub(crate) fn replace_after_save(&self, next: AppConfig) -> Result<(), String> {
        let mut current = self.config.write().map_err(|_| "配置写锁不可用".to_string())?;
        self.save_config(&next)?;
        *current = next;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn reset_to_default(&self) -> Result<(), String> {
        self.update(|config| *config = AppConfig::default())
    }

    fn save_config(&self, config: &AppConfig) -> Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败：{error}"))?;
        }

        let temp_path = self.config_path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(config)
            .map_err(|error| format!("序列化配置失败：{error}"))?;
        {
            let mut file =
                File::create(&temp_path).map_err(|error| format!("创建临时配置失败：{error}"))?;
            file.write_all(content.as_bytes())
                .map_err(|error| format!("写入临时配置失败：{error}"))?;
            file.sync_all()
                .map_err(|error| format!("同步临时配置失败：{error}"))?;
        }

        if cfg!(windows) && self.config_path.exists() {
            fs::remove_file(&self.config_path)
                .map_err(|error| format!("替换旧配置文件失败：{error}"))?;
        }
        fs::rename(&temp_path, &self.config_path)
            .map_err(|error| format!("替换配置文件失败：{error}"))?;
        Ok(())
    }
}

pub(crate) fn with_manager<R: Runtime, T>(
    app: &AppHandle<R>,
    operation: impl FnOnce(&ConfigManager) -> Result<T, String>,
) -> Result<T, String> {
    let manager = app
        .try_state::<ConfigManager>()
        .ok_or_else(|| "配置管理器尚未初始化".to_string())?;
    operation(&manager)
}

pub(crate) fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn app_data_dir_from_config_path(config_path: &Path) -> Result<PathBuf, String> {
    config_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "配置文件路径缺少父目录".to_string())
}

pub(crate) fn content_store_dir<R: Runtime>(
    app: &AppHandle<R>,
    directory_name: &str,
) -> Result<PathBuf, String> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("定位应用数据目录失败：{error}"))?;
    Ok(app_dir.join(directory_name))
}

pub(crate) fn validate_content_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_');
    if valid {
        Ok(())
    } else {
        Err("正文存储标识非法".to_string())
    }
}

pub(crate) fn markdown_store_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_content_id(id)?;
    Ok(root.join(format!("{id}.md")))
}

fn migrate_legacy_snapshot_markdown(
    config_path: &Path,
    config: &mut AppConfig,
) -> Result<(), String> {
    let app_dir = app_data_dir_from_config_path(config_path)?;
    let snapshot_dir = app_dir.join(SNAPSHOTS_DIR_NAME);
    let mut migrated = 0usize;

    for snapshots in config.snapshots.documents.values_mut() {
        for snapshot in snapshots.iter_mut() {
            if let Some(markdown) = snapshot.markdown.take() {
                let snapshot_path = markdown_store_path(&snapshot_dir, &snapshot.content_hash)?;
                if let Some(parent) = snapshot_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("创建快照目录失败：{error}"))?;
                }
                fs::write(&snapshot_path, markdown.as_bytes())
                    .map_err(|error| format!("迁移快照正文失败：{error}"))?;
                migrated += 1;
            }
        }
    }

    if migrated > 0 {
        crate::app_logger::info("Config", &format!("已迁移旧快照正文：{migrated} 个"));
    }
    Ok(())
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("定位应用数据目录失败：{error}"))?;
    fs::create_dir_all(&app_dir).map_err(|error| format!("创建应用数据目录失败：{error}"))?;
    Ok(app_dir.join(CONFIG_FILE_NAME))
}

fn backup_broken_config(config_path: &Path) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(());
    }
    let backup_path = config_path.with_file_name(format!("config.broken.{}.json", now_ts()));
    fs::rename(config_path, backup_path).map_err(|error| format!("备份损坏配置失败：{error}"))
}

/// 步骤1：根据应用标识符构造各平台的应用数据目录路径
pub(crate) fn resolve_app_data_dir(identifier: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // 与 Tauri 的 app_data_dir() 保持一致：Windows 使用 Roaming 目录（APPDATA）
        let app_data = std::env::var("APPDATA").ok()?;
        Some(PathBuf::from(app_data).join(identifier))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(identifier),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".local/share").join(identifier))
    }
}

/// 步骤2：从磁盘配置文件读取指定 key 的 JSON 字符串（启动前使用，无需 AppHandle）
pub(crate) fn read_app_setting_json(identifier: &str, key: &str) -> Option<String> {
    let app_data_dir = resolve_app_data_dir(identifier)?;
    let config_path = app_data_dir.join(CONFIG_FILE_NAME);
    let content = fs::read_to_string(&config_path).ok()?;
    let config: AppConfig = serde_json::from_str(&content).ok()?;
    config
        .app
        .settings
        .get(key)
        .map(|record| record.value_json.clone())
}

/// 步骤3：读取渲染模式设置，仅在显式设置为 software 时返回 true
#[cfg(target_os = "windows")]
pub(crate) fn is_software_render_mode(identifier: &str) -> bool {
    matches!(
        read_app_setting_json(identifier, "renderMode").as_deref(),
        Some("\"software\"")
    )
}

/// 步骤4：读取开发者模式设置，决定启动阶段是否启用日志
pub(crate) fn is_developer_mode(identifier: &str) -> bool {
    matches!(
        read_app_setting_json(identifier, "developerMode").as_deref(),
        Some("true")
    )
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ConfigManager};
    use std::{fs, path::PathBuf};

    #[test]
    fn fills_missing_sections_with_defaults() {
        let config = serde_json::from_str::<AppConfig>(r#"{"app":{}}"#).expect("config");

        assert!(config.app.settings.is_empty());
        assert!(config.recent.entries.is_empty());
        assert!(config.snapshots.documents.is_empty());
    }

    #[test]
    fn creates_default_config_file_when_missing() {
        let root = unique_test_dir("create-default");
        fs::create_dir_all(&root).expect("test dir");
        let config_path = root.join("config.json");

        let manager = ConfigManager::load_or_default_from_path(config_path.clone()).expect("load");

        assert!(config_path.exists());
        assert!(manager
            .get_config()
            .expect("config")
            .recent
            .entries
            .is_empty());
        cleanup(root);
    }

    #[test]
    fn backs_up_broken_config_and_uses_defaults() {
        let root = unique_test_dir("broken");
        fs::create_dir_all(&root).expect("test dir");
        let config_path = root.join("config.json");
        fs::write(&config_path, "{ broken").expect("broken config");

        let manager = ConfigManager::load_or_default_from_path(config_path.clone()).expect("load");

        assert!(config_path.exists());
        assert!(manager
            .get_config()
            .expect("config")
            .app
            .settings
            .is_empty());
        assert!(fs::read_dir(&root)
            .expect("read dir")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("config.broken.")));
        cleanup(root);
    }

    #[test]
    fn migrates_legacy_snapshot_markdown_to_content_files() {
        let root = unique_test_dir("snapshot-migration");
        fs::create_dir_all(&root).expect("test dir");
        let config_path = root.join("config.json");
        fs::write(
            &config_path,
            r##"{
              "app": {},
              "snapshots": {
                "documents": {
                  "D:\\docs\\a.md": [
                    {
                      "id": "1-abc123",
                      "document_path": "D:\\docs\\a.md",
                      "content_hash": "abc123",
                      "markdown": "# legacy",
                      "created_at": 1,
                      "reason": "before-save"
                    }
                  ]
                }
              }
            }"##,
        )
        .expect("write config");

        let manager = ConfigManager::load_or_default_from_path(config_path).expect("load");
        let config = manager.get_config().expect("config");
        let snapshot = config
            .snapshots
            .documents
            .get("D:\\docs\\a.md")
            .and_then(|items| items.first())
            .expect("snapshot");

        assert!(snapshot.markdown.is_none());
        assert_eq!(
            fs::read_to_string(root.join(super::SNAPSHOTS_DIR_NAME).join("abc123.md"))
                .expect("snapshot"),
            "# legacy"
        );
        cleanup(root);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn reads_software_render_mode_from_disk_before_app_handle_available() {
        use super::{is_software_render_mode, CONFIG_FILE_NAME};

        let root = unique_test_dir("render-mode");
        fs::create_dir_all(&root).expect("test dir");
        let identifier = "com.nomo.test-render-mode";
        let app_dir = root.join(identifier);
        fs::create_dir_all(&app_dir).expect("app dir");
        let config_path = app_dir.join(CONFIG_FILE_NAME);
        fs::write(
            &config_path,
            r#"{"app":{"settings":{"renderMode":{"key":"renderMode","value_json":"\"software\"","updated_at":1}}}}"#,
        )
        .expect("write config");

        let original_app_data = std::env::var("APPDATA").ok();
        std::env::set_var("APPDATA", &root);
        let result = is_software_render_mode(identifier);
        match original_app_data {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }

        cleanup(root);
        assert!(result);
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomo-config-{name}-{}", super::now_ts()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
