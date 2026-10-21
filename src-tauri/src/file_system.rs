pub(crate) mod image_assets;

use crate::models::{DocumentPayload, FileStatus, FileTreeEntry, FolderFileInfo, MarkdownEncoding};
use encoding_rs::GBK;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{path::BaseDirectory, AppHandle, Manager};
const SAMPLE_DOCUMENT_RESOURCE_PATH: &str = "samples/sample.md";
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];

#[tauri::command]
pub(crate) fn create_folder(path: String) -> Result<(), String> {
    crate::app_logger::info("FileSystem", &format!("创建文件夹：{path}"));
    fs::create_dir_all(&path).map_err(|error| format!("创建文件夹失败：{error}"))
}

#[tauri::command]
pub(crate) fn rename_file(old_path: String, new_path: String) -> Result<(), String> {
    crate::app_logger::info("FileSystem", &format!("重命名：{old_path} -> {new_path}"));
    fs::rename(&old_path, &new_path).map_err(|error| format!("重命名失败：{error}"))
}

#[tauri::command]
pub(crate) fn delete_file(path: String) -> Result<(), String> {
    crate::app_logger::info("FileSystem", &format!("删除文件或目录：{path}"));
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(format!("文件不存在：{path}"));
    }
    if file_path.is_dir() {
        fs::remove_dir_all(&path).map_err(|error| format!("删除文件夹失败：{error}"))
    } else {
        fs::remove_file(&path).map_err(|error| format!("删除文件失败：{error}"))
    }
}

#[tauri::command]
pub(crate) fn read_markdown_file(path: String) -> Result<DocumentPayload, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::info("FileSystem", &format!("开始打开文档：{path}"));
    let status = file_status(&path);
    if !status.exists {
        return Err(format!("文件不存在：{path}"));
    }
    if !status.is_file {
        return Err(format!("路径不是文件：{path}"));
    }

    let bytes = fs::read(&path).map_err(|error| format!("读取 Markdown 文件失败：{error}"))?;
    let (markdown, encoding) = decode_markdown(&bytes)?;
    let payload = document_payload(path, markdown, encoding)?;
    crate::app_logger::perf("FileSystem", "文档打开", timer.elapsed());
    Ok(payload)
}

#[tauri::command]
pub(crate) fn write_markdown_file(
    path: String,
    markdown: String,
) -> Result<DocumentPayload, String> {
    let encoding = if Path::new(&path).is_file() {
        let bytes =
            fs::read(&path).map_err(|error| format!("读取原 Markdown 文件编码失败：{error}"))?;
        Some(decode_markdown(&bytes)?.1)
    } else {
        None
    };
    write_markdown_file_with_encoding(path, markdown, encoding)
}

#[tauri::command]
pub(crate) fn write_markdown_file_with_encoding(
    path: String,
    markdown: String,
    encoding: Option<MarkdownEncoding>,
) -> Result<DocumentPayload, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::info(
        "FileSystem",
        &format!("开始保存文档：{path} bytes={}", markdown.len()),
    );
    if let Some(parent) = Path::new(&path).parent() {
        if !parent.exists() {
            return Err(format!("保存目录不存在：{}", parent.display()));
        }
    }

    let encoding = encoding.unwrap_or_default();
    let bytes = encode_markdown(&markdown, encoding)?;
    write_file_atomically(Path::new(&path), &bytes)?;
    let payload = document_payload(path, markdown, encoding)?;
    crate::app_logger::perf("FileSystem", "文档保存", timer.elapsed());
    Ok(payload)
}

#[tauri::command]
pub(crate) fn install_sample_document(app: AppHandle) -> Result<DocumentPayload, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::info("FileSystem", "打开实例文档");
    // 直接读取安装目录下的实例文档资源，不再复制到用户应用数据目录。
    // 因此用户在编辑器中保存时，若安装目录不可写，会触发写失败的错误兜底。
    let resource_path = resolve_sample_document_resource(&app)?;
    let payload = read_sample_document(&resource_path)?;
    crate::app_logger::perf("FileSystem", "打开实例文档", timer.elapsed());
    Ok(payload)
}

