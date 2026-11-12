//! 打开意图只持有有界路径与目标身份；批量请求串行收尾，正文仍由原会话加载。
use super::*;
use std::collections::VecDeque;

const MAX_PENDING_OPENS: usize = 64;

#[derive(Clone, Default)]
pub(super) struct OpenRequest {
    pub path: Option<PathBuf>,
    pub preview: bool,
    pub anchor: Option<String>,
    pub choose_window: bool,
    pub newly_created: bool,
    pub document_only: bool,
    pub generation: u64,
}

#[derive(Clone, Default)]
pub(super) struct AfterOpen {
    pub anchor: Option<String>,
    pub selection: Option<crate::nomo_mark::protocol::DocumentSelection>,
    pub save_as: bool,
    pub record_recent: bool,
}

#[derive(Default)]
pub(super) struct OpenRequests {
    pub generation: u64,
    pub pending: VecDeque<OpenRequest>,
}

impl Workspace {
    pub(super) fn next_open_intent(&mut self) -> u64 {
        self.open_requests.generation = self.open_requests.generation.wrapping_add(1);
        self.open_requests.generation
    }

    pub fn open_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let requests = paths.into_iter().map(|path| OpenRequest {
            path: Some(path),
            choose_window: true,
            ..Default::default()
        });
        self.open_batch(requests, window, cx);
    }

    pub(crate) fn open_dropped_documents(
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(workspace) = window.root::<Self>().flatten() else { return false; };
        let documents: Vec<_> = paths.paths().iter()
            .filter(|path| path.is_absolute() && crate::ui::file_tree::supported_document(path))
            .take(MAX_PENDING_OPENS + 1)
            .cloned()
            .collect();
        if documents.is_empty() { return false; }

        // 编辑器也处理外部落点，打开前先释放其事件回调的可变借用。
        window.defer(cx, move |window, cx| {
            workspace.update(cx, |workspace, cx| {
                let requests = documents.into_iter().map(|path| OpenRequest {
                    path: Some(path),
                    document_only: true,
                    ..Default::default()
                });
                workspace.open_batch(requests, window, cx);
            });
        });
        true
    }

    pub(super) fn open_batch(
        &mut self,
        requests: impl IntoIterator<Item = OpenRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_tree_click.take();
        self.deferred_open = None;
        for request in requests {
            self.enqueue_open_request(request, cx);
        }
        self.drain_open_requests(window, cx);
    }

    pub(super) fn enqueue_open(&mut self, path: Option<PathBuf>, preview: bool, anchor: Option<String>, choose_window: bool, cx: &mut Context<Self>) {
        self.enqueue_open_request(OpenRequest { path, preview, anchor, choose_window, ..Default::default() }, cx);
    }

    pub(super) fn enqueue_open_request(&mut self, mut request: OpenRequest, cx: &mut Context<Self>) {
        if self.open_requests.pending.len() >= MAX_PENDING_OPENS
            || request.path.as_ref().is_some_and(|path| path.as_os_str().len() > 32 * 1024)
            || request.anchor.as_ref().is_some_and(|anchor| anchor.len() > 8192)
        {
            self.failure_notice = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).app_text("open.queueFull").into());
            cx.notify();
            return;
        }
        if self.waiting_library_close { self.waiting_library_close = false; self.cancel_close(cx); }
        self.cancel_preview_opens(request.path.as_deref());
        // 连续单击预览可合并；显式多选、新建和固定打开都必须保留。
        self.open_requests.pending.retain(|request| !request.preview);
        request.generation = self.next_open_intent();
        self.open_requests.pending.push_back(request);
    }

    pub(super) fn drain_open_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.opening.is_empty() || self.pending_document_transition.is_some()
            || self.application_dialogs.busy || window.has_active_prompt() || self.closing_window
            || self.folder_open_pending()
        { return; }
        while !self.open_requests.pending.is_empty() {
            if let Some(document)=self.document() {
                if !self.prepare_document_transition(&document,false,window,cx,|workspace,window,cx|workspace.drain_open_requests(window,cx)) {
                    if self.pending_document_transition.is_none() {self.open_requests.pending.pop_front();}
                    return;
                }
            }
            let request=self.open_requests.pending.pop_front().expect("pending open");
            self.route_open_request(request, window, cx);
            if !self.opening.is_empty() || self.pending_document_transition.is_some()
                || self.application_dialogs.busy || window.has_active_prompt() || self.folder_open_pending() { break; }
        }
    }

    pub(super) fn open_request_here(&mut self, request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) {
        if request.path.is_some() {
            self.open_path_now(request, window, cx);
        } else {
            self.open_document_from(None, false, None, application_dialogs::OpenSource::File, request.generation, window, cx);
        }
    }

    /// 跨窗口复用同时覆盖已挂载标签和正在加载的文件，防止第二个 owner 竞争同一会话。
    pub(super) fn reuse_open_document(&mut self, request: &OpenRequest, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(path) = &request.path else { return false; };
        let foreground=request.generation==self.open_requests.generation;
        let current = window.window_handle();
        for handle in cx.windows().into_iter().filter_map(|handle| handle.downcast::<Workspace>()) {
            if gpui::AnyWindowHandle::from(handle) == current { continue; }
            let request = request.clone();
            let reused = handle.update(cx, |target, target_window, cx| {
                let index = target.tabs.iter().position(|tab| tab.document.read(cx).path().as_ref() == Some(path));
                let loading = target.opening.iter().any(|opening| opening.path.as_ref() == Some(path));
                let saved=target.saved_path_key(path);
                if index.is_none() && !loading && saved.is_none() { return false; }
                if !foreground {return true;}
                let generation = target.next_open_intent();
                if let Some(key)=saved {target.open_saved_member(key,request.anchor.clone(),target_window,cx);} else if let Some(index) = index {
                    if !request.preview { target.tabs[index].preview = false; }
                    target.activate_after_open(index, generation, AfterOpen { anchor: request.anchor, record_recent: true, ..Default::default() }, target_window, cx);
                } else if let Some(opening) = target.opening.iter_mut().find(|opening| opening.path.as_ref() == Some(path) && opening.request.check().is_ok()) {
                    opening.generation = generation;
                    opening.preview &= request.preview;
                    if request.anchor.is_some() { opening.anchor = request.anchor; }
                } else {
                    target.enqueue_open_request(OpenRequest { choose_window: false, ..request }, cx);
                }
                target.reveal_window(target_window, cx);
                true
            }).unwrap_or(false);
            if reused { return true; }
        }
        false
    }
}
