//! 先保留原工作区的草稿和布局，再释放已挂载文档并恢复目标目录。
use super::*;

pub(super) struct FolderOpen {
    root: PathBuf,
    reveal: Option<PathBuf>,
    file_after_switch: Option<OpenRequest>,
}

fn matches_folder(saved: &Path, root: &Path) -> bool {
    saved == root || saved.canonicalize().is_ok_and(|path| path == root)
}

impl Store {
    fn for_folder(&self, root: &Path) -> Option<&SavedLayout> {
        // 只看该目录最新的记录；空记录或正在使用的记录不能回退到更早的布局。
        self.book.windows.iter().rev()
            .find(|saved| saved.folder.as_deref().is_some_and(|folder| matches_folder(folder, root)))
            .filter(|saved| !self.claims.get(&saved.id).is_some_and(|owner| owner.upgrade().is_some()))
    }
}

impl Workspace {
    pub(in crate::ui::workspace) fn folder_open_pending(&self) -> bool {
        self.comparison.layout.folder_open.is_some()
    }

    pub(in crate::ui::workspace) fn cancel_folder_open(&mut self) {
        self.comparison.layout.folder_open = None;
    }

    pub(in crate::ui::workspace) fn reuse_open_folder(
        &mut self, root: &Path, reveal: Option<&Path>, foreground: bool,
        window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        let current = window.window_handle();
        for handle in cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
            if gpui::AnyWindowHandle::from(handle) == current { continue; }
            let reused = handle.update(cx, |target, target_window, cx| {
                if target.folder_open_pending() || target.closing_window
                    || !target.tree.as_ref().is_some_and(|tree| {
                        matches_folder(&tree.root, root) && reveal.is_none_or(|path| path.starts_with(&tree.root))
                    }) {
                    return false;
                }
                if foreground {
                    if let Some(path) = reveal {
                        target.open_batch([OpenRequest {
                            path: Some(path.to_path_buf()),
                            favorite_folder: true,
                            ..Default::default()
                        }], target_window, cx);
                    } else if !target.tab_interaction_busy(target_window) {
                        target.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(root.to_path_buf()), cx));
                        target.show_folder_workspace(target_window, cx);
                    }
                    target.reveal_window(target_window, cx);
                }
                true
            }).unwrap_or(false);
            if reused { return true; }
        }
        false
    }

    pub(in crate::ui::workspace) fn open_favorite_folder(
        &mut self, path: PathBuf, foreground: bool, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if !foreground { return; }
        if !path.is_dir() {
            self.failure_notice = Some(favorite_folder_unavailable(&path, cx).into());
            cx.notify();
            return;
        }
        if self.explorer.mutating.iter().any(|busy| path.starts_with(busy)) {
            self.notice = Some(ToolbarLocale::current(cx).app_text("workspace.fileBusy").into());
            cx.notify();
            return;
        }
        if let Some(tree) = self.tree.as_ref().filter(|tree| path.starts_with(&tree.root)) {
            let root = tree.root.clone();
            if self.leave_mini(window, cx) { self.open_folder_workspace(root, Some(path), None, window, cx); }
            return;
        }
        // 文件树使用目录项原路径；链接目录在所属工作区中按链接位置显示。
        let root = self.library.read(cx).snapshot.folders.iter()
            .filter(|entry| path.starts_with(&entry.path))
            .max_by_key(|entry| entry.path.components().count())
            .map(|entry| entry.path.clone());
        let (root, reveal) = if let Some(root) = root {
            (root, path)
        } else {
            let root = path.canonicalize().unwrap_or(path);
            (root.clone(), root)
        };
        if !self.reuse_open_folder(&root, Some(&reveal), true, window, cx) && self.leave_mini(window, cx) {
            self.open_folder_workspace(root, Some(reveal), None, window, cx);
        }
    }

    pub(in crate::ui::workspace) fn open_folder_workspace(
        &mut self, root: PathBuf, reveal: Option<PathBuf>, file_after_switch: Option<OpenRequest>,
        window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.tab_interaction_busy(window) { return; }
        if self.tree.as_ref().is_some_and(|tree| matches_folder(&tree.root, &root)) {
            self.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(root), cx));
            self.show_folder_workspace(window, cx);
            if let Some(path) = reveal { self.reveal_tree_folder(path, cx); }
            self.resume_folder_requests(file_after_switch, window, cx);
            return;
        }
        self.comparison.layout.folder_open = Some(FolderOpen { root, reveal, file_after_switch });
        self.start_recovery_catalog(cx);
        self.continue_folder_open(window, cx);
    }

    pub(in crate::ui::workspace) fn continue_folder_open(
        &mut self, window: &mut Window, cx: &mut Context<Self>,
    ) {
        let Some(pending) = &self.comparison.layout.folder_open else { return; };
        if self.comparison.closing.is_some() || !self.opening.is_empty() || self.pending_document_transition.is_some()
            || window.has_active_prompt() { return; }
        // 两个现有后台任务完成时直接继续，不增加轮询或常驻任务。
        if cx.try_global::<Store>().is_some_and(|store| store.importing) || self.recovery_catalog_loading(cx) {
            return;
        }
        let root = pending.root.clone();
        let error = if !root.is_dir() {
            Some(ToolbarLocale::current(cx).app_text("layout.folderUnavailable").to_owned())
        } else if let Some(path) = pending.reveal.as_ref().filter(|path| !path.is_dir()) {
            Some(favorite_folder_unavailable(path, cx))
        } else {
            cx.try_global::<Store>().and_then(|store| store.error.as_ref())
                .map(|error| error.render(ToolbarLocale::current(cx)))
                .or_else(|| self.layout_recovery_candidate("", cx).err())
        };
        if let Some(error) = error {
            self.failure_notice = Some(error.into());
            self.cancel_close(cx);
            cx.notify();
            return;
        }
        let keys: Vec<_> = self.tabs.iter().map(|tab| tab.key).collect();
        if !self.prepare_members(&keys, true, window, cx, |workspace, window, cx| {
            workspace.continue_folder_open(window, cx);
        }) { return; }

        // 布局包含未加载标签的引用；只为已挂载文档办理会话交接。
        self.comparison.layout.flush.take();
        self.claim_layout(cx);
        self.close_workspace_documents(keys, window, cx);
    }

    pub(in crate::ui::workspace) fn finish_folder_open(
        &mut self, window: &mut Window, cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.comparison.layout.folder_open.take() else { return; };
        if cx.try_global::<Store>().is_some() {
            cx.global_mut::<Store>().claims.remove(&self.comparison.layout.id);
        }
        self.comparison.layout = LayoutState::default();
        self.comparison.items.clear();
        self.active = 0;
        let saved = cx.try_global::<Store>().and_then(|store| store.for_folder(&pending.root)).cloned();
        self.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(pending.root.clone()), cx));
        self.replace_tree(pending.root.clone(), cx);
        if let Some(mut saved) = saved {
            saved.folder = Some(pending.root);
            self.apply_saved_layout(saved, window, cx);
        } else {
            self.show_folder_workspace(window, cx);
        }
        self.explorer.sidebar_temporarily_visible = !cx.global::<Settings>().show_sidebar;
        if let Some(path) = pending.reveal { self.reveal_tree_folder(path, cx); }
        self.resume_folder_requests(pending.file_after_switch, window, cx);
        cx.notify();
    }

    fn resume_folder_requests(&mut self, file_after_switch: Option<OpenRequest>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(request) = file_after_switch { self.open_requests.pending.push_front(request); }
        if self.open_requests.pending.is_empty() { return; }
        // 关闭和恢复标签会推进内部代次，排在目录之后的显式打开意图仍应获得最终焦点。
        let generation = self.next_open_intent();
        if let Some(request) = self.open_requests.pending.back_mut() { request.generation = generation; }
        self.drain_open_requests(window, cx);
    }

    fn show_folder_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer.sidebar_temporarily_visible = !cx.global::<Settings>().show_sidebar;
        if !self.tabs.is_empty() {
            self.activate(self.active, window, cx);
        } else {
            self.home_visible = false;
            self.home.update(cx, |home, cx| home.set_active(false, cx));
            self.resume_tree(cx);
            window.focus(&self.focus, cx);
            if let Some(item) = self.comparison.items.first().copied() {
                self.load_saved_item(item, window, cx);
            } else {
                self.record_layout(cx);
            }
            cx.notify();
        }
    }
}

fn favorite_folder_unavailable(path: &Path, cx: &App) -> String {
    let path = super::super::explorer_actions::display_tree_path(path);
    ToolbarLocale::current(cx).app_format("home.favoriteFolderUnavailable", &[("path", &path)])
}
