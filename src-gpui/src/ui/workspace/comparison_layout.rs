//! 同一份原子布局清单可同时发布移交的两个窗口。正文和撤销历史始终由原会话管理。
use super::*;
use crate::ui::toolbar_i18n::{ToolbarLocale, UiMessage};
use crate::nomo_mark::protocol::DocumentSelection;
use std::collections::{BTreeMap, HashMap, HashSet};
#[path = "comparison_legacy.rs"]
mod legacy;
#[path = "folder_workspace.rs"]
mod folder;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 512;
const CLOSED_WINDOWS: usize = 16;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::ui::workspace) struct SavedDocument {
    pub(in crate::ui::workspace) path: Option<PathBuf>,
    pub(in crate::ui::workspace) session: String,
    pub(in crate::ui::workspace) mode: EditorMode,
    pub(in crate::ui::workspace) dirty: bool,
    #[serde(default)]
    pub(in crate::ui::workspace) revision: u64,
    pub(in crate::ui::workspace) byte: u64,
    pub(in crate::ui::workspace) fraction: f32,
    pub(in crate::ui::workspace) selection: DocumentSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::ui::workspace) legacy_line: Option<u64>,
}
impl SavedDocument {
    pub(super) fn display_title(&self, cx: &App) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| ToolbarLocale::current(cx).app_text("document.untitled").into())
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedLayout {
    id: u64,
    pub(super) items: Vec<TabItem>,
    pub(super) documents: BTreeMap<u64, SavedDocument>,
    pub(super) active: Option<u64>,
    pub(super) folder: Option<PathBuf>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct LayoutBook {
    windows: Vec<SavedLayout>,
    #[serde(default)]
    legacy_imported: bool,
}
struct Store {
    book: LayoutBook,
    path: PathBuf,
    claims: HashMap<u64, gpui::WeakEntity<Workspace>>,
    error: Option<UiMessage>,
    importing: bool,
}
impl gpui::Global for Store {}

pub(super) struct LayoutState {
    id: u64,
    pub(super) tracked: bool,
    flush: Option<Task<()>>,
    pub(super) pending: BTreeMap<u64, SavedDocument>,
    target: Option<(u64, u64, bool)>,
    after_load: Option<Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>>,
    anchor: Option<String>,
    pub(super) loading: Option<u64>,
    folder_open: Option<folder::FolderOpen>,
}
impl Default for LayoutState {
    fn default() -> Self {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self {
            id,
            // 普通标签与组合标签使用同一份恢复记录。
            tracked: true,
            flush: None,
            pending: Default::default(),
            target: None,
            after_load: None,
            anchor: None,
            loading: None,
            folder_open: None,
        }
    }
}
impl Store {
    fn available(&self) -> Option<&SavedLayout> {
        self.book.windows.iter().rev().find(|s| {
            (!s.items.is_empty() || s.folder.is_some())
                && !self
                    .claims
                    .get(&s.id)
                    .is_some_and(|owner| owner.upgrade().is_some())
        })
    }
    fn update(&mut self, updates: Vec<SavedLayout>) -> Result<(), UiMessage> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let mut next = self.book.clone();
        for layout in updates {
            if layout.documents.len() > MAX_DOCUMENTS {
                return Err(UiMessage::new("layout.documentLimit").arg("count", MAX_DOCUMENTS));
            }
            next.windows.retain(|s| s.id != layout.id);
            // 空目录记录也是有效状态，避免下次打开时回退到更旧的非空标签列表。
            if !layout.items.is_empty() || layout.folder.is_some() {
                next.windows.push(layout);
            }
        }
        self.claims.retain(|_, owner| owner.upgrade().is_some());
        while next
            .windows
            .iter()
            .filter(|s| !self.claims.contains_key(&s.id))
            .count()
            > CLOSED_WINDOWS
        {
            let pos = next
                .windows
                .iter()
                .position(|s| !self.claims.contains_key(&s.id))
                .unwrap();
            next.windows.remove(pos);
        }
        if next.windows == self.book.windows {
            return Ok(());
        }
        self.persist(next)
    }
    fn persist(&mut self, next: LayoutBook) -> Result<(), UiMessage> {
        let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err(UiMessage::new("layout.budget").arg("size", "2 MiB"));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::file_system::write_file_atomically(&self.path, &bytes)?;
        self.book = next;
        Ok(())
    }
}

