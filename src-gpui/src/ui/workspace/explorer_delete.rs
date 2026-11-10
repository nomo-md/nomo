//! 一次确认覆盖所有窗口。会话层按实际删除结果提交关闭，视图随后统一更新。
use super::*;
use super::explorer_actions::{checked_existing, display_tree_path};
use crate::text_document::{
    TextDocumentError,
    markdown_session::{DeleteDocument, DeleteReport, SessionMember},
};
use crate::ui::{dialogs::DeleteChoice, toolbar_i18n::UiMessage};
use gpui::{AsyncApp, WeakEntity, WindowHandle};
use std::collections::{HashMap, HashSet};

struct DeleteRequest {
    origin: WindowHandle<Workspace>,
    generation: u64,
    root: PathBuf,
    path: PathBuf,
}

struct AffectedTab {
    window: WindowHandle<Workspace>,
    document: WeakEntity<DocumentHost>,
    member: SessionMember,
    path: PathBuf,
    dirty: bool,
}

#[derive(Default)]
struct DeleteScope {
    tabs: Vec<AffectedTab>,
    observed_paths: Vec<PathBuf>,
}

impl Workspace {
    pub(super) fn confirm_tree_delete(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_prompt() { return; }
        let Some(root) = self.tree.as_ref().map(|tree| tree.root.clone()) else { return; };
        let Some(manager) = self.services.clone() else {
            self.notice = Some(ToolbarLocale::current(cx).app_text("tree.checkUnavailable").into());
            cx.notify();
            return;
        };
        let Some(origin) = window.window_handle().downcast::<Workspace>() else { return; };
        let request = DeleteRequest { origin, generation: self.explorer.generation(), root, path };
        let busy_paths = self.tree_busy_paths(&request.path);
        self.explorer.mutating.extend(busy_paths.iter().cloned());
        cx.spawn(async move |_, cx| {
            let result = request.run(manager, cx).await;
            let _ = origin.update(cx, |workspace, _, cx| {
                for path in busy_paths { workspace.explorer.mutating.remove(&path); }
                if let Err(error) = result {
                    workspace.notice = Some(error.render(ToolbarLocale::current(cx)).into());
                }
                cx.notify();
            });
        }).detach();
    }

