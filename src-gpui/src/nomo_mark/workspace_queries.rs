//! 正式页面的全文查询。查询在会话后台执行，控件仍只消费命中附近的窗口。
use super::super::session_window::WindowRequest;
use super::*;
use crate::text_document::{
    DocumentStats, ReplaceQuery, SearchMatch, SearchQuery, StatsQuery,
};
use std::time::{Duration, Instant};
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub pending: bool,
    pub total: u64,
    pub current: Option<u64>,
    pub matches: Vec<SearchMatch>,
    pub error: Option<String>,
    task_id: Option<String>,
    request: WindowRequest,
    serial: u64,
}
#[derive(Default)]
pub(super) struct Queries {
    footnote_request: WindowRequest,
    pub search: SearchState,
    pub outline_state: super::outline_queries::OutlineState,
    pub stats: Option<DocumentStats>,
    selected_stats: Option<DocumentStats>,
    stats_selection: Option<DocumentSelection>,
    stats_requested_selection: Option<DocumentSelection>,
    pub stats_failed: bool,
    stats_enabled: bool,
    stats_revision: Option<u64>,
    stats_requested_revision: Option<u64>,
    stats_dirty_since: Option<Instant>,
    pub toc: super::toc_update::TocState,
    metadata_request: WindowRequest,
    metadata_task: Option<gpui::Task<()>>,
    serial: u64,
}
impl Drop for Queries {
    fn drop(&mut self) {
        self.footnote_request.cancel();
        self.metadata_request.cancel();
        self.search.request.cancel();
    }
}
fn error(e: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(e.to_string())
}

