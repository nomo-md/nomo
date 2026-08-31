//! Android 导入副本的持久化与旧缓存迁移；不回写来源，也不按时间删除用户文档。
use crate::config::ConfigManager;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(crate) const INCOMING_DIR: &str = "incoming";

/// 同名临时文件是保留锁；正式路径只有在所有字节落盘后才会出现。
pub(crate) struct StagedImport {
    pub(crate) file: File,
    temporary: PathBuf,
    target: PathBuf,
}

impl StagedImport {
    pub(crate) fn create(root: &Path, name: &str) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let safe = sanitize_name(name);
        let path = Path::new(&safe);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported");
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("md");
        for index in 0..10_000 {
            let target = root.join(if index == 0 {
                safe.clone()
            } else {
                format!("{stem}-{index}.{extension}")
            });
            if target.exists() {
                continue;
            }
            let temporary = target.with_file_name(format!(
                ".{}.part",
                target.file_name().unwrap().to_string_lossy()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        temporary,
                        target,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many import name collisions",
        ))
    }

    pub(crate) fn publish(mut self) -> io::Result<PathBuf> {
        self.file.flush()?;
        self.file.sync_all()?;
        // Hard-link publication is atomic and refuses to replace an existing document.
        fs::hard_link(&self.temporary, &self.target)?;
        Ok(self.target.clone())
    }
}

impl Drop for StagedImport {
    fn drop(&mut self) {
        // Only our own partial file is disposable. Published/user files are never cleaned here.
        let _ = fs::remove_file(&self.temporary);
    }
}

pub(crate) fn sanitize_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() {
        "imported.md".into()
    } else {
        safe.to_string()
    }
}

pub(crate) fn shared_name(delivery: &str) -> String {
    format!("shared-{:x}.md", Sha256::digest(delivery.as_bytes()))
}

pub(crate) fn write_shared(root: &Path, delivery: &str, text: &str) -> io::Result<PathBuf> {
    let name = shared_name(delivery);
    let existing = root.join(&name);
    if existing.is_file() {
        return Ok(existing);
    }
    let mut staged = StagedImport::create(root, &name)?;
    staged.file.write_all(text.as_bytes())?;
    staged.publish()
}

pub(crate) fn copy_file(root: &Path, source: &Path) -> io::Result<PathBuf> {
    let mut input = File::open(source)?;
    if !input.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a document",
        ));
    }
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let mut staged = StagedImport::create(root, &name.to_string_lossy())?;
    io::copy(&mut input, &mut staged.file)?;
    staged.publish()
}