    fn prepare_delete_scope(
        &mut self,
        target: &Path,
        restored: &mut HashSet<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, UiMessage> {
        let matches = |path: &Path| path.starts_with(target);
        if self.opening.iter().any(|opening| opening.path.as_deref().is_some_and(matches)) {
            return Ok(false);
        }
        // 只有含草稿的延迟恢复标签需要加载，普通标签直接在删除成功后移除。
        let mut draft_to_load = None;
        for (key, saved) in self.unloaded_tabs() {
            if !saved.path.as_deref().is_some_and(matches) { continue; }
            let draft = self.layout_recovery_candidate(&saved.session, cx).map_err(UiMessage::raw)?;
            if draft.is_none() {
                if saved.dirty { return Err(UiMessage::new("tree.deleteDraftUnavailable")); }
                continue;
            }
            draft_to_load = Some(key);
            break;
        }
        if let Some(key) = draft_to_load {
            if window.has_active_prompt() || !self.opening.is_empty() || self.pending_document_transition.is_some() {
                return Ok(false);
            }
            let item = self.comparison.item(key).ok_or_else(delete_changed)?;
            if !restored.insert(key) || !self.load_before(item, window, cx, |_, _, _| {}) {
                return Err(UiMessage::new("tree.deleteDraftUnavailable"));
            }
            return Ok(false);
        }
        let documents: Vec<_> = self.tabs.iter()
            .filter(|tab| tab.document.read(cx).path().as_deref().is_some_and(matches))
            .map(|tab| tab.document.clone()).collect();
        if documents.is_empty() { return Ok(true); }
        if self.close_prompt || self.closing_window || self.comparison.closing.is_some()
            || self.application_dialogs.busy || window.has_active_prompt()
            || self.pending_saves.iter().any(|save| documents.contains(&save.document))
        {
            return Ok(false);
        }
        for document in documents {
            let host = document.read(cx);
            if host.has_failed_input(cx) {
                return Err(UiMessage::new("tree.deleteInputPending"));
            }
            if host.ready(cx).is_ok() && !host.has_composition(cx) { continue; }
            if self.pending_document_transition.is_some() { return Ok(false); }
            if !self.prepare_document_transition(&document, false, window, cx, |_, _, _| {}) {
                return if self.pending_document_transition.is_some() { Ok(false) }
                    else { Err(UiMessage::new("tree.deleteInputPending")) };
            }
        }
        Ok(true)
    }

    fn apply_deleted_tabs(
        &mut self,
        target: &Path,
        report: &DeleteReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let whole = report.failure.is_none();
        let removed = |path: &Path| (whole && path.starts_with(target)) || report.missing.contains(path);
        for opening in &self.opening {
            if opening.path.as_deref().is_some_and(removed) { opening.request.cancel(); }
        }
        self.open_requests.pending.retain(|request| !request.path.as_deref().is_some_and(removed));
        if self.deferred_open.as_ref().and_then(|request| request.path.as_deref()).is_some_and(removed) {
            self.deferred_open = None;
        }

        let previous = self.tabs.get(self.active).map(|tab| tab.key);
        let unloaded = self.remove_deleted_layout_members(removed);
        let documents: Vec<_> = self.tabs.iter()
            .filter(|tab| report.closed.contains(&tab.document.read(cx).status().document_id))
            .map(|tab| (tab.key, tab.document.clone())).collect();
        let mut position = None;
        for (key, document) in documents {
            if let Err(error) = document.update(cx, |host, cx| host.release_closed_view(true, cx)) {
                self.failure_notice = Some(error.to_string().into());
            }
            let removed = self.remove_closed_tab(&document, true, cx);
            position = if previous == Some(key) {
                removed.or(position)
            } else {
                position.or(removed)
            };
        }

        if self.tree.as_ref().is_some_and(|tree| removed(&tree.root)) {
            self.suspend_tree(cx);
            self.tree = None;
            self.tree_animation = None;
            self.clear_tree_search(cx);
            self.sync_tree_selection(false, cx);
            self.record_layout(cx);
        } else if self.tree.as_ref().is_some_and(|tree| target.starts_with(&tree.root) || tree.root.starts_with(target)) {
            self.refresh_tree(cx);
        }
        if let Some(position) = position {
            self.finish_tab_removal(previous, position, window, cx);
        } else if unloaded {
            self.record_layout(cx);
            cx.notify();
        }
    }
}

impl DeleteRequest {
    async fn run(&self, manager: MarkdownDocumentSessionManager, cx: &mut AsyncApp) -> Result<(), UiMessage> {
        let (root, path) = (self.root.clone(), self.path.clone());
        let (target, directory) = cx.background_executor()
            .spawn(async move { checked_delete_target(&root, &path) }).await?;
        let reviewed = self.prepare(&target, cx).await?;
        let dirty = reviewed.tabs.iter().filter(|tab| tab.dirty);
        let names = dirty.clone().map(|tab| {
            if tab.path == target { tab.path.file_name().unwrap_or(tab.path.as_os_str()).to_string_lossy().into_owned() }
            else { display_tree_path(tab.path.strip_prefix(&target).unwrap_or(&tab.path)) }
        }).collect();
        let response = self.origin.update(cx, |workspace, window, cx| {
            self.check_current(workspace)?;
            Ok::<_, UiMessage>(dialogs::confirm_delete(Path::new(&display_tree_path(&target)), names, window, cx))
        }).map_err(UiMessage::raw)??;
        let mut copies = HashMap::new();
        match response.await {
            DeleteChoice::Cancel => return Ok(()),
            DeleteChoice::Delete => {}
            DeleteChoice::SaveCopies => {
                for tab in dirty {
                    let Some(path) = self.choose_copy_path(&target, &tab.path, cx).await? else { return Ok(()); };
                    copies.insert(tab.member.document_id.clone(), path);
                }
            }
        }

        let scope = self.prepare(&target, cx).await?;
        let current_ids: HashSet<_> = scope.tabs.iter().map(|tab| tab.member.document_id.as_str()).collect();
        if copies.keys().any(|id| !current_ids.contains(id.as_str())) {
            return Err(delete_changed());
        }
        let approved: HashMap<_, _> = reviewed.tabs.iter().filter(|tab| tab.dirty)
            .map(|tab| (tab.member.document_id.as_str(), tab)).collect();
        for tab in &scope.tabs {
            let needs_consent = tab.dirty || copies.contains_key(&tab.member.document_id);
            if needs_consent && !approved.get(tab.member.document_id.as_str()).is_some_and(|old| {
                old.member.owner == tab.member.owner && old.member.revision == tab.member.revision && old.path == tab.path
            }) {
                return Err(delete_changed());
            }
        }

        let mut frozen = Vec::new();
        let prepared = cx.update(|app| -> Result<(), UiMessage> {
            self.check_current(self.origin.read(app).map_err(UiMessage::raw)?)?;
            for tab in &scope.tabs {
                let resume_autosave = tab.window.update(app, |workspace, window, cx| {
                    let document = tab.document.upgrade().ok_or_else(delete_changed)?;
                    if !workspace.tabs.iter().any(|tab| tab.document == document) {
                        return Err(delete_changed());
                    }
                    let host = document.read(cx);
                    if host.ready(cx).is_err() || host.has_composition(cx)
                        || Some(host.status().revision) != tab.member.revision
                        || host.path().as_ref() != Some(&tab.path)
                        || (!tab.dirty && host.is_dirty(cx))
                    {
                        return Err(delete_changed());
                    }
                    if workspace.document().as_ref() == Some(&document) && !workspace.leave_mini(window, cx) {
                        return Err(UiMessage::new("tree.deleteInputPending"));
                    }
                    workspace.cancel_export_for(&document);
                    let resume_autosave = workspace.tabs.iter_mut().find(|tab| tab.document == document)
                        .and_then(|tab| tab.autosave.take()).is_some();
                    document.update(cx, |host, cx| host.set_deletion_pending(true, cx));
                    Ok(resume_autosave)
                }).map_err(UiMessage::raw)??;
                frozen.push((tab.window, tab.document.clone(), resume_autosave));
            }
            Ok(())
        });

        let result = match prepared {
            Ok(()) => {
                let locale = cx.update(|app| ToolbarLocale::current(app));
                let (root, path, target) = (self.root.clone(), self.path.clone(), target.clone());
                let plans = scope.tabs.into_iter().map(|tab| DeleteDocument {
                    copy_to: copies.remove(&tab.member.document_id),
                    path: tab.path, member: tab.member, discard: tab.dirty,
                }).collect();
                cx.background_executor().spawn(async move {
                    manager.delete_path(&target, directory, plans, scope.observed_paths, || {
                        let (checked, is_directory) = checked_delete_target(&root, &path).map_err(|error| TextDocumentError::new(
                            "delete-target", error.render(locale)
                        ))?;
                        if checked != target || is_directory != directory {
                            return Err(TextDocumentError::new("delete-changed", "删除目标已变化"));
                        }
                        Ok(())
                    }).map_err(delete_error)
                }).await
            }
            Err(error) => Err(error),
        };

        cx.update(|app| {
            if let Ok(report) = &result {
                for handle in workspaces(app) {
                    let _ = handle.update(app, |workspace, window, cx| {
                        workspace.apply_deleted_tabs(&target, report, window, cx);
                    });
                }
            }
            for (handle, document, resume_autosave) in frozen {
                let _ = handle.update(app, |workspace, window, cx| {
                    let Some(document) = document.upgrade().filter(|doc| workspace.tabs.iter().any(|tab| tab.document == *doc))
                        else { return; };
                    document.update(cx, |host, cx| host.set_deletion_pending(false, cx));
                    if resume_autosave && document.read(cx).is_dirty(cx) {
                        workspace.schedule_autosave(document, window, cx);
                    }
                });
            }
        });
        if let Some(error) = result?.failure {
            return Err(UiMessage::new("tree.deleteFailed").arg("error", error));
        }
        Ok(())
    }