#[tauri::command]
pub(crate) fn stat_markdown_file(path: String) -> FileStatus {
    crate::app_logger::debug("FileSystem", &format!("读取文件状态：{path}"));
    file_status(&path)
}

#[tauri::command]
pub(crate) fn list_folder_markdown_files(path: String) -> Result<Vec<FolderFileInfo>, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::info("FileSystem", &format!("列出目录支持的文档：{path}"));
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(format!("不是一个有效的目录：{path}"));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| format!("读取目录失败：{error}"))? {
        let entry = entry.map_err(|error| format!("读取目录项失败：{error}"))?;
        let path_buf = entry.path();
        if path_buf.is_file() && is_supported_document_file(&path_buf) {
            if let Some(name) = path_buf.file_name().and_then(|n| n.to_str()) {
                files.push(FolderFileInfo {
                    name: name.to_string(),
                    path: path_buf.to_string_lossy().to_string(),
                });
            }
        }
    }
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    crate::app_logger::perf("FileSystem", "列出目录支持的文档", timer.elapsed());
    Ok(files)
}

#[tauri::command]
pub(crate) fn get_folder_tree(path: String) -> Result<Vec<FileTreeEntry>, String> {
    crate::app_logger::info("FileSystem", &format!("读取文件夹树：{path}"));
    list_folder_children(path.clone(), Some(path))
}

#[tauri::command]
pub(crate) fn list_folder_children(
    path: String,
    root_path: Option<String>,
) -> Result<Vec<FileTreeEntry>, String> {
    let timer = std::time::Instant::now();
    crate::app_logger::debug("FileSystem", &format!("读取目录子项：{path}"));
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err(format!("不是一个有效的目录：{path}"));
    }

    let root = root_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .unwrap_or(dir);
    let ignore_rules = IgnoreRules::load(root, Some(dir));
    let children = read_dir_children(dir, root, &ignore_rules)?;
    crate::app_logger::perf("FileSystem", "读取目录子项", timer.elapsed());
    Ok(children)
}

pub(crate) fn file_modified_at(path: &str) -> i64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn document_payload(
    path: String,
    markdown: String,
    encoding: MarkdownEncoding,
) -> Result<DocumentPayload, String> {
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled.md")
        .to_string();

    Ok(DocumentPayload {
        modified_at: file_modified_at(&path),
        size_bytes: file_size(&path),
        readonly: file_readonly(&path),
        path,
        file_name,
        markdown,
        encoding,
    })
}

fn decode_markdown(bytes: &[u8]) -> Result<(String, MarkdownEncoding), String> {
    if let Some(body) = bytes.strip_prefix(UTF8_BOM) {
        return std::str::from_utf8(body)
            .map(|value| (value.to_string(), MarkdownEncoding::Utf8Bom))
            .map_err(|error| format!("读取 Markdown 文件失败：UTF-8 BOM 文件内容无效：{error}"));
    }

    if let Some(body) = bytes.strip_prefix(UTF16_LE_BOM) {
        return decode_utf16(body, true).map(|value| (value, MarkdownEncoding::Utf16LeBom));
    }

    if let Some(body) = bytes.strip_prefix(UTF16_BE_BOM) {
        return decode_utf16(body, false).map(|value| (value, MarkdownEncoding::Utf16BeBom));
    }

    if let Ok(value) = std::str::from_utf8(bytes) {
        return Ok((value.to_string(), MarkdownEncoding::Utf8));
    }

    if let Some(value) = GBK.decode_without_bom_handling_and_without_replacement(bytes) {
        return Ok((value.into_owned(), MarkdownEncoding::Gbk));
    }

    Err("读取 Markdown 文件失败：文件不是有效的 UTF-8、带 BOM 的 UTF-16 或 GBK 编码".to_string())
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("读取 Markdown 文件失败：UTF-16 文件字节数无效".to_string());
    }

    let units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();

    String::from_utf16(&units)
        .map_err(|error| format!("读取 Markdown 文件失败：UTF-16 文件内容无效：{error}"))
}

