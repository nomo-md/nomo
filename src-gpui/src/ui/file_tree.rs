//! 侧栏目录快照：后台按需读取，保留展开状态并限制目录缓存的总字节数。

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_normalization::UnicodeNormalization;

/// 缓存估算包含路径、名称、搜索键、Arc 和两份可见行索引；刷新时最多并存一份新快照。
pub const TREE_CACHE_BYTES: usize = 32 * 1024 * 1024;
const DIRECTORY_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const EXPANDED_PATH_BYTES: usize = 2 * 1024 * 1024;
const IGNORE_FILE_BYTES: u64 = 256 * 1024;
const DOCUMENT_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "json"];

#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub resolved_path: Option<PathBuf>,
    pub name: String,
    pub search_name: String,
    pub is_dir: bool,
    pub has_children: bool,
}

#[derive(Clone)]
pub struct Row {
    pub depth: usize,
    pub entry: Arc<Entry>,
    pub expanded: bool,
    pub loading: bool,
}

struct CachedDirectory {
    entries: Arc<[Arc<Entry>]>,
    bytes: usize,
    touched: u64,
}

pub struct DirectorySnapshot {
    entries: Arc<[Arc<Entry>]>,
    bytes: usize,
    unchanged_from: Option<Arc<[Arc<Entry>]>>,
}

impl DirectorySnapshot {
    /// 对比目录快照在后台执行，避免大目录的逐项比较占用界面线程。
    pub fn compare_previous(&mut self, previous: Option<Arc<[Arc<Entry>]>>) {
        self.unchanged_from = previous.filter(|entries| entries.as_ref() == self.entries.as_ref());
    }
}

#[derive(Debug)]
pub enum ScanError {
    Cancelled,
    Failed(String),
}

