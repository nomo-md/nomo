//! 文件树的后台扫描与刷新生命周期，所有结果均按当前根目录代次核验。

use super::*;
use crate::ui::file_tree::{self, ScanError};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct TreeScan {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
}

impl Drop for TreeScan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(super) struct ExplorerState {
    generation: u64,
    scan: Option<TreeScan>,
    queue: VecDeque<PathBuf>,
    scan_again: HashSet<PathBuf>,
    failed: HashSet<PathBuf>,
    refresh_task: Option<Task<()>>,
    pub(super) hovered: Option<PathBuf>,
    pub(super) scroll_visible: bool,
    pub(super) scroll_animation: Option<(Instant, usize, f32, f32)>,
    pub(super) scroll_animation_id: usize,
    pub(super) scroll_near: bool,
    pub(super) scroll_dragging: bool,
    pub(super) scroll_drag_anchor: f32,
    scroll_hide_task: Option<Task<()>>,
    pub(super) menu: Option<super::explorer_actions::TreeMenu>,
    pub(super) edit: Option<super::explorer_actions::TreeEdit>,
    pub(super) search_field: Option<Entity<TextField>>,
    pub(super) search_subscription: Option<Subscription>,
    pub(super) search_blur: Option<Subscription>,
    pub(super) search_focus: Option<Subscription>,
    pub(super) search_query: String,
    pub(super) search_matches: Vec<usize>,
    pub(super) search_index: usize,
    pub(super) search_focused: Option<PathBuf>,
    pub(super) search_visible: bool,
    pub(super) search_keyboard_nav: bool,
    pub(super) search_revision: u64,
    pub(super) search_task: Option<Task<()>>,
    pub(super) mutating: HashSet<PathBuf>,
    pub(super) sidebar_width: Option<f32>,
    pub(super) reduce_motion: bool,
    pub(super) resizing: bool,
    pub(super) sidebar_target: Option<bool>,
    pub(super) sidebar_motion: Option<(Instant, usize, f32, f32, Duration)>,
    pub(super) sidebar_motion_id: usize,
    pub(super) sidebar_motion_task: Option<Task<()>>,
}

impl ExplorerState {
    pub(super) fn generation(&self) -> u64 { self.generation }
}

impl Workspace {
    pub(super) fn replace_tree(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.explorer.generation = self.explorer.generation.wrapping_add(1);
        // 旧扫描即使尚未从系统调用返回，也只允许它和新请求串行，避免连续切换根目录时积累后台任务。
        if let Some(scan) = &self.explorer.scan { scan.cancel.store(true, Ordering::Release); }
        self.explorer.queue.clear();
        self.explorer.scan_again.clear();
        self.explorer.failed.clear();
        self.explorer.hovered = None;
        self.explorer.menu = None;
        self.explorer.edit = None;
        self.clear_tree_search(cx);
        self.explorer.search_focused = None;
        self.explorer.scroll_dragging = false;
        self.tree_scroll.0.borrow().base_handle.set_offset(gpui::point(px(0.0), px(0.0)));
        self.tree = Some(FileTree::new(root.clone()));
        self.tree_animation = None;
        self.sync_tree_selection(false, cx);
        self.queue_tree_scan(root, false, cx);
        self.ensure_tree_refresh_task(cx);
    }

