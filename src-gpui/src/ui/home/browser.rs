//! 主页分组入口与文件夹卡片预览；只扫描可见文件夹的直接子项。
use super::*;
use crate::ui::file_tree::{self, ScanError};
use std::collections::HashSet;

const DIRECTORY_CACHE_BYTES: usize = 16 * 1024 * 1024;
const DIRECTORY_LIMIT: usize = 128;

#[derive(Clone)]
pub(super) struct Node {
    pub entry: Entry,
    pub root: Arc<Path>,
    pub directory: bool,
}

impl Node {
    pub fn drag(&self) -> crate::ui::path_drag::PathDrag {
        crate::ui::path_drag::PathDrag {
            path: self.entry.path.clone(),
            root: if self.entry.path.as_path() == self.root.as_ref() {
                self.entry.path.parent().unwrap_or(&self.entry.path).to_path_buf()
            } else { self.root.to_path_buf() },
            directory: self.directory,
            title: if self.directory { entry_name(&self.entry.path) } else { document_name(&self.entry.path) },
            visual: Default::default(),
        }
    }
}

struct Directory {
    entries: Arc<[Arc<file_tree::Entry>]>,
    front: Option<Arc<file_tree::Entry>>,
    papers: usize,
    bytes: usize,
    used: u64,
    stale: bool,
}

#[derive(Default)]
pub(super) struct Browser {
    pub rows: Arc<[Node]>,
    pub grid: Arc<[Node]>,
    pub revision: u64,
    pub tree_revision: u64,
    pub error: Option<UiMessage>,
    directories: HashMap<PathBuf, Directory>,
    bytes: usize,
    clock: u64,
    generation: u64,
    queue: VecDeque<PathBuf>,
    failed: HashSet<PathBuf>,
    scan_again: HashSet<PathBuf>,
    running: Option<(PathBuf, Arc<AtomicBool>)>,
    worker: Option<Task<()>>,
}

impl Browser {
    pub(super) fn root_for(&self, path: &Path, snapshot: &Snapshot) -> Option<PathBuf> {
        snapshot.folders.iter().filter(|entry| path.starts_with(&entry.path))
            .max_by_key(|entry| entry.path.components().count()).map(|entry| entry.path.clone())
    }

    pub fn entries(&self, path: &Path) -> Option<&[Arc<file_tree::Entry>]> {
        self.directories.get(path).map(|directory| directory.entries.as_ref())
    }

    pub fn cover_document(&self, path: &Path) -> Option<&file_tree::Entry> {
        self.directories.get(path).and_then(|directory| directory.front.as_deref())
    }

    pub fn cover_papers(&self, path: &Path) -> usize {
        self.directories.get(path).map_or(0, |directory| directory.papers)
    }

    pub fn loading(&self, path: &Path) -> bool {
        self.queue.iter().any(|item| item == path) || self.running.as_ref().is_some_and(|(item, _)| item == path)
    }

    pub fn needs_directory(&self, path: &Path) -> bool {
        self.failed.len() < DIRECTORY_LIMIT && self.directories.get(path).is_none_or(|directory| directory.stale)
            && !self.loading(path) && !self.failed.contains(path)
    }

    pub fn suspend(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some((_, cancel)) = &self.running { cancel.store(true, Ordering::Release); }
        self.queue.clear();
        self.failed.clear();
        self.scan_again.clear();
        self.directories.clear();
        self.rows = Arc::from([]);
        self.grid = Arc::from([]);
        self.bytes = 0;
    }