fn encode_markdown(markdown: &str, encoding: MarkdownEncoding) -> Result<Vec<u8>, String> {
    match encoding {
        MarkdownEncoding::Utf8 => Ok(markdown.as_bytes().to_vec()),
        MarkdownEncoding::Utf8Bom => {
            let mut bytes = Vec::with_capacity(UTF8_BOM.len() + markdown.len());
            bytes.extend_from_slice(UTF8_BOM);
            bytes.extend_from_slice(markdown.as_bytes());
            Ok(bytes)
        }
        MarkdownEncoding::Utf16LeBom => {
            let mut bytes = Vec::with_capacity(UTF16_LE_BOM.len() + markdown.len() * 2);
            bytes.extend_from_slice(UTF16_LE_BOM);
            for unit in markdown.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(bytes)
        }
        MarkdownEncoding::Utf16BeBom => {
            let mut bytes = Vec::with_capacity(UTF16_BE_BOM.len() + markdown.len() * 2);
            bytes.extend_from_slice(UTF16_BE_BOM);
            for unit in markdown.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            Ok(bytes)
        }
        MarkdownEncoding::Gbk => {
            let (bytes, _, had_errors) = GBK.encode(markdown);
            if had_errors {
                return Err(
                    "保存 Markdown 文件失败：文档包含 GBK 无法表示的字符，原文件未被修改"
                        .to_string(),
                );
            }
            Ok(bytes.into_owned())
        }
    }
}

pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_file_atomically_with(path, |writer| writer.write_all(bytes).map_err(|error| error.to_string()))
}

/// 完整快照流式写入；仅在编码、写入和同步全部成功后替换目标。
pub(crate) fn write_file_atomically_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temp_path = unique_temp_path(parent, path);

    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| format!("创建临时保存文件失败：{error}"))?;

    if let Err(error) = write(&mut temp_file) {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(format!("写入临时保存文件失败：{error}"));
    }
    if let Err(error) = temp_file.sync_all() {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(format!("同步临时保存文件失败：{error}"));
    }
    drop(temp_file);

    if let Err(error) = replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("保存文件失败：{error}"));
    }

    sync_parent_dir(parent);
    Ok(())
}

fn unique_temp_path(parent: &Path, target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("document.md");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(".{file_name}.{}.{}.tmp", process::id(), nonce))
}

#[cfg(target_os = "windows")]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let mut from: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    from.push(0);
    let mut to: Vec<u16> = target_path.as_os_str().encode_wide().collect();
    to.push(0);

    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    fs::rename(temp_path, target_path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_dir(parent: &Path) {
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_parent_dir(_parent: &Path) {}

fn read_sample_document(resource_path: &Path) -> Result<DocumentPayload, String> {
    // 直接读取安装目录下的实例文档资源，不再复制到用户应用数据目录。
    let bytes = fs::read(resource_path).map_err(|error| format!("读取实例文档失败：{error}"))?;
    let (markdown, encoding) = decode_markdown(&bytes)?;
    document_payload(
        resource_path.to_string_lossy().to_string(),
        markdown,
        encoding,
    )
}

fn resolve_sample_document_resource(app: &AppHandle) -> Result<PathBuf, String> {
    let mut attempts = Vec::new();

    if let Ok(path) = app
        .path()
        .resolve(SAMPLE_DOCUMENT_RESOURCE_PATH, BaseDirectory::Resource)
    {
        attempts.push(path);
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            attempts.push(exe_dir.join(SAMPLE_DOCUMENT_RESOURCE_PATH));
            attempts.push(
                exe_dir
                    .join("resources")
                    .join(SAMPLE_DOCUMENT_RESOURCE_PATH),
            );
        }
    }

    for path in attempts {
        if path.is_file() && file_size(&path.to_string_lossy()) > 0 {
            return Ok(path);
        }
    }

    Err("定位实例文档资源失败：未找到有效的 samples/sample.md".to_string())
}

#[tauri::command]
pub(crate) fn check_paths_exist(paths: Vec<String>) -> Result<Vec<bool>, String> {
    crate::app_logger::debug(
        "FileSystem",
        &format!("批量检查路径存在性：count={}", paths.len()),
    );
    Ok(paths
        .into_iter()
        .map(|path| Path::new(&path).exists())
        .collect())
}

fn file_status(path: &str) -> FileStatus {
    let metadata = fs::metadata(path);

    FileStatus {
        path: path.to_string(),
        exists: Path::new(path).exists(),
        is_file: metadata
            .as_ref()
            .map(|value| value.is_file())
            .unwrap_or(false),
        modified_at: file_modified_at(path),
        size_bytes: metadata
            .as_ref()
            .map(|value| value.len() as i64)
            .unwrap_or_default(),
        readonly: metadata
            .as_ref()
            .map(|value| value.permissions().readonly())
            .unwrap_or(false),
    }
}

fn file_size(path: &str) -> i64 {
    fs::metadata(path)
        .map(|metadata| metadata.len() as i64)
        .unwrap_or_default()
}

fn file_readonly(path: &str) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().readonly())
        .unwrap_or(false)
}