pub struct FileTree {
    pub root: PathBuf,
    expanded: HashSet<PathBuf>,
    expanded_bytes: usize,
    children: HashMap<PathBuf, CachedDirectory>,
    cache_bytes: usize,
    clock: u64,
    loading: HashSet<PathBuf>,
    rows: Arc<[Row]>,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());
        Self {
            expanded_bytes: path_cost(&root),
            root,
            expanded,
            children: HashMap::new(),
            cache_bytes: 0,
            clock: 0,
            loading: HashSet::new(),
            rows: Arc::from([]),
        }
    }

    pub fn root_name(&self) -> String {
        self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |name| name.to_string_lossy().nfc().collect(),
        )
    }

    pub fn rows(&self) -> Arc<[Row]> {
        self.rows.clone()
    }

    pub fn cached_entries(&self, path: &Path) -> Option<Arc<[Arc<Entry>]>> {
        self.children.get(path).map(|directory| directory.entries.clone())
    }

    pub fn root_expanded(&self) -> bool {
        self.expanded.contains(&self.root)
    }

    pub fn toggle_root(&mut self) -> Result<bool, String> {
        let root = self.root.clone();
        self.toggle(&root)
    }

    pub fn toggle(&mut self, path: &Path) -> Result<bool, String> {
        if self.expanded.remove(path) {
            self.expanded_bytes = self.expanded_bytes.saturating_sub(path_cost(path));
            self.rebuild();
            return Ok(false);
        }
        let cost = path_cost(path);
        if self.expanded_bytes.saturating_add(cost) > EXPANDED_PATH_BYTES {
            return Err("展开的目录过多，请先折叠部分目录。".into());
        }
        self.expanded.insert(path.to_path_buf());
        self.expanded_bytes += cost;
        self.clock = self.clock.wrapping_add(1);
        if let Some(cached) = self.children.get_mut(path) { cached.touched = self.clock; }
        self.rebuild();
        Ok(true)
    }

    pub fn expand(&mut self, path: &Path) -> Result<(), String> {
        if !self.expanded.contains(path) {
            self.toggle(path)?;
        }
        Ok(())
    }

    pub fn collapse_all(&mut self) {
        let root_expanded = self.root_expanded();
        self.expanded.clear();
        if root_expanded { self.expanded.insert(self.root.clone()); }
        self.expanded_bytes = if root_expanded { path_cost(&self.root) } else { 0 };
        self.rebuild();
    }

    pub fn rename_subtree(&mut self, old: &Path, new: &Path) {
        let next_root = self.root.strip_prefix(old).ok().map(|suffix| {
            if suffix.as_os_str().is_empty() { new.to_path_buf() } else { new.join(suffix) }
        });
        let mut affected: Vec<_> = self.expanded.iter().filter(|path| path.starts_with(old)).cloned().collect();
        affected.sort_by_key(|path| usize::from(path != &self.root));
        for path in &affected {
            self.expanded.remove(path);
            self.expanded_bytes = self.expanded_bytes.saturating_sub(path_cost(path));
        }
        for path in affected {
            if let Ok(suffix) = path.strip_prefix(old) {
                let replacement = new.join(suffix);
                let cost = path_cost(&replacement);
                if self.expanded_bytes.saturating_add(cost) <= EXPANDED_PATH_BYTES {
                    self.expanded.insert(replacement);
                    self.expanded_bytes += cost;
                }
            }
        }
        let cached: Vec<_> = self.children.keys().filter(|path| path.starts_with(old)).cloned().collect();
        for path in cached {
            if let Some(directory) = self.children.remove(&path) { self.cache_bytes = self.cache_bytes.saturating_sub(directory.bytes); }
        }
        if let Some(root) = next_root { self.root = root; }
        self.rebuild();
    }

    pub fn is_visible_directory(&self, path: &Path) -> bool {
        if !path.starts_with(&self.root) || !self.expanded.contains(path) {
            return false;
        }
        let mut cursor = path;
        while cursor != self.root {
            let Some(parent) = cursor.parent() else { return false; };
            if !self.expanded.contains(parent) { return false; }
            let Some(directory) = self.children.get(parent) else { return false; };
            let Some(name) = cursor.file_name() else { return false; };
            let name: String = name.to_string_lossy().to_lowercase().nfc().collect();
            let start = directory.entries.partition_point(|entry| entry.is_dir && entry.search_name < name);
            if !directory.entries[start..].iter().take_while(|entry| entry.is_dir && entry.search_name == name)
                .any(|entry| entry.path == cursor) { return false; }
            cursor = parent;
        }
        true
    }

    pub fn needs_load(&self, path: &Path) -> bool {
        self.is_visible_directory(path) && !self.children.contains_key(path) && !self.loading.contains(path)
    }

    pub fn begin_load(&mut self, path: &Path, force: bool) -> bool {
        if !self.is_visible_directory(path) || self.loading.contains(path) || (!force && self.children.contains_key(path)) {
            return false;
        }
        self.loading.insert(path.to_path_buf());
        if !self.children.contains_key(path) { self.rebuild(); }
        true
    }

    pub fn cancel_load(&mut self, path: &Path) {
        self.loading.remove(path);
        if !self.children.contains_key(path) { self.rebuild(); }
    }

    pub fn cancel_all_loads(&mut self) {
        if !self.loading.is_empty() { self.loading.clear(); self.rebuild(); }
    }

    pub fn loading(&self, path: &Path) -> bool {
        self.loading.contains(path) && !self.children.contains_key(path)
    }

    /// 仅刷新根目录与当前可见、已展开的目录，折叠分支保留展开记录但不后台扫描。
    pub fn refresh_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if self.root_expanded() { paths.push(self.root.clone()); }
        paths.extend(self.rows.iter().filter(|row| row.entry.is_dir && row.expanded).map(|row| row.entry.path.clone()));
        paths
    }

    pub fn complete_load(&mut self, path: &Path, snapshot: DirectorySnapshot) -> Result<(), String> {
        self.loading.remove(path);
        if !self.is_visible_directory(path) {
            self.rebuild();
            return Ok(());
        }
        if snapshot.unchanged_from.as_ref().is_some_and(|previous| {
            self.children.get(path).is_some_and(|cached| Arc::ptr_eq(&cached.entries, previous))
        }) {
            self.clock = self.clock.wrapping_add(1);
            if let Some(cached) = self.children.get_mut(path) { cached.touched = self.clock; }
            return Ok(());
        }
        let old = self.children.get(path).map_or(0, |entry| entry.bytes);
        while self.cache_bytes.saturating_sub(old).saturating_add(snapshot.bytes) > TREE_CACHE_BYTES {
            let victim = self.children.iter()
                .filter(|(candidate, _)| candidate.as_path() != path && !self.is_visible_directory(candidate))
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(candidate, _)| candidate.clone());
            let Some(victim) = victim else {
                self.rebuild();
                return Err("文件树缓存已达到 32 MiB，请折叠部分目录后重试。".into());
            };
            if let Some(entry) = self.children.remove(&victim) {
                self.cache_bytes = self.cache_bytes.saturating_sub(entry.bytes);
            }
        }
        if let Some(previous) = self.children.remove(path) {
            self.cache_bytes = self.cache_bytes.saturating_sub(previous.bytes);
        }
        self.clock = self.clock.wrapping_add(1);
        self.cache_bytes = self.cache_bytes.saturating_add(snapshot.bytes);
        self.children.insert(path.to_path_buf(), CachedDirectory {
            entries: snapshot.entries,
            bytes: snapshot.bytes,
            touched: self.clock,
        });
        self.rebuild();
        Ok(())
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if self.root_expanded() {
            self.walk(&self.root, 1, &mut rows);
        }
        self.rows = rows.into();
    }

    fn walk(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let Some(children) = self.children.get(dir) else { return; };
        for entry in children.entries.iter() {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            rows.push(Row {
                depth,
                entry: entry.clone(),
                expanded,
                loading: self.loading(&entry.path),
            });
            if expanded {
                self.walk(&entry.path, depth + 1, rows);
            }
        }
    }
}