    pub(super) fn rebuild(&mut self, snapshot: &Snapshot) {
        let grid: Vec<_> = snapshot.home_entries().into_iter().map(|(entry, directory)| Node {
            entry: entry.clone(),
            root: if directory { entry.path.as_path() } else { entry.path.parent().unwrap_or(&entry.path) }.into(),
            directory,
        }).collect();
        let rows: Vec<_> = grid.iter().filter(|node| node.directory)
            .chain(grid.iter().filter(|node| !node.directory)).cloned().collect();
        if self.rows.len() != rows.len() || self.rows.iter().zip(&rows).any(|(a, b)| a.entry.path != b.entry.path) {
            self.tree_revision = self.tree_revision.wrapping_add(1);
        }
        if self.grid.len() != grid.len() || self.grid.iter().zip(&grid).any(|(a, b)| a.entry.path != b.entry.path) {
            self.revision = self.revision.wrapping_add(1);
        }
        self.rows = rows.into();
        self.grid = grid.into();
    }
}

impl Home {
    pub(crate) fn root_for_file(&self, path: &Path) -> Option<PathBuf> {
        if self.snapshot.folders.iter().any(|entry| entry.path == path) { return None; }
        self.browser.root_for(path, &self.snapshot)
    }

    pub(super) fn sync_browser(&mut self, cx: &mut Context<Self>) {
        if !self.active { return; }
        self.browser.rebuild(&self.snapshot);
        let retired: Vec<_> = self.browser.directories.keys().filter(|path| self.browser.root_for(path, &self.snapshot).is_none()).cloned().collect();
        for path in retired {
            if let Some(directory) = self.browser.directories.remove(&path) { self.browser.bytes = self.browser.bytes.saturating_sub(directory.bytes); }
        }
        self.cancel_hidden_directory_scan();
        self.refresh_directories(cx);
    }

    fn directory_wanted(&self, path: &Path) -> bool {
        self.visible_folders.iter().any(|visible| visible == path)
    }

    pub(super) fn cancel_hidden_directory_scan(&mut self) {
        if let Some((path, cancel)) = &self.browser.running {
            if !self.directory_wanted(path) { cancel.store(true, Ordering::Release); }
        }
        self.browser.queue = self.browser.queue.iter().filter(|path| self.directory_wanted(path)).cloned().collect();
    }

    pub(super) fn refresh_directories(&mut self, cx: &mut Context<Self>) {
        if !self.active { return; }
        self.browser.failed.clear();
        let mut paths = self.visible_folders.clone();
        let mut seen = HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        for path in paths { self.queue_directory(path, true, cx); }
    }

    pub(super) fn queue_directory(&mut self, path: PathBuf, force: bool, cx: &mut Context<Self>) {
        if force {
            self.browser.failed.remove(&path);
            if let Some(directory) = self.browser.directories.get_mut(&path) { directory.stale = true; }
        }
        else if self.browser.failed.contains(&path) || self.browser.failed.len() >= DIRECTORY_LIMIT { return; }
        if force && self.browser.running.as_ref().is_some_and(|(running, _)| running == &path) {
            self.browser.scan_again.insert(path); return;
        }
        if !self.active || (!force && self.browser.directories.get(&path).is_some_and(|directory| !directory.stale)) || self.browser.loading(&path) { return; }
        if self.browser.queue.len() >= DIRECTORY_LIMIT { return; }
        self.browser.queue.push_back(path);
        self.start_directory_scan(cx);
    }