fn read_dir_children(
    dir: &Path,
    root: &Path,
    ignore_rules: &IgnoreRules,
) -> Result<Vec<FileTreeEntry>, String> {
    let mut entries = Vec::new();
    let snapshots = snapshot_directory_entries(dir)?;

    // 父目录的 ReadDir 已在快照函数返回前释放，后续元数据和子目录探测不会延长其句柄生命周期。
    for snapshot in snapshots {
        let path_buf = snapshot.path;
        let name = snapshot.name;
        let is_dir = path_buf.is_dir();

        if ignore_rules.is_ignored(root, &path_buf, &name, is_dir) {
            continue;
        }

        if is_dir {
            entries.push(FileTreeEntry {
                name,
                path: path_buf.to_string_lossy().to_string(),
                is_dir: true,
                has_children: has_visible_children(&path_buf, root, ignore_rules),
                children_loaded: false,
                children: Vec::new(),
            });
        } else if path_buf.is_file() && is_supported_document_file(&path_buf) {
            entries.push(FileTreeEntry {
                name,
                path: path_buf.to_string_lossy().to_string(),
                is_dir: false,
                has_children: false,
                children_loaded: true,
                children: Vec::new(),
            });
        }
    }

    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(entries)
}

#[derive(Debug)]
struct DirectoryEntrySnapshot {
    path: PathBuf,
    name: String,
}

fn snapshot_directory_entries(dir: &Path) -> Result<Vec<DirectoryEntrySnapshot>, String> {
    let read_dir = fs::read_dir(dir).map_err(|error| format!("读取目录失败：{error}"))?;
    let mut snapshots = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|error| format!("读取目录项失败：{error}"))?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        snapshots.push(DirectoryEntrySnapshot { path, name });
    }
    Ok(snapshots)
}

fn has_visible_children(dir: &Path, root: &Path, ignore_rules: &IgnoreRules) -> bool {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return false;
    };

    for entry in read_dir.flatten() {
        let path_buf = entry.path();
        let name = path_buf
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let is_dir = path_buf.is_dir();
        if ignore_rules.is_ignored(root, &path_buf, name, is_dir) {
            continue;
        }
        if is_dir || (path_buf.is_file() && is_supported_document_file(&path_buf)) {
            return true;
        }
    }

    false
}

// 文件夹树和外部目录入口必须共享同一扩展名契约，避免某个入口静默隐藏分段文档。
fn is_supported_document_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = ext.to_lowercase();
            matches!(ext.as_str(), "md" | "markdown" | "txt" | "json")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct IgnorePattern {
    value: String,
    negated: bool,
    directory_only: bool,
}