impl DocumentHost {
    pub(super) fn preview_footnote(&mut self, source: &Entity<NomoMark>, identity: &WindowIdentity, label: &str,
        tooltip: &gpui::WeakEntity<crate::nomo_mark::footnote_preview::FootnotePreview>, cx: &mut Context<Self>) {
        if self.hidden || source.read(cx).host.parsed.identity != *identity { return; }
        let Some(preview) = tooltip.upgrade() else { return; };
        self.queries.footnote_request.cancel();
        let request = preview.read(cx).request.clone();
        self.queries.footnote_request = request.clone();
        let port = self.port.clone();
        let label = label.to_string();
        let identity = identity.clone();
        let tooltip = tooltip.clone();
        let source = source.downgrade();
        let revision = identity.revision;
        let task = cx.background_executor().spawn(async move {
            crate::nomo_mark::footnote_preview::read(&port, revision, &label, &request)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden || !source.upgrade().is_some_and(|source| source.read(cx).host.parsed.identity == identity) { return; }
                if let Ok(text) = result {
                    let _ = tooltip.update(cx, |preview, cx| { preview.text = text; cx.notify(); });
                }
            });
        }).detach();
    }
    pub(crate) fn stats(&self) -> Option<&DocumentStats> {
        self.queries.selected_stats.as_ref().or(self.queries.stats.as_ref())
    }
    pub(crate) fn stats_failed(&self) -> bool {
        self.queries.stats_failed
    }
    pub(crate) fn sync_stats_visibility(&mut self, cx: &mut Context<Self>) {
        if self.queries.stats_enabled != cx.global::<crate::settings::Settings>().stats_visible {
            self.schedule_stats(cx);
        }
    }
    pub(crate) fn retry_stats(&mut self, cx: &mut Context<Self>) {
        if self.queries.stats_failed {
            self.queries.stats_requested_revision = None;
            self.schedule_stats(cx);
        }
    }
    pub(crate) fn search(&self) -> &SearchState {
        &self.queries.search
    }
    pub(super) fn cancel_queries(&mut self, cx: &mut Context<Self>) {
        self.metadata.save_dialog.cancel();
        self.queries.footnote_request.cancel();
        self.queries.search.serial += 1;
        self.cancel_stats();
        self.cancel_toc_when_hidden();
        // 非活动标签释放分页和任务，只保留六个统计数值与其修订，不持有正文。
        self.close_outline(cx);
        self.queries.search.matches.clear();
        self.queries.search.request.cancel();
        if let Some(id) = self.queries.search.task_id.take() {
            let port = self.port.clone();
            cx.background_executor()
                .spawn(async move {
                    port.manager.release_search(&port.owner, &id);
                })
                .detach();
        }
    }
    fn cancel_stats(&mut self) {
        self.port.manager.cancel_writing_stats(&self.port.owner, Some(self.queries.serial));
        self.queries.metadata_request.cancel();
        self.queries.metadata_task = None;
        self.queries.stats_requested_revision = None;
        self.queries.stats_dirty_since = None;
        self.queries.serial += 1;
    }
    pub(super) fn schedule_metadata(&mut self, cx: &mut Context<Self>) {
        self.refresh_outline(cx);
        self.schedule_stats(cx);
    }
    pub(super) fn schedule_stats(&mut self, cx: &mut Context<Self>) {
        self.queries.stats_enabled = cx.global::<crate::settings::Settings>().stats_visible;
        if self.hidden || self.closed || !self.queries.stats_enabled {
            self.cancel_stats();
            return;
        }
        let revision = self.status().revision;
        let selection = self.selection(cx);
        let selection = (!selection.range().is_empty()).then_some(selection);
        if selection != self.queries.stats_selection || self.queries.stats_revision != Some(revision) { self.queries.selected_stats = None; }
        if (self.queries.stats_revision == Some(revision) && self.queries.stats_selection == selection)
            || (self.queries.stats_requested_revision == Some(revision) && self.queries.stats_requested_selection == selection) { return; }
        // 首屏先让引用上下文恢复；已有统计刷新不再依赖局部窗口的引用查询。
        if self.queries.stats.is_none() && self.control.read(cx).host.parsed.context.pending_references { return; }
        self.queries.metadata_request.cancel();
        self.port.manager.cancel_writing_stats(&self.port.owner, Some(self.queries.serial));
        self.queries.metadata_request = WindowRequest::default();
        self.queries.serial += 1;
        self.queries.stats_requested_revision = Some(revision);
        self.queries.stats_requested_selection = selection;
        self.queries.stats_failed = false;
        let dirty = *self.queries.stats_dirty_since.get_or_insert_with(Instant::now);
        let delay = Duration::from_millis(300).min(Duration::from_millis(900).saturating_sub(dirty.elapsed()));
        let request = self.queries.metadata_request.clone();
        let serial = self.queries.serial;
        let port = self.port.clone();
        self.queries.metadata_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            if !this.update(cx, |host, _| {
                if host.queries.serial != serial || host.hidden || host.closed { return false; }
                host.queries.stats_dirty_since = Some(Instant::now());
                true
            }).unwrap_or(false) { return; }
            let task = cx.background_executor().spawn(async move {
                request.check()?;
                let stats = port
                    .manager
                    .writing_stats(
                        port.owner.clone(),
                        StatsQuery {
                            document_id: port.document_id.clone(),
                            revision,
                            request_id: serial,
                            selection_version: serial,
                            selection: selection.map(|s| crate::text_document::ByteRange { from: s.range().start, to: s.range().end }),
                        },
                    )
                    .await
                    .map_err(error)?;
                request.check()?;
                Ok::<_, ControlError>(stats)
            });
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden || host.closed || !host.queries.stats_enabled
                    || host.queries.serial != serial
                    || host.status().revision != revision
                {
                    return;
                }
                match result {
                    Ok(stats) => {
                        host.queries.stats = Some(stats.full);
                        host.queries.selected_stats = stats.selected;
                        host.queries.stats_selection = selection;
                        host.queries.stats_revision = Some(revision);
                        host.queries.stats_dirty_since = None;
                    }
                    Err(ControlError::Stale) => return,
                    Err(e) => {
                        host.queries.stats_requested_revision = None;
                        host.queries.stats_failed = true;
                        host.report(e, cx);
                    }
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        }));
    }
    pub(crate) fn set_search(
        &mut self,
        query: String,
        case_sensitive: bool,
        whole_word: bool,
        cx: &mut Context<Self>,
    ) {
        self.queries.search.request.cancel();
        let old = self.queries.search.task_id.take();
        self.queries.search.serial += 1;
        let serial = self.queries.search.serial;
        self.queries.search.query = query.clone();
        self.queries.search.case_sensitive = case_sensitive;
        self.queries.search.whole_word = whole_word;
        self.queries.search.matches.clear();
        self.queries.search.total = 0;
        self.queries.search.current = None;
        self.queries.search.error = None;
        self.queries.search.pending = !query.is_empty();
        self.queries.search.request = WindowRequest::default();
        let request = self.queries.search.request.clone();
        let port = self.port.clone();
        let revision = self.status().revision;
        let task = cx.background_executor().spawn(async move {
            if let Some(id) = old {
                port.manager.release_search(&port.owner, &id);
            }
            if query.is_empty() {
                return Ok(None);
            }
            request.check()?;
            let identity = port
                .manager
                .start_search(
                    &port.owner,
                    SearchQuery {
                        document_id: port.document_id.clone(),
                        base_revision: revision,
                        request_id: serial.to_string(),
                        query,
                        case_sensitive,
                        whole_word,
                    },
                    None,
                )
                .map_err(error)?;
            let result = (|| {
                loop {
                    request.check()?;
                    let progress = port
                        .manager
                        .search_progress(&port.owner, &identity.task_id)
                        .map_err(error)?;
                    match progress.state.as_str() {
                        "ready" => break,
                        "cancelled" | "conflict" | "failed" => {
                            return Err(error(
                                progress
                                    .error
                                    .map(|e| e.to_string())
                                    .unwrap_or(progress.state),
                            ));
                        }
                        _ => std::thread::sleep(Duration::from_millis(15)),
                    }
                }
                let page = port
                    .manager
                    .search_page(&port.owner, &identity.task_id, 0, 128)
                    .map_err(error)?;
                request.check()?;
                Ok(Some(page))
            })();
            if result.is_err() {
                port.manager.release_search(&port.owner, &identity.task_id);
            }
            result
        });
        let cleanup = self.port.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let id = result
                .as_ref()
                .ok()
                .and_then(|p| p.as_ref())
                .map(|p| p.identity.task_id.clone());
            let accepted = this
                .update(cx, |host, cx| {
                    if host.queries.search.serial != serial
                        || host.status().revision != revision
                        || host.closed
                        || host.hidden
                        || host.queries.search.request.check().is_err()
                    {
                        return false;
                    }
                    host.queries.search.pending = false;
                    match result {
                        Ok(Some(page)) => {
                            host.queries.search.total = page.total_count;
                            host.queries.search.matches = page.matches;
                            host.queries.search.task_id = Some(page.identity.task_id);
                        }
                        Ok(None) => {}
                        Err(ControlError::Stale) => {}
                        Err(e) => {
                            host.queries.search.error = Some(e.to_string());
                            host.report(e, cx);
                        }
                    }
                    cx.emit(DocumentEvent::Status);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !accepted {
                if let Some(id) = id {
                    cx.background_executor()
                        .spawn(async move {
                            cleanup.manager.release_search(&cleanup.owner, &id);
                        })
                        .await;
                }
            }
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn step_match(&mut self, forward: bool, wrap: bool, cx: &mut Context<Self>) {
        let state = &self.queries.search;
        if state.pending || state.total == 0 {
            return;
        }
        let index = match state.current {
            None => {
                if forward {
                    0
                } else {
                    state.total - 1
                }
            }
            Some(i) => {
                if !wrap && ((forward && i + 1 >= state.total) || (!forward && i == 0)) { return; }
                if forward { (i + 1) % state.total } else { (i + state.total - 1) % state.total }
            }
        };
        let Some(id) = state.task_id.clone() else {
            return;
        };
        let serial = state.serial;
        let revision = self.status().revision;
        let port = self.port.clone();
        let task = cx
            .background_executor()
            .spawn(async move { port.manager.search_page(&port.owner, &id, index, 128) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden
                    || host.closed
                    || host.status().revision != revision
                    || host.queries.search.serial != serial
                {
                    return;
                }
                match result {
                    Ok(page) => {
                        if let Some(found) = page.matches.first() {
                            let selection = DocumentSelection {
                                anchor_byte: found.from_byte + found.preserved_prefix.len() as u64,
                                head_byte: found.to_byte - found.preserved_suffix.len() as u64,
                            };
                            host.load_at_focused(selection.head_byte, Some(selection), false, cx);
                            host.queries.search.current = Some(index);
                        }
                        host.queries.search.matches = page.matches;
                    }
                    Err(e) => host.report(error(e), cx),
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn replace_current(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(index) = self.queries.search.current else {
            self.step_match(true, true, cx);
            return;
        };
        let Some(found) = self
            .queries
            .search
            .matches
            .iter()
            .find(|m| m.index == index)
            .cloned()
        else {
            return;
        };
        self.global_edit(
            DocumentSelection {
                anchor_byte: found.from_byte,
                head_byte: found.to_byte,
            },
            EditIntent::Replace(format!(
                "{}{replacement}{}",
                found.preserved_prefix, found.preserved_suffix
            )),
            cx,
        );
    }
    pub(crate) fn replace_all(&mut self, replacement: String, cx: &mut Context<Self>) {
        if self.queries.search.query.is_empty() || !self.ready_for_session_action(cx) {
            return;
        }
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let serial = self.queries.search.serial;
        let search = SearchQuery {
            document_id: port.document_id.clone(),
            base_revision: self.status().revision,
            request_id: format!("replace-{serial}"),
            query: self.queries.search.query.clone(),
            case_sensitive: self.queries.search.case_sensitive,
            whole_word: self.queries.search.whole_word,
        };
        let selection = self.selection(cx);
        let task = cx.background_executor().spawn(async move {
            let operation = format!(
                "replace-all-{}-{}",
                port.document_id,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let identity = port
                .manager
                .prepare_replace_all(
                    &port.owner,
                    ReplaceQuery {
                        search,
                        replacement,
                        operation_id: operation.clone(),
                        selection: Some(crate::text_document::MarkdownSelection {
                            anchor_byte: selection.anchor_byte,
                            head_byte: selection.head_byte,
                        }),
                        anchors: vec![selection.head_byte],
                    },
                    None,
                )
                .map_err(error)?;
            let result = (|| {
                loop {
                    request.check()?;
                    let progress = port
                        .manager
                        .search_progress(&port.owner, &identity.task_id)
                        .map_err(error)?;
                    match progress.state.as_str() {
                        "prepared" => break,
                        "failed" | "conflict" | "cancelled" => {
                            return Err(error(
                                progress
                                    .error
                                    .map(|e| e.to_string())
                                    .unwrap_or(progress.state),
                            ));
                        }
                        _ => std::thread::sleep(Duration::from_millis(15)),
                    }
                }
                request.check()?;
                let result = port
                    .manager
                    .commit_replace_all(
                        &port.owner,
                        &port.document_id,
                        &identity.task_id,
                        &operation,
                    )
                    .map_err(error)?;
                let byte = result.anchors.first().copied().unwrap_or(0);
                let loaded = port.load_group(result.status.revision, byte, None, &request)?;
                Ok::<_, ControlError>((loaded, byte, result.changed_count))
            })();
            port.manager.release_search(&port.owner, &identity.task_id);
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                match result {
                    Ok((loaded, byte, count)) => {
                        if let Err(e) =
                            host.install(loaded, Some(DocumentSelection::caret(byte)), cx)
                        {
                            host.action_failed(e, cx);
                        } else {
                            host.message = format!("已替换 {count} 处");
                        }
                    }
                    Err(e) => host.action_failed(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