fn path_cost(path: &Path) -> usize {
    path.as_os_str().len().saturating_mul(2).saturating_add(96)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), ScanError> {
    if cancel.load(Ordering::Acquire) { Err(ScanError::Cancelled) } else { Ok(()) }
}

/// 单次扫描只持有一个有界快照；目录变更通过取消标记和调用方的代次失效。
pub fn scan_directory(root: &Path, dir: &Path, cancel: &AtomicBool) -> Result<DirectorySnapshot, ScanError> {
    check_cancel(cancel)?;
    let rules = IgnoreRules::load(root, dir)?;
    let iter = std::fs::read_dir(dir).map_err(|error| ScanError::Failed(format!("读取目录失败：{error}")))?;
    let mut entries = Vec::new();
    let mut bytes = path_cost(dir);
    let resolved_dir = dir.canonicalize().ok();
    let through_link = resolved_dir.as_ref().is_some_and(|resolved| resolved != dir);
    for item in iter {
        check_cancel(cancel)?;
        let item = item.map_err(|error| ScanError::Failed(format!("读取目录项失败：{error}")))?;
        let raw_name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        let kind = item.file_type().map_err(|error| ScanError::Failed(format!("读取目录项类型失败：{error}")))?;
        let linked = kind.is_symlink();
        let metadata = if linked { std::fs::metadata(&path).ok() } else { None };
        let is_dir = if linked { metadata.as_ref().is_some_and(|metadata| metadata.is_dir()) } else { kind.is_dir() };
        let is_file = if linked { metadata.as_ref().is_some_and(|metadata| metadata.is_file()) } else { kind.is_file() };
        if rules.is_ignored(root, &path, &raw_name, is_dir) {
            continue;
        }
        if !is_dir && !(is_file && supported_document(&path)) {
            continue;
        }
        let resolved_path = (linked || through_link).then(|| path.canonicalize().ok()).flatten();
        let name: String = raw_name.nfc().collect();
        let search_name: String = name.to_lowercase().nfc().collect();
        let cyclic = linked && is_dir && resolved_path.as_ref().is_some_and(|target| {
            dir.ancestors().take_while(|ancestor| ancestor.starts_with(root))
                .any(|ancestor| ancestor.canonicalize().ok().as_ref() == Some(target))
        });
        bytes = bytes.saturating_add(path_cost(&path))
            .saturating_add(resolved_path.as_deref().map_or(0, path_cost))
            .saturating_add(name.len())
            .saturating_add(search_name.len())
            .saturating_add(std::mem::size_of::<Entry>() + 2 * std::mem::size_of::<Row>() + 64);
        if bytes > DIRECTORY_SNAPSHOT_BYTES {
            return Err(ScanError::Failed("单个目录的文件树快照超过 16 MiB，无法完整显示。".into()));
        }
        entries.push((Entry { path, resolved_path, name, search_name, is_dir, has_children: false }, cyclic));
    }
    // 先释放父目录迭代器，再探测子目录，避免长时间占用目录句柄。
    for (entry, cyclic) in &mut entries {
        if entry.is_dir && !*cyclic { entry.has_children = has_visible_children(root, &entry.path, &rules, cancel)?; }
    }
    entries.sort_by(|(a, _), (b, _)| b.is_dir.cmp(&a.is_dir).then_with(|| a.search_name.cmp(&b.search_name)));
    check_cancel(cancel)?;
    Ok(DirectorySnapshot { entries: entries.into_iter().map(|(entry, _)| Arc::new(entry)).collect::<Vec<_>>().into(), bytes, unchanged_from: None })
}