    pub(super) fn show_tree_scrollbar(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.set_tree_scrollbar_visible(true);
        self.explorer.scroll_hide_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |workspace, cx| {
                if !workspace.explorer.scroll_dragging && !workspace.explorer.scroll_near {
                    workspace.set_tree_scrollbar_visible(false);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn set_tree_scrollbar_visible(&mut self, visible: bool) {
        if self.explorer.scroll_visible == visible { return; }
        if self.explorer.reduce_motion {
            self.explorer.scroll_visible = visible;
            self.explorer.scroll_animation = None;
            return;
        }
        let from = self.explorer.scroll_animation.map(|(started, _, from, to)| {
            let progress = (started.elapsed().as_secs_f32() / 0.12).min(1.0);
            from + (to - from) * ease_in_out(progress)
        }).unwrap_or(if self.explorer.scroll_visible { 1.0 } else { 0.0 });
        self.explorer.scroll_visible = visible;
        self.explorer.scroll_animation_id = self.explorer.scroll_animation_id.wrapping_add(1);
        self.explorer.scroll_animation = Some((Instant::now(), self.explorer.scroll_animation_id, from, if visible { 1.0 } else { 0.0 }));
    }

    pub(super) fn queue_tree_scan(&mut self, path: PathBuf, force: bool, cx: &mut Context<Self>) {
        if force { self.explorer.failed.remove(&path); }
        else if self.explorer.failed.contains(&path) { return; }
        let Some(tree) = &mut self.tree else { return; };
        if tree.loading(&path) {
            if force { self.explorer.scan_again.insert(path); }
            return;
        }
        if tree.begin_load(&path, force) {
            self.explorer.queue.push_back(path);
            self.start_next_tree_scan(cx);
        }
    }

    pub(super) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.explorer.failed.clear();
        let paths = self.tree.as_ref().map(FileTree::refresh_paths).unwrap_or_default();
        for path in paths {
            self.queue_tree_scan(path, true, cx);
        }
        cx.notify();
    }

    pub(super) fn cancel_hidden_tree_scan(&mut self) {
        if let (Some(scan), Some(tree)) = (&self.explorer.scan, &self.tree) {
            if !tree.is_visible_directory(&scan.path) { scan.cancel.store(true, Ordering::Release); }
        }
    }

    pub(super) fn apply_tree_rename(&mut self, paths: &[(PathBuf, PathBuf)], cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return; };
        if !paths.iter().any(|(old, _)| old.starts_with(&tree.root) || tree.root.starts_with(old)) { return; }
        self.explorer.generation = self.explorer.generation.wrapping_add(1);
        if let Some(scan) = &self.explorer.scan { scan.cancel.store(true, Ordering::Release); }
        self.explorer.queue.clear();
        self.explorer.scan_again.clear();
        self.explorer.failed.clear();
        if let Some(tree) = &mut self.tree {
            tree.cancel_all_loads();
            for (old, new) in paths { tree.rename_subtree(old, new); }
        }
        self.refresh_tree(cx);
        self.sync_tree_selection(false, cx);
        self.recompute_tree_search(cx);
    }

    fn ensure_tree_refresh_task(&mut self, cx: &mut Context<Self>) {
        if self.explorer.refresh_task.is_some() { return; }
        self.explorer.refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this.update(cx, |workspace, cx| {
                    if workspace.tree.is_some() && cx.global::<Settings>().show_sidebar {
                        workspace.refresh_tree(cx);
                    }
                }).is_err() { break; }
            }
        }));
    }

    fn start_next_tree_scan(&mut self, cx: &mut Context<Self>) {
        if self.explorer.scan.is_some() { return; }
        let Some(tree) = &mut self.tree else {
            self.explorer.queue.clear();
            return;
        };
        let path = loop {
            let Some(path) = self.explorer.queue.pop_front() else { return; };
            if tree.is_visible_directory(&path) { break path; }
            tree.cancel_load(&path);
            self.explorer.scan_again.remove(&path);
        };
        let root = tree.root.clone();
        let previous = tree.cached_entries(&path);
        let generation = self.explorer.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.explorer.scan = Some(TreeScan { path: path.clone(), cancel: cancel.clone() });
        let scan_path = path.clone();
        let task = cx.background_executor().spawn(async move {
            let mut snapshot = file_tree::scan_directory(&root, &scan_path, &cancel)?;
            snapshot.compare_previous(previous);
            Ok(snapshot)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.explorer.scan.as_ref().is_none_or(|scan| scan.path != path) {
                    return;
                }
                workspace.explorer.scan = None;
                if workspace.explorer.generation != generation {
                    workspace.start_next_tree_scan(cx);
                    cx.notify();
                    return;
                }
                let before = workspace.tree.as_ref().map(FileTree::rows);
                let mut failed = false;
                if let Some(tree) = &mut workspace.tree {
                    match result {
                        Ok(snapshot) => {
                            if let Err(error) = tree.complete_load(&path, snapshot) {
                                failed = true;
                                workspace.notice = Some(format!("{}：{error}", path.display()).into());
                            }
                        }
                        Err(ScanError::Cancelled) => tree.cancel_load(&path),
                        Err(ScanError::Failed(error)) => {
                            failed = true;
                            tree.cancel_load(&path);
                            workspace.notice = Some(format!("{}：{error}", path.display()).into());
                        }
                    }
                }
                if failed { workspace.explorer.failed.insert(path.clone()); }
                else { workspace.explorer.failed.remove(&path); }
                let again = workspace.explorer.scan_again.remove(&path);
                let needed = workspace.tree.as_ref().map(|tree| {
                    tree.refresh_paths().into_iter().filter(|candidate| !workspace.explorer.failed.contains(candidate) && tree.needs_load(candidate)).collect::<Vec<_>>()
                }).unwrap_or_default();
                if again { workspace.queue_tree_scan(path, true, cx); }
                for path in needed { workspace.queue_tree_scan(path, false, cx); }
                if before.as_ref().zip(workspace.tree.as_ref().map(FileTree::rows).as_ref())
                    .is_some_and(|(before, after)| !Arc::ptr_eq(before, after)) {
                    workspace.sync_tree_selection(false, cx);
                    workspace.recompute_tree_search(cx);
                }
                workspace.start_next_tree_scan(cx);
                cx.notify();
            });
        }).detach();
    }
}

#[cfg(target_os = "windows")]
pub(super) fn system_reduce_motion() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" { fn SystemParametersInfoW(action: u32, param: u32, value: *mut std::ffi::c_void, flags: u32) -> i32; }
    let mut animations_enabled: i32 = 1;
    unsafe { SystemParametersInfoW(0x1042, 0, (&mut animations_enabled as *mut i32).cast(), 0) != 0 && animations_enabled == 0 }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn system_reduce_motion() -> bool { false }