#[derive(Debug, Clone)]
struct IgnoreRules {
    patterns: Vec<IgnorePattern>,
}

impl IgnoreRules {
    fn load(root: &Path, extra_dir: Option<&Path>) -> Self {
        let mut patterns = built_in_ignore_patterns();
        append_gitignore_patterns(root, &mut patterns);
        if let Some(dir) = extra_dir {
            if dir != root {
                append_gitignore_patterns(dir, &mut patterns);
            }
        }
        Self { patterns }
    }

    fn is_ignored(&self, root: &Path, path: &Path, name: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.directory_only && !is_dir {
                continue;
            }
            if pattern_matches(root, path, name, pattern) {
                ignored = !pattern.negated;
            }
        }
        ignored
    }
}

fn built_in_ignore_patterns() -> Vec<IgnorePattern> {
    [
        ".git/",
        ".hg/",
        ".svn/",
        "node_modules/",
        "target/",
        "dist/",
        "build/",
        "out/",
        "coverage/",
        ".next/",
        ".svelte-kit/",
        ".turbo/",
        ".cache/",
    ]
    .into_iter()
    .filter_map(parse_ignore_pattern)
    .collect()
}

fn append_gitignore_patterns(dir: &Path, patterns: &mut Vec<IgnorePattern>) {
    let content = fs::read_to_string(dir.join(".gitignore")).unwrap_or_default();
    patterns.extend(content.lines().filter_map(parse_ignore_pattern));
}

fn parse_ignore_pattern(line: &str) -> Option<IgnorePattern> {
    let mut value = line.trim();
    if value.is_empty() || value.starts_with('#') {
        return None;
    }

    let negated = value.starts_with('!');
    if negated {
        value = value.trim_start_matches('!').trim();
    }
    if value.is_empty() {
        return None;
    }

    let directory_only = value.ends_with('/');
    let value = value
        .trim_start_matches('/')
        .trim_end_matches('/')
        .replace('\\', "/");
    if value.is_empty() {
        return None;
    }

    Some(IgnorePattern {
        value,
        negated,
        directory_only,
    })
}