fn has_visible_children(root: &Path, dir: &Path, rules: &IgnoreRules, cancel: &AtomicBool) -> Result<bool, ScanError> {
    let iter = match std::fs::read_dir(dir) {
        Ok(iter) => iter,
        // 保留展开入口，让用户在展开时看到明确的读取错误。
        Err(_) => return Ok(true),
    };
    for item in iter {
        check_cancel(cancel)?;
        let item = match item { Ok(item) => item, Err(_) => return Ok(true) };
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        let kind = match item.file_type() { Ok(kind) => kind, Err(_) => return Ok(true) };
        let metadata = if kind.is_symlink() { std::fs::metadata(&path).ok() } else { None };
        let is_dir = if kind.is_symlink() { metadata.as_ref().is_some_and(|value| value.is_dir()) } else { kind.is_dir() };
        let is_file = if kind.is_symlink() { metadata.as_ref().is_some_and(|value| value.is_file()) } else { kind.is_file() };
        if !rules.is_ignored(root, &path, &name, is_dir)
            && (is_dir || (is_file && supported_document(&path))) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn supported_document(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str())
        .is_some_and(|ext| DOCUMENT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

struct IgnorePattern {
    value: String,
    negated: bool,
    directory_only: bool,
}

struct IgnoreRules {
    patterns: Vec<IgnorePattern>,
}

impl IgnoreRules {
    fn load(root: &Path, dir: &Path) -> Result<Self, ScanError> {
        let mut patterns: Vec<_> = [
            ".git/", ".hg/", ".svn/", "node_modules/", "target/", "dist/",
            "build/", "out/", "coverage/", ".next/", ".svelte-kit/", ".turbo/", ".cache/",
        ].into_iter().filter_map(parse_ignore).collect();
        append_ignore_file(root, &mut patterns)?;
        if dir != root { append_ignore_file(dir, &mut patterns)?; }
        Ok(Self { patterns })
    }

    fn is_ignored(&self, root: &Path, path: &Path, name: &str, is_dir: bool) -> bool {
        let relative = path.strip_prefix(root).ok()
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| name.replace('\\', "/"));
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.directory_only && !is_dir { continue; }
            let matched = if pattern.value.contains('/') {
                wildcard_match(&pattern.value, &relative) || relative.starts_with(&(pattern.value.clone() + "/"))
            } else {
                wildcard_match(&pattern.value, name)
            };
            if matched { ignored = !pattern.negated; }
        }
        ignored
    }
}

fn append_ignore_file(dir: &Path, patterns: &mut Vec<IgnorePattern>) -> Result<(), ScanError> {
    let mut file = match std::fs::File::open(dir.join(".gitignore")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ScanError::Failed(format!("读取忽略规则失败：{error}"))),
    };
    let mut bytes = Vec::new();
    file.by_ref().take(IGNORE_FILE_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|error| ScanError::Failed(format!("读取忽略规则失败：{error}")))?;
    if bytes.len() as u64 > IGNORE_FILE_BYTES {
        return Err(ScanError::Failed("忽略规则文件超过 256 KiB。".into()));
    }
    let text = String::from_utf8(bytes).map_err(|error| ScanError::Failed(format!("忽略规则编码无效：{error}")))?;
    patterns.extend(text.lines().filter_map(parse_ignore));
    Ok(())
}

fn parse_ignore(line: &str) -> Option<IgnorePattern> {
    let mut value = line.trim();
    if value.is_empty() || value.starts_with('#') { return None; }
    let negated = value.starts_with('!');
    if negated { value = value.trim_start_matches('!').trim(); }
    if value.is_empty() { return None; }
    let directory_only = value.ends_with('/');
    let value = value.trim_start_matches('/').trim_end_matches('/').replace('\\', "/");
    (!value.is_empty()).then_some(IgnorePattern { value, negated, directory_only })
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t, mut star, mut after_star) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            after_star = t;
        } else if let Some(index) = star {
            p = index + 1;
            after_star += 1;
            t = after_star;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' { p += 1; }
    p == pattern.len()
}