    async fn prepare(&self, target: &Path, cx: &mut AsyncApp) -> Result<DeleteScope, UiMessage> {
        let started = Instant::now();
        let mut restored = HashSet::new();
        loop {
            let scope = cx.update(|app| -> Result<Option<DeleteScope>, UiMessage> {
                self.check_current(self.origin.read(app).map_err(UiMessage::raw)?)?;
                let handles = workspaces(app);
                for handle in &handles {
                    if !handle.update(app, |workspace, window, cx| {
                        workspace.prepare_delete_scope(target, &mut restored, window, cx)
                    }).map_err(UiMessage::raw)?? {
                        return Ok(None);
                    }
                }
                let mut scope = DeleteScope::default();
                for handle in handles {
                    let workspace = handle.read(app).map_err(UiMessage::raw)?;
                    scope.tabs.extend(workspace.tabs.iter().filter_map(|tab| {
                        let host = tab.document.read(app);
                        let path = host.path().filter(|path| path.starts_with(target))?;
                        let port = host.port();
                        Some(AffectedTab {
                            window: handle, document: tab.document.downgrade(), path,
                            member: SessionMember {
                                document_id: port.document_id, owner: port.owner, revision: Some(host.status().revision),
                            },
                            dirty: host.is_dirty(app),
                        })
                    }));
                    scope.observed_paths.extend(workspace.unloaded_tabs().filter_map(|(_, saved)| {
                        saved.path.clone().filter(|path| path.starts_with(target))
                    }));
                    scope.observed_paths.extend(workspace.tree.as_ref().filter(|tree| tree.root.starts_with(target)).map(|tree| tree.root.clone()));
                }
                scope.tabs.sort_by(|a, b| a.path.cmp(&b.path));
                Ok(Some(scope))
            })?;
            if let Some(scope) = scope { return Ok(scope); }
            if started.elapsed() >= Duration::from_secs(30) {
                return Err(UiMessage::new("tree.deleteInputPending"));
            }
            cx.background_executor().timer(Duration::from_millis(32)).await;
        }
    }

