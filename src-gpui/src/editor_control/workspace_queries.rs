//! 正式页面的全文查询。查询在会话后台执行，控件仍只消费命中附近的窗口。
use super::super::session_window::WindowRequest;
use super::*;
use crate::text_document::{
    DocumentStats, ReplaceQuery, SearchMatch, SearchQuery, StatsQuery,
};
use std::time::Duration;
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
    pub search: SearchState,
    pub outline_state: super::outline_queries::OutlineState,
    pub stats: Option<DocumentStats>,
    pub toc: super::toc_update::TocState,
    metadata_request: WindowRequest,
    metadata_task: Option<gpui::Task<()>>,
    serial: u64,
}
impl Drop for Queries {
    fn drop(&mut self) {
        self.metadata_request.cancel();
        self.search.request.cancel();
    }
}
fn error(e: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(e.to_string())
}

impl DocumentHost {
    pub(crate) fn stats(&self) -> Option<&DocumentStats> {
        self.queries.stats.as_ref()
    }
    pub(crate) fn search(&self) -> &SearchState {
        &self.queries.search
    }
    pub(super) fn cancel_queries(&mut self, cx: &mut Context<Self>) {
        self.queries.search.serial += 1;
        self.queries.metadata_request.cancel();
        self.queries.metadata_task = None;
        self.cancel_toc_when_hidden();
        // 非活动标签不保留页面查询结果；正文、历史、选区和阅读位置仍属于会话/视图。
        self.close_outline(cx);
        self.queries.stats = None;
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
        self.port
            .manager
            .cancel_writing_stats(&self.port.owner, Some(self.queries.serial));
    }
    pub(super) fn schedule_metadata(&mut self, cx: &mut Context<Self>) {
        self.queries.metadata_request.cancel();
        self.queries.metadata_request = WindowRequest::default();
        self.queries.serial += 1;
        self.queries.stats = None;
        if self.hidden {
            return;
        }
        self.refresh_outline(cx);
        let request = self.queries.metadata_request.clone();
        let serial = self.queries.serial;
        let port = self.port.clone();
        let revision = self.status().revision;
        self.queries.metadata_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
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
                            selection_version: 0,
                            selection: None,
                        },
                    )
                    .await
                    .map_err(error)?;
                request.check()?;
                request.check()?;
                Ok::<_, ControlError>(stats.full)
            });
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden
                    || host.queries.serial != serial
                    || host.status().revision != revision
                {
                    return;
                }
                match result {
                    Ok(stats) => {
                        host.queries.stats = Some(stats);
                    }
                    Err(ControlError::Stale) => return,
                    Err(e) => host.report(e, cx),
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