/// 可重复执行：持久副本已经存在时不覆盖；配置保存失败时内存和原配置保持不变。
pub(crate) fn migrate_legacy(
    config: &ConfigManager,
    cache: &Path,
    durable: &Path,
) -> Result<usize, String> {
    if !cache.is_dir() {
        return Ok(0);
    }
    let mut mappings = BTreeMap::new();
    let mut failures = 0;
    let entries = fs::read_dir(cache).map_err(|_| "无法读取旧导入缓存".to_string())?;
    for entry in entries {
        let Ok(entry) = entry else {
            failures += 1;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            failures += 1;
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let source = entry.path();
        if !matches!(
            source
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "md" | "markdown" | "txt" | "json"
        ) {
            continue;
        }
        let source_string = source.to_string_lossy().into_owned();
        let name = format!(
            "legacy-{:x}-{}",
            Sha256::digest(source_string.as_bytes()),
            sanitize_name(&entry.file_name().to_string_lossy())
        );
        let target = durable.join(&name);
        let copied = (|| -> io::Result<PathBuf> {
            if target.is_file() {
                return Ok(target.clone());
            }
            let mut input = File::open(&source)?;
            let mut staged = StagedImport::create(durable, &name)?;
            io::copy(&mut input, &mut staged.file)?;
            staged.publish()
        })();
        match copied {
            Ok(target) => {
                mappings.insert(source_string, target.to_string_lossy().into_owned());
            }
            Err(_) => failures += 1,
        }
    }
    if !mappings.is_empty() {
        let mut next = config.get_config()?;
        for entry in &mut next.recent.entries {
            replace_path(&mut entry.path, &mappings);
        }
        for value in next.workspace.settings.values_mut() {
            remap_workspace(value, &mappings);
        }
        for (key, value) in &mut next.window.settings {
            if key.starts_with("pendingExternalOpen:") {
                remap_pending(value, &mappings);
            }
        }
        for (key, record) in &mut next.app.settings {
            if key.starts_with("pendingExternalOpen:") {
                if let Ok(mut value) = serde_json::from_str::<Value>(&record.value_json) {
                    remap_pending(&mut value, &mappings);
                    record.value_json = value.to_string();
                }
            }
        }
        let snapshots = std::mem::take(&mut next.snapshots.documents);
        for (mut path, mut records) in snapshots {
            replace_path(&mut path, &mappings);
            for record in &mut records {
                replace_path(&mut record.document_path, &mappings);
            }
            let destination = next.snapshots.documents.entry(path).or_default();
            for record in records {
                if !destination.iter().any(|item| item.id == record.id) {
                    destination.push(record);
                }
            }
        }
        config.replace_after_save(next)?;
    }
    Ok(failures)
}

fn replace_path(path: &mut String, mappings: &BTreeMap<String, String>) {
    if let Some(next) = mappings.get(path) {
        *path = next.clone();
    }
}

fn remap_pending(value: &mut Value, mappings: &BTreeMap<String, String>) {
    if let Some(paths) = value.as_array_mut() {
        for path in paths {
            if let Some(next) = path.as_str().and_then(|path| mappings.get(path)).cloned() {
                *path = Value::String(next);
            }
        }
    }
}

fn remap_workspace(value: &mut Value, mappings: &BTreeMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "filePath" | "nativePath" | "recoveryConflictPath"
                ) {
                    if let Some(next) = value.as_str().and_then(|path| mappings.get(path)) {
                        *value = Value::String(next.clone());
                    }
                } else if matches!(value, Value::Object(_) | Value::Array(_)) {
                    remap_workspace(value, mappings);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                remap_workspace(item, mappings);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RecentEntry, SettingRecord, StoredSnapshotRecord};
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::UNIX_EPOCH,
    };
    static NEXT_ROOT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            // Reserve the directory before sharing its path; wall-clock ticks can repeat on Windows.
            loop {
                let path = std::env::temp_dir().join(format!(
                    "nomo-mobile-import-{}-{}",
                    std::process::id(),
                    NEXT_ROOT_ID.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot reserve import test directory: {error}"),
                }
            }
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mobile_imports_test_roots_are_exclusive_during_parallel_runs() {
        let barrier = std::sync::Barrier::new(32);
        let mut roots = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..32)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        TestRoot::new()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        let unique: std::collections::HashSet<_> = roots.iter().map(|root| &root.0).collect();
        assert_eq!(unique.len(), 32);
        for root in &roots {
            fs::write(root.0.join("owned.txt"), "keep").unwrap();
        }
        drop(roots.remove(0));
        for root in roots {
            assert_eq!(
                fs::read_to_string(root.0.join("owned.txt")).unwrap(),
                "keep"
            );
        }
    }

    #[test]
    fn mobile_imports_publish_atomically_and_preserve_edits_and_collisions() {
        let root = TestRoot::new();
        let mut incomplete = StagedImport::create(&root.0, "note.md").unwrap();
        incomplete.file.write_all(b"partial").unwrap();
        assert!(!root.0.join("note.md").exists());
        drop(incomplete);
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
        let first = write_shared(&root.0, "delivery-1", "hello").unwrap();
        fs::write(&first, "user edits").unwrap();
        assert_eq!(first, write_shared(&root.0, "delivery-1", "hello").unwrap());
        assert_eq!(fs::read_to_string(first).unwrap(), "user edits");
        let one = StagedImport::create(&root.0, "same.md")
            .unwrap()
            .publish()
            .unwrap();
        let two = StagedImport::create(&root.0, "same.md")
            .unwrap()
            .publish()
            .unwrap();
        assert_ne!(one, two);
        let source = root.0.join("source.txt");
        fs::write(&source, "source content").unwrap();
        let copy = copy_file(&root.0.join("incoming"), &source).unwrap();
        fs::write(copy, "edited imported copy").unwrap();
        assert_eq!(fs::read_to_string(source).unwrap(), "source content");
    }

    #[test]
    fn mobile_imports_migrate_old_files_and_references_without_touching_body() {
        let root = TestRoot::new();
        let cache = root.0.join("cache/incoming");
        fs::create_dir_all(&cache).unwrap();
        let old = cache.join("old.md");
        fs::write(&old, "saved user document").unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(UNIX_EPOCH)
            .unwrap();
        let config =
            ConfigManager::load_or_default_from_path(root.0.join("data/config.json")).unwrap();
        let path = old.to_string_lossy().into_owned();
        config.update(|config| {
            config.workspace.settings.insert("main".into(), serde_json::json!({"tabs":[{"nativePath":path,"filePath":path,"markdown":path}]}));
            config.window.settings.insert("pendingExternalOpen:main".into(), serde_json::json!([path]));
            config.app.settings.insert("pendingExternalOpen:main".into(), SettingRecord {
                key: "pendingExternalOpen:main".into(), value_json: serde_json::json!([path]).to_string(), updated_at: 1,
            });
            config.recent.entries.push(RecentEntry { path: path.clone(), entry_type: "file".into(), title: None, modified_at: 0, word_count: 0, opened_at: 1 });
            config.snapshots.documents.insert(path.clone(), vec![StoredSnapshotRecord {
                id: "snapshot-1".into(), document_path: path.clone(), content_hash: "saved-content".into(), created_at: 1, reason: "manual".into(), markdown: None,
            }]);
        }).unwrap();
        let durable = root.0.join("data/incoming");
        assert_eq!(migrate_legacy(&config, &cache, &durable).unwrap(), 0);
        let first = config.get_config().unwrap();
        let tab = &first.workspace.settings["main"]["tabs"][0];
        let target = tab["nativePath"].as_str().unwrap();
        assert_ne!(target, path);
        assert_eq!(tab["markdown"], path);
        assert_eq!(first.window.settings["pendingExternalOpen:main"][0], target);
        assert_eq!(first.recent.entries[0].path, target);
        assert_eq!(first.snapshots.documents[target][0].document_path, target);
        let pending: Value =
            serde_json::from_str(&first.app.settings["pendingExternalOpen:main"].value_json)
                .unwrap();
        assert_eq!(pending[0], target);
        fs::write(target, "new edits").unwrap();
        migrate_legacy(&config, &cache, &durable).unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "new edits");
        assert!(old.exists());
        assert_eq!(fs::read_dir(&durable).unwrap().count(), 1);
        // Simulate clearing only the old cache and restarting the application.
        fs::remove_file(&old).unwrap();
        let reopened =
            ConfigManager::load_or_default_from_path(root.0.join("data/config.json")).unwrap();
        assert_eq!(
            reopened.get_config().unwrap().recent.entries[0].path,
            target
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "new edits");
    }

    #[test]
    fn mobile_imports_copy_and_config_failures_preserve_original_references() {
        let root = TestRoot::new();
        let cache = root.0.join("cache/incoming");
        fs::create_dir_all(&cache).unwrap();
        let old = cache.join("old.md");
        fs::write(&old, "original").unwrap();
        let config_path = root.0.join("data/config.json");
        let config = ConfigManager::load_or_default_from_path(config_path.clone()).unwrap();
        config
            .update(|config| {
                config
                    .window
                    .settings
                    .insert("pendingExternalOpen:main".into(), serde_json::json!([old]));
            })
            .unwrap();
        let original_config = fs::read(&config_path).unwrap();
        let durable = root.0.join("data/incoming");
        // An ordinary file where a directory is required deterministically prevents copying.
        fs::write(&durable, "blocked").unwrap();
        assert_eq!(migrate_legacy(&config, &cache, &durable).unwrap(), 1);
        assert_eq!(fs::read(&config_path).unwrap(), original_config);
        assert_eq!(
            config.get_config().unwrap().window.settings["pendingExternalOpen:main"][0],
            old.to_string_lossy().as_ref()
        );
        fs::remove_file(&durable).unwrap();
        // Refuse the temporary config write after a successful document copy.
        let blocked_temp = config_path.with_extension("json.tmp");
        fs::create_dir(&blocked_temp).unwrap();
        assert!(migrate_legacy(&config, &cache, &durable).is_err());
        assert_eq!(fs::read(&config_path).unwrap(), original_config);
        assert_eq!(
            config.get_config().unwrap().window.settings["pendingExternalOpen:main"][0],
            old.to_string_lossy().as_ref()
        );
        assert_eq!(fs::read_to_string(&old).unwrap(), "original");
        assert_eq!(fs::read_dir(&durable).unwrap().count(), 1);
        fs::remove_dir(&blocked_temp).unwrap();
        assert_eq!(migrate_legacy(&config, &cache, &durable).unwrap(), 0);
        assert_eq!(fs::read_dir(&durable).unwrap().count(), 1);
    }
}