    fn start_directory_scan(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.browser.worker.is_some() { return; }
        let path = loop {
            let Some(path) = self.browser.queue.pop_front() else { return; };
            if self.directory_wanted(&path) { break path; }
        };
        let Some(root) = self.browser.root_for(&path, &self.snapshot) else { self.start_directory_scan(cx); return; };
        let generation = self.browser.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.browser.running = Some((path.clone(), cancel.clone()));
        let scan_path = path.clone();
        let scan_cancel = cancel.clone();
        let task = cx.background_executor().spawn(async move { file_tree::scan_directory_message(&root, &scan_path, &scan_cancel) });
        self.browser.worker = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |home, cx| {
                home.browser.worker = None;
                home.browser.running = None;
                if home.active && generation == home.browser.generation && !cancel.load(Ordering::Acquire) {
                    match result {
                        Ok(snapshot) => {
                            let (entries, bytes) = snapshot.into_entries();
                            let bytes = bytes.saturating_add(std::mem::size_of::<Option<Arc<file_tree::Entry>>>() + std::mem::size_of::<usize>());
                            let previous_bytes = home.browser.directories.get(&path).map_or(0, |entry| entry.bytes);
                            while home.browser.bytes.saturating_sub(previous_bytes) + bytes > DIRECTORY_CACHE_BYTES
                                || (!home.browser.directories.contains_key(&path) && home.browser.directories.len() >= DIRECTORY_LIMIT) {
                                let victim = home.browser.directories.iter()
                                    .filter(|(candidate, _)| **candidate != path && !home.visible_folders.contains(candidate))
                                    .min_by_key(|(_, dir)| dir.used).map(|(path, _)| path.clone());
                                let Some(victim) = victim else { break; };
                                if let Some(old) = home.browser.directories.remove(&victim) { home.browser.bytes -= old.bytes; }
                            }
                            if home.browser.bytes.saturating_sub(previous_bytes) + bytes <= DIRECTORY_CACHE_BYTES
                                && (home.browser.directories.contains_key(&path) || home.browser.directories.len() < DIRECTORY_LIMIT) {
                                home.browser.clock = home.browser.clock.wrapping_add(1);
                                home.browser.bytes = home.browser.bytes.saturating_sub(previous_bytes) + bytes;
                                // 只在目录快照到达时选前页，避免每帧遍历目录或为背页加载正文。
                                let mut documents = entries.iter().filter(|entry| !entry.is_dir);
                                let front = documents.next().cloned();
                                let papers = usize::from(front.is_some()) + documents.take(2).count();
                                home.browser.directories.insert(path.clone(), Directory { entries, front, papers, bytes, used: home.browser.clock, stale: false });
                            } else {
                                home.browser.failed.insert(path.clone());
                                home.browser.error = Some(UiMessage::new("tree.cacheBudget").arg("size", "16 MiB"));
                            }
                        }
                        Err(ScanError::Failed(error)) => { home.browser.failed.insert(path.clone()); home.browser.error = Some(error); },
                        Err(ScanError::Cancelled) => {},
                    }
                    home.browser.rebuild(&home.snapshot);
                    cx.notify();
                }
                if home.browser.scan_again.remove(&path) { home.queue_directory(path, true, cx); }
                else if home.active && home.directory_wanted(&path) && home.browser.needs_directory(&path) { home.queue_directory(path, false, cx); }
                home.start_directory_scan(cx);
                if home.active { cx.notify(); }
            });
        }));
    }

    pub(crate) fn paths_renamed(&mut self, paths: &[(PathBuf, PathBuf)], cx: &mut Context<Self>) {
        self.browser.generation = self.browser.generation.wrapping_add(1);
        if let Some((_, cancel)) = &self.browser.running { cancel.store(true, Ordering::Release); }
        self.browser.queue.clear();
        self.browser.failed.clear();
        self.browser.scan_again.clear();
        let retired: Vec<_> = self.browser.directories.keys().filter(|path| paths.iter().any(|(old, _)| path.starts_with(old))).cloned().collect();
        for path in retired {
            if let Some(directory) = self.browser.directories.remove(&path) { self.browser.bytes = self.browser.bytes.saturating_sub(directory.bytes); }
        }
        // 移动只使相关路径失效，其他卡片保留位图，避免整理文件时整页闪回骨架。
        let affected = |path: &Path| paths.iter().any(|(old, _)| path.starts_with(old));
        let previews: Vec<_> = self.previews.keys().filter(|path| affected(path)).cloned().collect();
        for path in previews { self.remove_preview(&path, cx); }
        self.pending.retain(|path| !affected(path));
        if let Some((path, cancel)) = &self.running { if affected(path) { cancel.store(true, Ordering::Release); } }
    }
}