fn pattern_matches(root: &Path, path: &Path, name: &str, pattern: &IgnorePattern) -> bool {
    let relative = path
        .strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| name.replace('\\', "/"));

    if pattern.value.contains('/') {
        wildcard_match(&pattern.value, &relative)
            || relative.starts_with(&(pattern.value.clone() + "/"))
    } else {
        wildcard_match(&pattern.value, name)
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let mut p = 0;
    let mut t = 0;
    let mut star = None;
    let mut match_after_star = 0;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            match_after_star = t;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            match_after_star += 1;
            t = match_after_star;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::{
        list_folder_markdown_files, read_dir_children, read_sample_document,
        snapshot_directory_entries, write_markdown_file, IgnoreRules,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn reads_sample_document_from_resource_path() {
        let root = unique_test_dir("read-sample");
        let resource_path = root.join("resource.md");
        let sample_markdown = "# Nomo Markdown 全元素实例\n\n示例内容";
        fs::create_dir_all(&root).expect("create root");
        fs::write(&resource_path, sample_markdown).expect("write resource");

        let document = read_sample_document(&resource_path).expect("read");

        // 直接读取资源路径，不再复制到任何其他目录
        assert_eq!(document.path, resource_path.to_string_lossy().to_string());
        assert_eq!(document.file_name, "resource.md");
        assert_eq!(document.markdown, sample_markdown);

        cleanup(root);
    }

    #[test]
    fn fails_when_sample_resource_missing() {
        let root = unique_test_dir("read-sample-missing");
        let resource_path = root.join("missing.md");
        fs::create_dir_all(&root).expect("create root");

        let result = read_sample_document(&resource_path);
        assert!(result.is_err());

        cleanup(root);
    }

    #[test]
    fn write_markdown_file_overwrites_existing_file_with_payload() {
        let root = unique_test_dir("atomic-write-overwrite");
        let file_path = root.join("note.md");
        fs::create_dir_all(&root).expect("create root");
        fs::write(&file_path, "old").expect("write old");

        let payload =
            write_markdown_file(file_path.to_string_lossy().to_string(), "# 新内容".into())
                .expect("write markdown");

        assert_eq!(payload.path, file_path.to_string_lossy().to_string());
        assert_eq!(payload.file_name, "note.md");
        assert_eq!(payload.markdown, "# 新内容");
        assert_eq!(
            fs::read_to_string(&file_path).expect("read saved"),
            "# 新内容"
        );

        cleanup(root);
    }

    #[test]
    fn write_markdown_file_removes_temporary_file_after_success() {
        let root = unique_test_dir("atomic-write-cleanup");
        let file_path = root.join("note.md");
        fs::create_dir_all(&root).expect("create root");

        write_markdown_file(file_path.to_string_lossy().to_string(), "content".into())
            .expect("write markdown");

        let temp_files: Vec<_> = fs::read_dir(&root)
            .expect("read root")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(temp_files.is_empty());

        cleanup(root);
    }

    #[test]
    fn folder_listing_includes_segmented_text_and_json_documents() {
        let root = unique_test_dir("folder-list-segmented");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("note.md"), "# note").expect("write markdown");
        fs::write(root.join("large.txt"), "plain text").expect("write text");
        fs::write(root.join("data.json"), "{}").expect("write json");
        fs::write(root.join("ignored.bin"), [0_u8]).expect("write binary");

        let files = list_folder_markdown_files(root.to_string_lossy().to_string())
            .expect("list supported documents");
        let names = files.into_iter().map(|file| file.name).collect::<Vec<_>>();

        assert_eq!(names, vec!["data.json", "large.txt", "note.md"]);
        cleanup(root);
    }

    #[test]
    fn directory_snapshot_releases_parent_directory_handle() {
        let root = unique_test_dir("directory-snapshot-handle");
        let moved = root.with_extension("moved");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("note.md"), "# note").expect("write markdown");

        let snapshots = snapshot_directory_entries(&root).expect("snapshot directory");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "note.md");

        // Windows 只有在 ReadDir 已释放后才能稳定重命名目录；快照仍存活但只持有路径数据。
        fs::rename(&root, &moved).expect("rename directory after snapshot");
        assert!(snapshots[0].path.ends_with("note.md"));

        cleanup(moved);
    }

    #[test]
    fn directory_listing_is_shallow_and_preserves_filtering_sorting_and_chevrons() {
        let root = unique_test_dir("directory-listing-shallow");
        let empty_dir = root.join("a-empty");
        let visible_dir = root.join("B-visible");
        let deep_dir = visible_dir.join("deep");
        let ignored_dir = root.join("node_modules");
        fs::create_dir_all(&empty_dir).expect("create empty dir");
        fs::create_dir_all(&deep_dir).expect("create deep dir");
        fs::create_dir_all(&ignored_dir).expect("create ignored dir");
        fs::write(deep_dir.join("nested.md"), "# nested").expect("write nested markdown");
        fs::write(ignored_dir.join("ignored.md"), "# ignored").expect("write ignored markdown");
        fs::write(root.join("z.md"), "# z").expect("write z markdown");
        fs::write(root.join("A.txt"), "a").expect("write text");
        fs::write(root.join("ignored.bin"), [0_u8]).expect("write binary");

        let ignore_rules = IgnoreRules::load(&root, Some(&root));
        let entries = read_dir_children(&root, &root, &ignore_rules).expect("read children");
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["a-empty", "B-visible", "A.txt", "z.md"]);
        assert!(!entries[0].has_children);
        assert!(entries[1].has_children);
        assert!(entries[1].children.is_empty());
        assert!(!names.contains(&"deep"));
        assert!(!names.contains(&"node_modules"));

        cleanup(root);
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nomo-file-system-{name}-{nonce}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