    async fn choose_copy_path(&self, target: &Path, source: &Path, cx: &mut AsyncApp) -> Result<Option<PathBuf>, UiMessage> {
        let directory = target.parent().ok_or_else(delete_changed)?;
        let stem = source.file_stem().unwrap_or(source.as_os_str()).to_string_lossy();
        let filename = match source.extension() {
            Some(ext) => format!("{stem}-copy.{}", ext.to_string_lossy()),
            None => format!("{stem}-copy"),
        };
        let response = self.origin.update(cx, |workspace, _, cx| {
            self.check_current(workspace)?;
            Ok::<_, UiMessage>(cx.prompt_for_new_path(directory, Some(&filename)))
        }).map_err(UiMessage::raw)??;
        response.await.map_err(UiMessage::raw)?.map_err(UiMessage::raw)
    }

    fn check_current(&self, workspace: &Workspace) -> Result<(), UiMessage> {
        if workspace.explorer.generation() != self.generation || workspace.closing_window
            || workspace.tree.as_ref().is_none_or(|tree| tree.root != self.root)
        {
            return Err(delete_changed());
        }
        Ok(())
    }
}

fn workspaces(app: &App) -> Vec<WindowHandle<Workspace>> {
    app.windows().into_iter().filter_map(|window| window.downcast::<Workspace>()).collect()
}

fn checked_delete_target(root: &Path, path: &Path) -> Result<(PathBuf, bool), UiMessage> {
    let target = checked_existing(root, path)?;
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|error| UiMessage::new("tree.targetAccess").arg("error", error))?;
    Ok((target, metadata.is_dir()))
}

fn delete_changed() -> UiMessage {
    UiMessage::new("tree.deleteChanged")
}

fn delete_error(error: TextDocumentError) -> UiMessage {
    match error.code {
        "delete-changed" | "revision-conflict" | "document-closed" | "document-owner-conflict" => delete_changed(),
        "delete-copy-inside" => UiMessage::new("tree.deleteCopyInside"),
        "delete-copy-exists" => UiMessage::new("tree.deleteCopyExists"),
        "delete-copy-failed" => UiMessage::new("tree.deleteCopyFailed").arg("error", error),
        "delete-target" => UiMessage::raw(error),
        _ => UiMessage::new("tree.deleteFailed").arg("error", error),
    }
}
