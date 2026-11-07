//! 旧 Web/Tauri 记录的只读兼容层；不改写 config.json 或 drafts。
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashSet}, fs::{self, File}, io::{Read, Write}, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

pub(crate) const MAX_RECORDS: usize = 512;
const CONFIG_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const DRAFT_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) fn read_config(root: &Path) -> Result<Value, String> {
    let file = match File::open(root.join("config.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(CONFIG_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > CONFIG_BYTES { return Err("旧版配置超过 16 MiB，原记录已保留".into()); }
    serde_json::from_slice(&bytes).map_err(|e| format!("旧版配置无效：{e}"))
}

pub(crate) fn setting(config: &Value, key: &str) -> Result<Value, String> {
    match config["app"]["settings"][key]["value_json"].as_str() {
        Some(value) => serde_json::from_str(value).map_err(|e| format!("旧版 {key} 记录无效：{e}")),
        None => Ok(Value::Null),
    }
}

pub(crate) struct WorkspaceRecord {
    pub key: String,
    pub value: Value,
    pub updated: u64,
}

pub(crate) fn workspaces(config: &Value) -> Result<Vec<WorkspaceRecord>, String> {
    let mut records = BTreeMap::new();
    if let Some(settings) = config["app"]["settings"].as_object() {
        for (key, record) in settings {
            if key == "workspaceTabs" || key.starts_with("workspaceTabs:") {
                records.insert(key.clone(), WorkspaceRecord { key: key.clone(), value: setting(config, key)?, updated: record["updated_at"].as_u64().unwrap_or(0) });
            }
        }
    }
    if let Some(settings) = config["workspace"]["settings"].as_object() {
        for (label, value) in settings {
            let key = format!("workspaceTabs:{label}");
            records.entry(key.clone()).or_insert_with(|| WorkspaceRecord { key, value: value.clone(), updated: 0 });
        }
    }
    if records.len() > MAX_RECORDS { return Err("旧版工作区记录过多，原记录已保留".into()); }
    let mut count = 0;
    for record in records.values() {
        let tabs = record.value["tabs"].as_array().ok_or("旧版工作区缺少标签列表，原记录已保留")?;
        if record.value["version"].as_u64().is_some_and(|v| !matches!(v, 1..=4)) { return Err("无法识别旧版工作区版本，原记录已保留".into()); }
        count += tabs.len();
        if count > MAX_RECORDS { return Err("旧版工作区超过 512 个标签记录，原记录已保留".into()); }
    }
    let mut records: Vec<_> = records.into_values().collect();
    records.sort_by_key(|record| record.updated);
    Ok(records)
}

pub(crate) fn absolute_path(value: &Value) -> Option<PathBuf> {
    let text = value.as_str()?.trim();
    let path = PathBuf::from(text);
    (path.is_absolute() && text.len() <= 8192).then(|| fs::canonicalize(&path).unwrap_or(path))
}

pub(crate) fn session_id(key: &str) -> String { format!("md-legacy-{:x}", Sha256::digest(key.as_bytes())) }

pub(crate) fn copy_draft(mut source: impl Read, output: &mut File, cancelled: &dyn Fn() -> bool) -> Result<u64, String> {
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 0;
    loop {
        if cancelled() { return Err("旧草稿恢复已取消，原记录仍保留".into()); }
        let read = source.read(&mut buffer).map_err(|e|e.to_string())?;
        if read == 0 {return Ok(length);}
        length += read as u64;
        if length > DRAFT_BYTES {return Err("旧草稿或磁盘保存点超过 256 MiB，原文件已保留".into());}
        output.write_all(&buffer[..read]).map_err(|e|e.to_string())?;
    }
}

#[derive(Clone)]
enum Content {
    File { path: PathBuf, len: u64, modified: SystemTime },
    Inline { workspace: String, index: usize, digest: String },
}

#[derive(Clone)]
pub(crate) struct Draft {
    pub root: PathBuf,
    pub id: String,
    pub path: Option<PathBuf>,
    pub title: String,
    pub modified: SystemTime,
    content: Content,
}

fn valid_draft_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 96 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl Draft {
    fn file(root: &Path, id: &str, path: Option<PathBuf>, title: String) -> Result<Self, String> {
        if !valid_draft_id(id) { return Err("旧草稿标识无效，未读取目录外文件".into()); }
        if root.join("drafts").is_symlink() { return Err("旧草稿目录是符号链接，原资料已保留".into()); }
        let source = root.join("drafts").join(format!("{id}.md"));
        let metadata = fs::symlink_metadata(&source).map_err(|e| format!("旧草稿 {id} 无法读取：{e}"))?;
        if !metadata.is_file() || metadata.is_symlink() { return Err("旧草稿不是普通文件".into()); }
        let modified = metadata.modified().map_err(|e| e.to_string())?;
        let stamp = modified.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        Ok(Self { root: root.into(), id: session_id(&format!("draft:{id}:{}:{stamp}", metadata.len())), path, title, modified,
            content: Content::File { path: source, len: metadata.len(), modified } })
    }

    pub(crate) fn for_tab(root: &Path, workspace: &str, index: usize, tab: &Value) -> Result<Option<Self>, String> {
        let path = absolute_path(&tab["nativePath"]);
        let title = tab["fileName"].as_str().unwrap_or("旧版未命名草稿").chars().take(256).collect();
        // 无版本记录的正文是事实源，不能因同条记录还保留 draftId 而采用较旧文件。
        if let Some(markdown) = tab["markdown"].as_str().filter(|text| tab["dirty"].as_bool() == Some(true) || path.is_none() && !text.is_empty()) {
            let digest = format!("{:x}", Sha256::digest(markdown.as_bytes()));
            let tab_id = tab["id"].as_str().map(str::to_owned).unwrap_or_else(|| format!("{workspace}:{index}"));
            return Ok(Some(Self { root: root.into(), id: session_id(&format!("inline:{tab_id}:{digest}")), path, title, modified: fs::metadata(root.join("config.json")).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH),
                content: Content::Inline { workspace: workspace.into(), index, digest } }));
        }
        tab["draftId"].as_str().filter(|id| !id.is_empty()).map(|id| Self::file(root, id, path, title)).transpose()
    }

    fn receipt(&self) -> PathBuf { self.root.join("gpui-legacy-records").join(format!("{}.done", self.id)) }
    pub(crate) fn handled(&self) -> bool { self.receipt().is_file() }
    pub(crate) fn mark_handled(&self) -> Result<(), String> {
        let path = self.receipt();
        fs::create_dir_all(path.parent().ok_or("旧记录标记目录无效")?).map_err(|e| e.to_string())?;
        crate::file_system::write_file_atomically(&path, b"1\n")
    }

    pub(crate) fn copy_to(&self, output: &mut File, cancelled: &dyn Fn() -> bool) -> Result<u64, String> {
        match &self.content {
            Content::File { path, len, modified } => {
                let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.is_symlink() || metadata.len() != *len || metadata.modified().ok() != Some(*modified) { return Err("旧草稿已变化，请重启后重新检查恢复记录".into()); }
                if *len > DRAFT_BYTES { return Err("旧草稿超过 256 MiB，原文件已保留".into()); }
                let mut source = File::open(path).map_err(|e| e.to_string())?;
                let opened = source.metadata().map_err(|e|e.to_string())?;
                if opened.len()!=*len || opened.modified().ok()!=Some(*modified) {return Err("旧草稿在打开时发生变化，原记录仍保留".into());}
                let copied = copy_draft(&mut source, output, cancelled)?;
                let after = fs::metadata(path).map_err(|e| e.to_string())?;
                if copied != *len || after.len() != *len || after.modified().ok() != Some(*modified) { return Err("旧草稿在读取期间变化，未导入不完整内容".into()); }
                Ok(copied)
            }
            Content::Inline { workspace, index, digest } => {
                let config = read_config(&self.root)?;
                let records = workspaces(&config)?;
                let text = records.iter().find(|r| &r.key == workspace).and_then(|r| r.value["tabs"][*index]["markdown"].as_str()).ok_or("旧内嵌草稿已不存在")?;
                if format!("{:x}", Sha256::digest(text.as_bytes())) != *digest { return Err("旧内嵌草稿已变化，请重启后重新检查恢复记录".into()); }
                copy_draft(text.as_bytes(), output, cancelled)
            }
        }
    }
}

pub(crate) fn drafts(root: &Path) -> Result<(Vec<Draft>, Option<String>), String> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    let mut referenced = HashSet::new();
    let mut warning = None;
    let records = read_config(root).and_then(|config| workspaces(&config));
    match records {
        Ok(records) => for record in records {
            for (index, tab) in record.value["tabs"].as_array().unwrap().iter().enumerate() {
                if let Some(id) = tab["draftId"].as_str() { referenced.insert(id.to_owned()); }
                match Draft::for_tab(root, &record.key, index, tab) {
                    Ok(Some(draft)) if !draft.handled() && seen.insert(draft.id.clone()) => output.push(draft),
                    Err(error) => { warning.get_or_insert(error); },
                    _ => {},
                }
            }
        },
        Err(error) => { warning = Some(error); },
    }
    let directory = root.join("drafts");
    if directory.is_symlink() { return Err("旧草稿目录是符号链接，原资料已保留".into()); }
    let files = match fs::read_dir(&directory) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((output, warning)),
        Err(error) => return Err(error.to_string()),
    };
    for (index, entry) in files.enumerate() {
        if index >= MAX_RECORDS { warning.get_or_insert("旧草稿目录超过 512 项，超出部分仍在原目录保留".into()); break; }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".md")) else { continue; };
        if referenced.contains(id) || !valid_draft_id(id) { continue; }
        match Draft::file(root, id, None, id.into()) {
            Ok(draft) if !draft.handled() && seen.insert(draft.id.clone()) => output.push(draft),
            Err(error) => { warning.get_or_insert(error); },
            _ => {},
        }
    }
    output.sort_by_key(|draft| std::cmp::Reverse(draft.modified));
    Ok((output, warning))
}
