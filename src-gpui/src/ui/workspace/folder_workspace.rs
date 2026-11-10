//! 目录切换沿用标签关闭确认；历史只保存引用，离开后释放全部已挂载文档。
use super::*;

pub(super) struct FolderOpen {
    root: PathBuf,
    closing: bool,
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
        &mut self, request: &OpenRequest, window: &mut Window, cx: &mut Context<Self>,
    ) -> bool {
        let Some(root) = &request.path else { return false; };
        let current = window.window_handle();
        for handle in cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
            if gpui::AnyWindowHandle::from(handle) == current { continue; }
            let reused = handle.update(cx, |target, target_window, cx| {
                if target.folder_open_pending() || target.closing_window
                    || !target.tree.as_ref().is_some_and(|tree| matches_folder(&tree.root, root)) {
                    return false;
                }
                if request.generation == self.open_requests.generation {
                    if !target.tab_interaction_busy(target_window) { target.show_folder_workspace(target_window, cx); }
                    target.reveal_window(target_window, cx);
                }
                true
            }).unwrap_or(false);
            if reused { return true; }
        }
        false
    }

    pub(in crate::ui::workspace) fn open_folder_workspace(
        &mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.tab_interaction_busy(window) { return; }
        if self.tree.as_ref().is_some_and(|tree| matches_folder(&tree.root, &root)) {
            self.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(root), cx));
            self.show_folder_workspace(window, cx);
            self.resume_folder_requests(window, cx);
            return;
        }
        self.comparison.layout.folder_open = Some(FolderOpen { root, closing: false });
        self.start_recovery_catalog(cx);
        self.continue_folder_open(window, cx);
    }

    pub(in crate::ui::workspace) fn continue_folder_open(
        &mut self, window: &mut Window, cx: &mut Context<Self>,
    ) {
        let Some(pending) = &self.comparison.layout.folder_open else { return; };
        if pending.closing || !self.opening.is_empty() || self.pending_document_transition.is_some()
            || window.has_active_prompt() { return; }
        // 两个现有后台任务完成时直接继续，不增加轮询或常驻任务。
        if cx.try_global::<Store>().is_some_and(|store| store.importing) || self.recovery_catalog_loading(cx) {
            return;
        }
        let root = pending.root.clone();
        let error = if !root.is_dir() {
            Some(ToolbarLocale::current(cx).app_text("layout.folderUnavailable").to_owned())
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

        // 先落盘完整原布局，再关闭会话；关闭期间不发布逐渐减少的标签列表。
        self.comparison.layout.flush.take();
        self.claim_layout(cx);
        if let Err(error) = Self::publish_layouts(vec![self.layout_snapshot(cx)], cx) {
            self.failure_notice = Some(error.into());
            self.cancel_close(cx);
            cx.notify();
            return;
        }
        self.comparison.layout.folder_open.as_mut().unwrap().closing = true;
        // 未加载的后台标签只保留原记录，不为了切换目录加载其正文。
        self.comparison.batch = self.comparison.items.iter()
            .filter(|item| self.tabs.iter().any(|tab| item.contains(tab.key)))
            .map(|item| item.first()).collect();
        self.next_batch_close(window, cx);
    }

    pub(in crate::ui::workspace) fn finish_folder_open(
        &mut self, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if !self.tabs.is_empty() || !self.opening.is_empty()
            || !self.comparison.layout.folder_open.as_ref().is_some_and(|pending| pending.closing) {
            return;
        }
        let pending = self.comparison.layout.folder_open.take().unwrap();
        if cx.try_global::<Store>().is_some() {
            cx.global_mut::<Store>().claims.remove(&self.comparison.layout.id);
        }
        self.comparison.layout = LayoutState::default();
        self.comparison.items.clear();
        self.active = 0;
        let saved = cx.try_global::<Store>().and_then(|store| store.for_folder(&pending.root)).cloned();
        self.library.update(cx, |library, cx| library.change(LibraryChange::AddFolder(pending.root.clone()), cx));
        // 目标目录在确认保存期间消失时也不能把空窗口重新记到原目录名下。
        self.replace_tree(pending.root.clone(), cx);
        if let Some(mut saved) = saved {
            saved.folder = Some(pending.root);
            self.apply_saved_layout(saved, window, cx);
        } else {
            self.show_folder_workspace(window, cx);
        }
        self.explorer.sidebar_temporarily_visible = !cx.global::<Settings>().show_sidebar;
        self.resume_folder_requests(window, cx);
        cx.notify();
    }

    fn resume_folder_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