impl Workspace {
    pub(in crate::ui::workspace) fn init_layout_store(&mut self, cx: &mut Context<Self>) {
        if cx.try_global::<Store>().is_some() {
            return;
        }
        let Some(path) = crate::settings::config_path()
            .and_then(|p| p.parent().map(|dir| dir.join("gpui-tab-layouts-v1.json")))
        else {
            return;
        };
        let data = (|| -> Result<LayoutBook, UiMessage> {
            use std::io::Read;
            let file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
                Err(e) => return Err(UiMessage::raw(e)),
            };
            let mut bytes = Vec::new();
            file.take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > MAX_BYTES {
                return Err(UiMessage::new("layout.tooLarge"));
            }
            serde_json::from_slice(&bytes).map_err(UiMessage::raw)
        })();
        let error = data
            .as_ref()
            .err()
            .map(|e| e.clone().context("layout.readFailed"));
        if let Some(error) = &error {
            self.failure_notice = Some(error.render(ToolbarLocale::current(cx)).into());
        }
        let book = data.unwrap_or_default();
        let importing = error.is_none() && !book.legacy_imported;
        cx.set_global(Store {
            book,
            claims: Default::default(),
            path: path.clone(),
            error,
            importing,
        });
        if importing {
            let root = path.parent().unwrap().to_path_buf();
            let task = cx.background_executor().spawn(async move { legacy::load(&root) });
            cx.spawn(async move |_, cx| {
                let result = task.await;
                let _ = cx.update(|cx| {
                    let store = cx.global_mut::<Store>();
                    store.importing = false;
                    let result = result.map_err(UiMessage::raw).and_then(|(mut layouts, warning)| {
                        if !layouts.is_empty() && layouts.len() + store.book.windows.len() > CLOSED_WINDOWS {
                            return Err(UiMessage::new("layout.windowLimit").arg("count", CLOSED_WINDOWS));
                        }
                        layouts.extend(store.book.windows.clone());
                        store.persist(LayoutBook {windows: layouts, legacy_imported: true})?;
                        Ok(warning)
                    });
                    for handle in cx.windows().into_iter().filter_map(|handle|handle.downcast::<Workspace>()) {
                        let _ = handle.update(cx, |workspace, window, cx| {
                            if let Err(error) = &result {workspace.failure_notice=Some(ToolbarLocale::current(cx).app_format("layout.legacyFailed", &[("error", &error.render(ToolbarLocale::current(cx)))]).into());}
                            if let Ok(Some(warning)) = &result {workspace.failure_notice=Some(ToolbarLocale::current(cx).app_format("layout.legacyPartial", &[("error", warning)]).into());}
                            workspace.continue_folder_open(window, cx);
                            workspace.drain_open_requests(window, cx);
                            cx.notify();
                        });
                    }
                });
            }).detach();
        }
    }
    pub(in crate::ui::workspace) fn has_saved_layout(&self, cx: &App) -> bool {
        cx.try_global::<Store>()
            .is_some_and(|store| store.available().is_some())
    }
    pub(super) fn save_member(&self, tab: &Tab, cx: &App) -> SavedDocument {
        let doc = tab.document.read(cx);
        let selection = doc.selection(cx);
        let (byte, fraction) = doc
            .workspace_reading_position(cx)
            .unwrap_or((selection.head_byte, 0.30));
        SavedDocument {
            path: doc.path(),
            session: doc.status().document_id.clone(),
            mode: tab.mode,
            dirty: doc.is_dirty(cx),
            revision: doc.status().revision,
            byte,
            fraction,
            selection,
            legacy_line: None,
        }
    }
    pub(super) fn layout_snapshot(&self, cx: &App) -> SavedLayout {
        let mut documents = self.comparison.layout.pending.clone();
        for tab in &self.tabs {
            documents.insert(tab.key, self.save_member(tab, cx));
        }
        SavedLayout {
            id: self.comparison.layout.id,
            items: self.comparison.items.clone(),
            documents,
            active: self.tabs.get(self.active).map(|tab| tab.key),
            folder: self.tree.as_ref().map(|tree| tree.root.clone()),
        }
    }
    pub(super) fn publish_layouts(updates: Vec<SavedLayout>, cx: &mut App) -> Result<(), String> {
        let locale = ToolbarLocale::current(cx);
        cx.try_global::<Store>().ok_or_else(|| locale.app_text("layout.directory").to_owned())?;
        cx.global_mut::<Store>().update(updates).map_err(|message| message.render(locale))
    }
    pub(super) fn claim_layout(&self, cx: &mut Context<Self>) {
        if cx.try_global::<Store>().is_some() {
            let weak = cx.entity().downgrade();
            cx.global_mut::<Store>()
                .claims
                .insert(self.comparison.layout.id, weak);
        }
    }
    pub(in crate::ui::workspace) fn record_layout(&mut self, cx: &mut Context<Self>) {
        if !self.comparison.layout.tracked
            || self.closing_window
            || self.folder_open_pending()
            || self.comparison.layout.flush.is_some()
        {
            return;
        }
        // 同一轮焦点/布局通知只安排一次短任务，正文输入和滚动不触碰磁盘。
        self.comparison.layout.flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(160))
                .await;
            let _ = this.update(cx, |workspace, cx| workspace.flush_layout(cx));
        }));
    }
    pub(in crate::ui::workspace) fn flush_layout(&mut self, cx: &mut Context<Self>) {
        self.comparison.layout.flush.take();
        if !self.comparison.layout.tracked
            || self.closing_window
            || self.folder_open_pending()
            || self.comparison.layout.loading.is_some()
        {
            return;
        }
        self.claim_layout(cx);
        if let Err(error) = Self::publish_layouts(vec![self.layout_snapshot(cx)], cx) {
            self.failure_notice = Some(error.into());
            cx.notify();
        }
    }
    pub(in crate::ui::workspace) fn note_closed_layout(
        &mut self,
        key: u64,
        doc: &Entity<DocumentHost>,
        discard: bool,
        cx: &mut Context<Self>,
    ) {
        if (!self.closing_window && !self.folder_open_pending()) || !self.comparison.layout.tracked {
            return;
        }
        let Some(store) = cx.try_global::<Store>() else {
            return;
        };
        let Some(mut saved) = store
            .book
            .windows
            .iter()
            .find(|s| s.id == self.comparison.layout.id)
            .cloned()
        else {
            return;
        };
        if let Some(member) = saved.documents.get_mut(&key) {
            member.path = doc.read(cx).path();
            member.dirty = false;
            if discard && member.path.is_none() {
                saved.documents.remove(&key);
                saved.normalize();
            }
        }
        if let Err(error) = Self::publish_layouts(vec![saved], cx) {
            self.failure_notice = Some(ToolbarLocale::current(cx).app_format("layout.saveFailed", &[("error", &error)]).into());
            if self.folder_open_pending() { self.cancel_close(cx); }
            cx.notify();
        }
    }
    pub(in crate::ui::workspace) fn restore_last_layout(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.is_empty() || !self.opening.is_empty() || self.tab_interaction_busy(window) {
            return;
        }
        if cx.try_global::<Store>().is_some_and(|store|store.importing) {
            self.notice=Some(ToolbarLocale::current(cx).app_text("layout.loading").into());cx.notify();return;
        }
        if let Err(error) = self.layout_recovery_candidate("", cx) {
            self.failure_notice = Some(error.into());
            cx.notify();
            return;
        }
        let Some(saved) = cx
            .try_global::<Store>()
            .and_then(|store| store.available())
            .cloned()
        else {
            return;
        };
        self.apply_saved_layout(saved, window, cx);
    }
    fn apply_saved_layout(&mut self, mut saved: SavedLayout, window: &mut Window, cx: &mut Context<Self>) {
        saved.normalize();
        let folder = saved.folder.as_ref().filter(|path| path.is_dir()).cloned();
        if saved.documents.is_empty() && folder.is_none() {
            if saved.folder.is_some() {
                self.failure_notice = Some(ToolbarLocale::current(cx).app_text("layout.folderUnavailable").into());
                cx.notify();
            }
            return;
        }
        // 文件中是上次进程的逻辑 ID。本次分配新 ID，多个窗口可独立恢复、组合或交换。
        let keys: HashMap<_, _> = saved
            .documents
            .keys()
            .map(|key| (*key, next_tab_key()))
            .collect();
        let mut items = saved.items;
        for item in &mut items {
            *item = match *item {
                TabItem::Single(key) => TabItem::Single(keys[&key]),
                TabItem::Pair(p) => TabItem::Pair(Pair {
                    left: keys[&p.left],
                    right: keys[&p.right],
                    focused: keys[&p.focused],
                    ratio: p.ratio,
                }),
            };
        }
        self.home_visible = false;
        self.home.update(cx, |home, cx| home.set_active(false, cx));
        window.focus(&self.focus, cx);
        self.comparison.items = items;
        // 文件夹工作区也可再次恢复其他布局，原布局此时不再归当前窗口占用。
        cx.global_mut::<Store>().claims.remove(&self.comparison.layout.id);
        self.comparison.layout.id = saved.id;
        self.comparison.layout.tracked = true;
        self.comparison.layout.pending = saved
            .documents
            .into_iter()
            .map(|(key, doc)| (keys[&key], doc))
            .collect();
        self.claim_layout(cx);
        if let Some(folder) = folder {
            if self.tree.as_ref().is_none_or(|tree| tree.root != folder) {
                self.replace_tree(folder, cx);
            } else {
                self.resume_tree(cx);
            }
        } else if self.tree.is_some() {
            self.suspend_tree(cx);
            self.tree = None;
            self.tree_animation = None;
            self.clear_tree_search(cx);
            self.explorer.search_focused = None;
        }
        let item = saved
            .active
            .and_then(|key| keys.get(&key))
            .and_then(|key| self.comparison.item(*key))
            .or_else(|| self.comparison.items.first().copied());
        if let Some(item) = item {
            self.load_saved_item(item, window, cx);
        } else {
            self.record_layout(cx);
            cx.notify();
        }
    }
    pub(super) fn load_to_close(
        &mut self,
        item: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.request_saved_item(item, true, window, cx)
    }
    pub(in crate::ui::workspace) fn load_saved_item(
        &mut self,
        item: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.request_saved_item(item, false, window, cx)
    }
    fn request_saved_item(
        &mut self,
        item: TabItem,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !item
            .keys()
            .iter()
            .any(|key| self.comparison.layout.pending.contains_key(key))
        {
            return false;
        }
        self.comparison.layout.after_load = None;
        self.comparison.layout.anchor = None;
        let generation = self.next_open_intent();
        self.comparison.layout.target = Some((item.focused(), generation, close));
        self.advance_saved_open(window, cx);
        true
    }
    pub(in crate::ui::workspace) fn remap_saved_paths(
        &mut self,
        mut update: impl FnMut(&mut PathBuf),
    ) {
        for doc in self.comparison.layout.pending.values_mut() {
            if let Some(path) = &mut doc.path {
                update(path);
            }
        }
    }
    pub(in crate::ui::workspace) fn unloaded_tabs(&self) -> impl Iterator<Item = (u64, &SavedDocument)> {
        self.comparison.layout.pending.iter().map(|(&key, saved)| (key, saved))
    }
    pub(in crate::ui::workspace) fn remove_deleted_layout_members(
        &mut self,
        removed: impl Fn(&Path) -> bool,
    ) -> bool {
        let keys: Vec<_> = self.comparison.layout.pending.iter().filter_map(|(&key, saved)| {
            (!saved.dirty && saved.path.as_deref().is_some_and(&removed)).then_some(key)
        }).collect();
        for key in &keys {
            self.comparison.layout.pending.remove(key);
            self.comparison.remove_member(*key);
        }
        let layout = &mut self.comparison.layout;
        if layout.target.is_some_and(|(key, _, _)| keys.contains(&key)) {
            layout.target = None;
            layout.after_load = None;
            layout.anchor = None;
        }
        !keys.is_empty()
    }
    pub(in crate::ui::workspace) fn load_before(
        &mut self,
        item: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
        resume: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> bool {
        if !item
            .keys()
            .iter()
            .any(|k| self.comparison.layout.pending.contains_key(k))
        {
            return false;
        }
        let generation = self.next_open_intent();
        self.comparison.layout.target = Some((item.focused(), generation, false));
        self.comparison.layout.anchor = None;
        self.comparison.layout.after_load = Some(Box::new(resume));
        self.advance_saved_open(window, cx);
        true
    }
    pub(in crate::ui::workspace) fn saved_path_key(&self, path: &Path) -> Option<u64> {
        self.comparison
            .layout
            .pending
            .iter()
            .find(|(_, doc)| doc.path.as_deref() == Some(path))
            .map(|(key, _)| *key)
    }
    pub(in crate::ui::workspace) fn open_saved_member(
        &mut self,
        key: u64,
        anchor: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_open_intent();
        self.comparison.layout.after_load = None;
        self.comparison.layout.target = Some((key, generation, false));
        self.comparison.layout.anchor = anchor;
        self.advance_saved_open(window, cx);
    }
    pub(in crate::ui::workspace) fn mark_saved_loading(
        &mut self,
        source: &application_dialogs::OpenSource,
    ) {
        if let application_dialogs::OpenSource::Layout(key, _) = source {
            self.comparison.layout.loading = Some(*key);
        }
    }
    pub(in crate::ui::workspace) fn cancel_saved_request(
        &mut self,
        source: &application_dialogs::OpenSource,
    ) {
        if matches!(source, application_dialogs::OpenSource::Layout(..)) {
            self.comparison.layout.target = None;
            self.comparison.layout.loading = None;
        }
    }
    pub(in crate::ui::workspace) fn advance_saved_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comparison.layout.loading.is_some() || !self.opening.is_empty() {
            return;
        }
        let Some((key, generation, close)) = self.comparison.layout.target else {
            return;
        };
        if generation != self.open_requests.generation {
            self.comparison.layout.target = None;
            self.comparison.layout.after_load = None;
            return;
        }
        let Some(item) = self.comparison.item(key) else {
            self.comparison.layout.target = None;
            return;
        };
        if let Some((next, saved)) = item.keys().into_iter().find_map(|k| {
            self.comparison
                .layout
                .pending
                .get(&k)
                .cloned()
                .map(|doc| (k, doc))
        }) {
            let candidate = match self.layout_recovery_candidate(&saved.session, cx) {
                Ok(candidate) => candidate,
                Err(e) => {
                    self.failure_notice = Some(e.into());
                    self.comparison.layout.target = None;
                    if self.folder_open_pending() { self.cancel_close(cx); }
                    cx.notify();
                    return;
                }
            };
            if candidate.is_none()
                && (saved.path.as_ref().is_some_and(|p| !p.is_file())
                    || saved.dirty)
            {
                self.failure_notice =
                    Some(ToolbarLocale::current(cx).app_format("layout.restoreFailed", &[("name", &saved.display_title(cx))]).into());
                self.finish_saved_open(next, false, window, cx);
                return;
            }
            self.open_document_from(
                saved.path,
                false,
                None,
                application_dialogs::OpenSource::Layout(next, candidate),
                generation,
                window,
                cx,
            );
        } else {
            self.comparison.layout.target = None;
            if let Some(resume) = self.comparison.layout.after_load.take() {
                resume(self, window, cx);
            } else if close {
                self.close_item(item, window, cx);
            } else if let Some(index) = self.tab_index(key) {
                let anchor = self.comparison.layout.anchor.take();
                self.activate_after_open(
                    index,
                    generation,
                    AfterOpen {
                        anchor,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }
        }
    }
    pub(in crate::ui::workspace) fn saved_open_document(
        &self,
        source: &application_dialogs::OpenSource,
    ) -> Option<SavedDocument> {
        if let application_dialogs::OpenSource::Layout(key, _) = source {
            self.comparison.layout.pending.get(key).cloned()
        } else {
            None
        }
    }
    pub(in crate::ui::workspace) fn finish_saved_open(
        &mut self,
        key: u64,
        accepted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comparison.layout.loading = None;
        if !accepted
            && self
                .comparison
                .layout
                .pending
                .get(&key)
                .is_some_and(|saved| {
                    saved.path.as_ref().is_some_and(|p| p.is_file())
                        || self
                            .layout_recovery_candidate(&saved.session, cx)
                            .ok()
                            .flatten()
                            .is_some()
                })
        {
            self.comparison.layout.target = None;
            self.comparison.layout.after_load = None;
            self.comparison.batch.clear();
            if self.folder_open_pending() { self.cancel_close(cx); }
            cx.notify();
            return;
        }
        self.comparison.layout.pending.remove(&key);
        if !accepted {
            self.comparison.layout.after_load = None;
            let fallback = self
                .comparison
                .item(key)
                .and_then(|item| item.keys().into_iter().find(|k| *k != key));
            self.comparison.remove_member(key);
            if let Some((wanted, generation, close)) = self.comparison.layout.target {
                if wanted == key {
                    self.comparison.layout.target = fallback
                        .or_else(|| self.comparison.items.first().map(|item| item.focused()))
                        .map(|k| (k, generation, close && fallback.is_some()));
                }
            }
        }
        self.advance_saved_open(window, cx);
        if self.comparison.items.is_empty() {
            self.record_layout(cx);
        }
        cx.notify();
    }
}
impl SavedLayout {
    fn normalize(&mut self) {
        let mut paths = HashSet::new();
        let mut sessions = HashSet::new();
        self.documents.retain(|_, doc| {
            !doc.session.is_empty()
                && sessions.insert(doc.session.clone())
                && doc.path.as_ref().is_none_or(|p| paths.insert(p.clone()))
        });
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for item in self.items.drain(..) {
            let valid: Vec<_> = item
                .keys()
                .into_iter()
                .filter(|key| self.documents.contains_key(key) && seen.insert(*key))
                .collect();
            match (item, valid.as_slice()) {
                (TabItem::Pair(mut p), [_, _])
                    if cfg!(windows) && [p.left, p.right].iter().all(|key| {
                        self.documents[key]
                            .path
                            .as_ref()
                            .is_none_or(|path| super::markdown_path(path))
                    }) =>
                {
                    if p.focused != p.right {
                        p.focused = p.left;
                    }
                    let mode = if self.documents[&p.focused].mode == EditorMode::Source {
                        EditorMode::Source
                    } else {
                        EditorMode::Semantic
                    };
                    for key in [p.left, p.right] {
                        self.documents.get_mut(&key).unwrap().mode = mode;
                    }
                    p.ratio = if p.ratio.is_finite() {
                        p.ratio.clamp(0.2, 0.8)
                    } else {
                        0.5
                    };
                    out.push(TabItem::Pair(p));
                }
                (_, keys) => out.extend(keys.iter().copied().map(TabItem::Single)),
            }
        }
        out.extend(
            self.documents
                .keys()
                .filter(|key| seen.insert(**key))
                .copied()
                .map(TabItem::Single),
        );
        self.items = out;
    }
}
