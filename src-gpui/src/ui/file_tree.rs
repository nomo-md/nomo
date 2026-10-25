//! 侧栏文件树：按需读取目录（只在展开时 `read_dir`），只列出子目录与 Markdown/文本文件，隐藏点开头的条目。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// 展开后的一行：缩进层级与条目。
#[derive(Clone)]
pub struct Row {
    pub depth: usize,
    pub entry: Entry,
    pub expanded: bool,
}

pub struct FileTree {
    pub root: PathBuf,
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
}

const DOCUMENT_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "txt"];
/// 单个目录最多列出的条目数，防止误开超大目录时界面卡顿。
const MAX_ENTRIES_PER_DIR: usize = 2000;

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut tree = Self { root, expanded: HashSet::new(), children: HashMap::new(), rows: Vec::new() };
        let root = tree.root.clone();
        tree.expanded.insert(root);
        tree.rebuild();
        tree
    }

    pub fn root_name(&self) -> String {
        self.root.file_name().map_or_else(|| self.root.display().to_string(), |name| name.to_string_lossy().into_owned())
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
        self.rebuild();
    }

    pub fn refresh(&mut self) {
        self.children.clear();
        self.rebuild();
    }

    fn read(&mut self, dir: &Path) -> Vec<Entry> {
        if let Some(entries) = self.children.get(dir) {
            return entries.clone();
        }
        let mut entries: Vec<Entry> = std::fs::read_dir(dir)
            .map(|iter| {
                iter.filter_map(Result::ok)
                    .filter_map(|item| {
                        let name = item.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        let path = item.path();
                        let is_dir = item.file_type().ok()?.is_dir();
                        let keep = is_dir
                            || path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| DOCUMENT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
                        keep.then_some(Entry { path, name, is_dir })
                    })
                    .take(MAX_ENTRIES_PER_DIR)
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.children.insert(dir.to_path_buf(), entries.clone());
        entries
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut rows);
        self.rows = rows;
    }

    fn walk(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        for entry in self.read(dir) {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            rows.push(Row { depth, entry, expanded });
            if expanded {
                self.walk(&path, depth + 1, rows);
            }
        }
    }
}
