//! 正式页面的全文查询。查询在会话后台执行，控件仍只消费命中附近的窗口。
use super::super::session_window::WindowRequest;
use super::*;
use gpui::App;
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
    pub(super) task_id: Option<String>,
    pub(super) revision: u64,
    anchor: Option<u64>,
    pane_id: Option<u64>,
    first: u64,
    target: Option<u64>,
    navigation: WindowRequest,
    navigation_serial: u64,
    queued_steps: Vec<(bool, bool, u32)>,
    feedback_timer: Option<gpui::Task<()>>,
    pub(super) after_replace: Option<(u64, u64, bool)>,
    pub feedback: Option<String>,
    pub panel_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    pub(super) replacing: bool,
    request: WindowRequest,
    pub(super) serial: u64,
}
impl SearchState {
    fn destination(&self, forward: bool, wrap: bool, count: u32) -> Option<(u64, bool)> {
        if self.pending || self.total == 0 || self.replacing { return None; }
        let current = self.target.or(self.current);
        let index = if forward { current.map_or(self.first, |i| i + 1) as i128 }
            else { current.unwrap_or(self.first) as i128 - 1 };
        let crossed = !(0..self.total as i128).contains(&index);
        if crossed && !wrap { return None; }
        let extra = count.saturating_sub(1) as i128;
        let end = index + if forward { extra } else { -extra };
        let total = self.total as i128;
        let wrapped = wrap && !(0..total).contains(&end);
        Some((if wrap { end.rem_euclid(total) } else { end.clamp(0, total - 1) } as u64, wrapped))
    }
    pub fn can_step(&self, forward: bool, wrap: bool) -> bool { self.destination(forward, wrap, 1).is_some() }
    pub fn can_replace(&self) -> bool { self.current.is_some() && self.target.is_none() && !self.replacing && !self.pending }
    pub fn can_replace_all(&self) -> bool { self.total > 0 && !self.pending && !self.replacing }
    pub(super) fn current_range(&self) -> Option<std::ops::Range<u64>> {
        let i = self.current?;
        let m = self.matches.iter().find(|m| m.index == i)?;
        Some(m.from_byte + m.preserved_prefix.len() as u64..m.to_byte - m.preserved_suffix.len() as u64)
    }
}
#[derive(Default)]
pub(super) struct Queries {
    pub lint: super::markdown_lint::LintState,
    footnote_request: WindowRequest,
    pub search: SearchState,
    pub outline_state: super::outline_queries::OutlineState,
    pub stats: Option<DocumentStats>,
    selected_stats: Option<DocumentStats>,
    stats_selection: Option<DocumentSelection>,
    stats_requested_selection: Option<DocumentSelection>,
    stats_failed: bool,
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
        self.search.navigation.cancel();
        self.search.request.cancel();
    }
}
fn error(e: impl std::fmt::Display) -> ControlError {
    ControlError::HostFailure(e.to_string())
}
fn search_error(e: crate::text_document::TextDocumentError) -> ControlError {
    match e.code {
        "search-expired" | "task-cancelled" | "revision-conflict" | "document-closed" => ControlError::Stale,
        _ => error(e),
    }
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
    pub(crate) fn stats_are_selected(&self) -> bool {
        self.queries.selected_stats.is_some()
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
        self.queries.lint.cancel();
        self.metadata.save_dialog.cancel();
        self.queries.footnote_request.cancel();
        self.queries.search.serial += 1;
        self.cancel_stats();
        self.cancel_toc_when_hidden();
        // 隐藏时取消任务，保留有界标题分页和折叠状态；关闭会话才释放投影。
        if self.closed { self.close_outline(cx); }
        else { self.set_outline_enabled(false, cx.global::<crate::settings::Settings>().outline_level, cx); }
        self.queries.search.feedback_timer = None;
        self.queries.search.navigation.cancel();
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
        self.sync_search_marks(cx);
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
        self.sync_lint(cx);
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
        // 拖选和编辑期间保留最近一次选区结果，只有取消选择才立即恢复全文统计。
        if selection.is_none() { self.queries.selected_stats = None; }
        if self.queries.stats_revision == Some(revision) && self.queries.stats_selection == selection
            && (selection.is_none() || self.queries.selected_stats.is_some()) {
            // 快速回到已缓存的范围时，也必须取消中途发出的请求，避免迟到结果覆盖当前范围。
            if self.queries.stats_requested_revision.is_some()
                && (self.queries.stats_requested_revision != Some(revision) || self.queries.stats_requested_selection != selection) {
                self.cancel_stats();
            }
            return;
        }
        if self.queries.stats_requested_revision == Some(revision) && self.queries.stats_requested_selection == selection { return; }
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
                    .map_err(|err| match err.code {
                        "stats-cancelled" | "revision-conflict" | "document-closed" => ControlError::Stale,
                        _ => error(err),
                    })?;
                request.check()?;
                Ok::<_, ControlError>(stats)
            });
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                let current_selection = host.selection(cx);
                let current_selection = (!current_selection.range().is_empty()).then_some(current_selection);
                if host.hidden || host.closed || !host.queries.stats_enabled || host.queries.serial != serial { return; }
                if host.status().revision != revision || current_selection != selection {
                    host.queries.stats_requested_revision = None;
                    host.schedule_stats(cx);
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
                    Err(ControlError::Stale) => {
                        host.queries.stats_requested_revision = None;
                        return;
                    },
                    Err(e) => {
                        host.queries.stats_failed = true;
                        crate::app_logger::warn("WritingStats", &e.to_string());
                    }
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        }));
    }
    pub(crate) fn search_source_hint(&self, cx: &App) -> Option<&'static str> {
        self.control.read(cx).hidden_search_hit(self.queries.search.current_range()?)
    }

    pub(crate) fn set_search_panel_bounds(&mut self, bounds: gpui::Bounds<gpui::Pixels>) {
        self.queries.search.panel_bounds = Some(bounds);
    }

    pub(super) fn sync_search_marks(&self, cx: &mut Context<Self>) {
        let state = &self.queries.search;
        let task_id = state.task_id.as_ref().filter(|_| state.total > 0);
        let current = task_id.and_then(|_| state.current_range());
        for pane in &self.panes {
            for view in std::iter::once(&pane.control).chain(pane.viewport.mounts.iter().map(|m| &m.view).filter(|v| *v != &pane.control)) {
                let marks = &view.read(cx).find_marks;
                if marks.as_ref().map(|m| &m.task_id) == task_id && marks.as_ref().and_then(|m| m.current.as_ref()) == current.as_ref() { continue; }
                view.update(cx, |v, cx| {
                    if v.find_marks.as_ref().map(|m| &m.task_id) != task_id {
                        v.find_marks = task_id.map(|id| Box::new(super::super::view::search_highlights::VisibleMatches::new(id.clone(), state.revision)));
                    }
                    if let Some(marks) = &mut v.find_marks { marks.current = current.clone(); }
                    cx.notify();
                });
            }
        }
    }

    fn restore_find_pane(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.panes.iter().find(|pane| Some(pane.id) == self.queries.search.pane_id) else { return true };
        let view = pane.control.clone();
        self.control == view || self.promote(&view, cx)
    }

    pub(crate) fn focus_after_find(&mut self, cx: &mut Context<Self>) -> gpui::FocusHandle {
        self.restore_find_pane(cx);
        self.control.focus_handle(cx)
    }

    pub(crate) fn begin_find(&mut self, cx: &App) {
        self.queries.search.pane_id = Some(self.panes[self.active_pane].id);
        self.queries.search.anchor = Some(self.selection(cx).range().start);
    }

    pub(crate) fn end_find(&mut self, cx: &mut Context<Self>) {
        self.queries.search.panel_bounds = None;
        self.set_search(String::new(), false, false, cx);
        self.queries.search.anchor = None;
        self.queries.search.pane_id = None;
    }

    pub(crate) fn set_search(
        &mut self,
        query: String,
        case_sensitive: bool,
        whole_word: bool,
        cx: &mut Context<Self>,
    ) {
        let follow = self.queries.search.after_replace.take();
        let anchor = follow.map(|(_, end, _)| end).or(self.queries.search.anchor)
            .unwrap_or_else(|| self.selection(cx).range().start);
        self.queries.search.anchor = Some(anchor);
        self.queries.search.revision = self.status().revision;
        self.queries.search.queued_steps.clear();
        self.queries.search.feedback_timer = None;
        self.queries.search.target = None;
        self.queries.search.feedback = None;
        self.queries.search.navigation.cancel();
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
        if query.is_empty() {
            self.sync_search_marks(cx);
            if let Some(id) = old { cx.background_executor().spawn(async move { port.manager.release_search(&port.owner, &id); }).detach(); }
            cx.emit(DocumentEvent::Status);
            cx.notify();
            return;
        }
        let revision = self.status().revision;
        let debounce = cx.background_executor().timer(Duration::from_millis(50));
        let task = cx.background_executor().spawn(async move {
            if let Some(id) = old {
                port.manager.release_search(&port.owner, &id);
            }
            debounce.await;
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
                    .map_err(search_error)?;
            let result = (|| {
                loop {
                    request.check()?;
                    let progress = port
                        .manager
                        .search_progress(&port.owner, &identity.task_id)
                        .map_err(search_error)?;
                    match progress.state.as_str() {
                        "ready" => break,
                        "cancelled" | "conflict" => return Err(ControlError::Stale),
                        "failed" => {
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
                    .map_err(search_error)?;
                request.check()?;
                let rank = |at| port.manager.search_region(&port.owner, &identity.task_id, at, at)
                    .map(|(ordinal, _)| ordinal).map_err(search_error);
                let first = rank(anchor)?;
                let advance = if let Some((start, _, wrap)) = follow {
                    first < page.total_count || (wrap && rank(start)? > 0)
                } else { false };
                Ok((page, first, advance))
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
                .map(|(p, _, _)| p.identity.task_id.clone());
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
                        Ok((page, first, advance)) => {
                            host.queries.search.first = first;
                            host.queries.search.total = page.total_count;
                            host.queries.search.matches = page.matches;
                            host.queries.search.task_id = Some(page.identity.task_id);
                            let queued = std::mem::take(&mut host.queries.search.queued_steps);
                            if queued.is_empty() {
                                if advance { host.step_match(true, true, cx); }
                            } else {
                                let state = &mut host.queries.search;
                                let mut last = None;
                                for (forward, wrap, count) in queued {
                                    if let Some((index, crossed)) = state.destination(forward, wrap, count) {
                                        state.target = Some(index);
                                        last = Some((index, forward, crossed));
                                    }
                                }
                                state.target = None;
                                if let Some((index, forward, crossed)) = last { host.navigate_match(index, forward, crossed, cx); }
                            }
                        }
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
        if self.queries.search.pending {
            let queue = &mut self.queries.search.queued_steps;
            if let Some((_, _, count)) = queue.last_mut().filter(|(dir, cycle, _)| (*dir, *cycle) == (forward, wrap)) {
                *count = count.saturating_add(1);
            } else if queue.len() < 128 {
                queue.push((forward, wrap, 1));
            }
            return;
        }
        let Some((index, crossed)) = self.queries.search.destination(forward, wrap, 1) else { return; };
        self.navigate_match(index, forward, crossed, cx);
    }

    fn navigate_match(&mut self, index: u64, forward: bool, crossed: bool, cx: &mut Context<Self>) {
        if !self.restore_find_pane(cx) || !self.ready_for_session_action(cx) { return; }
        let state = &mut self.queries.search;
        let Some(id) = state.task_id.clone() else { return; };
        state.navigation.cancel();
        state.navigation = WindowRequest::default();
        state.navigation_serial += 1;
        state.target = Some(index);
        let sequence = state.navigation_serial;
        let serial = state.serial;
        let revision = state.revision;
        let request = state.navigation.clone();
        let cached = state.matches.iter().find(|m| m.index == index).cloned();
        let port = self.port.clone();
        let control = self.control.clone();
        let identity = control.read(cx).host.parsed.identity.clone();
        let loaded_range = identity.from_byte..identity.to_byte;
        let settle = cx.background_executor().timer(Duration::from_millis(10));
        let task = cx.background_executor().spawn(async move {
            settle.await;
            request.check()?;
            let page = if cached.is_none() {
                Some(port.manager.search_page(&port.owner, &id, index, 128).map_err(search_error)?)
            } else { None };
            let found = cached.as_ref().or_else(|| page.as_ref()?.matches.first()).ok_or(ControlError::Stale)?;
            let selection = DocumentSelection { anchor_byte: found.from_byte + found.preserved_prefix.len() as u64,
                head_byte: found.to_byte - found.preserved_suffix.len() as u64 };
            request.check()?;
            let loaded = if loaded_range.start <= selection.anchor_byte && selection.head_byte <= loaded_range.end { None }
                else { Some(port.load_group(revision, selection.head_byte, None, &request)?) };
            Ok::<_, ControlError>((selection, loaded, page))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.closed || host.hidden || host.queries.search.serial != serial
                    || host.queries.search.navigation_serial != sequence { return; }
                host.queries.search.target = None;
                if host.control != control || control.read(cx).host.parsed.identity != identity
                    || host.status().revision != revision || !host.ready_for_session_action(cx) {
                    cx.emit(DocumentEvent::Status); cx.notify(); return;
                }
                match result {
                    Ok((selection, loaded, page)) => {
                        if let Some(loaded) = loaded {
                            if let Err(e) = host.install(loaded, Some(selection), cx) { host.report(e, cx); return; }
                            host.focus_pending = false;
                        } else {
                            control.update(cx, |view, cx| view.restore_session_selection(selection, cx));
                            host.selection_changed(true, cx);
                        }
                        if let Some(page) = page { host.queries.search.matches = page.matches; }
                        host.queries.search.current = Some(index);
                        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
                        host.queries.search.feedback_timer = None;
                        host.queries.search.feedback = crossed.then(|| locale.text(if forward { "search_wrap_start" } else { "search_wrap_end" }).into());
                        if crossed {
                            let timer = cx.background_executor().timer(Duration::from_millis(1800));
                            host.queries.search.feedback_timer = Some(cx.spawn(async move |this, cx| {
                                timer.await;
                                let _ = this.update(cx, |host, cx| {
                                    host.queries.search.feedback = None; cx.emit(DocumentEvent::Status); cx.notify();
                                });
                            }));
                        }
                    }
                    Err(ControlError::Stale) => {
                        let state = &host.queries.search;
                        host.set_search(state.query.clone(), state.case_sensitive, state.whole_word, cx);
                    }
                    Err(e) => host.report(e, cx),
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        }).detach();
        cx.emit(DocumentEvent::Status);
        cx.notify();
    }
    pub(crate) fn replace_current(&mut self, replacement: &str, wrap: bool, cx: &mut Context<Self>) {
        if self.editing_locked() || !self.queries.search.can_replace()
            || self.queries.search.revision != self.status().revision
            || self.control.read(cx).host.parsed.identity.revision != self.queries.search.revision
            || !self.ready_for_session_action(cx) { return; }
        let Some(index) = self.queries.search.current else { return; };
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
        let text = format!("{}{replacement}{}", found.preserved_prefix, found.preserved_suffix);
        let selection = DocumentSelection { anchor_byte: found.from_byte, head_byte: found.to_byte };
        // 同内容判断以实际命中为准；不区分大小写时 Fox → fox 仍然是一次修改。
        let view = self.control.read(cx);
        if super::super::mapping::local_range(&view.host.parsed.source, view.host.parsed.identity.from_byte, selection.range())
            .ok().and_then(|r| view.host.parsed.source.get(r)) == Some(text.as_str()) {
            self.queries.search.feedback = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).text("search_unchanged").into());
            cx.emit(DocumentEvent::Status); cx.notify(); return;
        }
        self.global_edit_follow(selection, EditIntent::Replace(text), Some((self.queries.search.serial, wrap)), cx);
    }
    pub(crate) fn replace_all(&mut self, replacement: String, cx: &mut Context<Self>) {
        if self.editing_locked() || !self.queries.search.can_replace_all() || self.queries.search.revision != self.status().revision || !self.ready_for_session_action(cx) {
            return;
        }
        self.queries.search.replacing = true;
        cx.emit(DocumentEvent::Status);
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
                    .map_err(search_error)?;
            let result = (|| {
                loop {
                    request.check()?;
                    let progress = port
                        .manager
                        .search_progress(&port.owner, &identity.task_id)
                        .map_err(search_error)?;
                    match progress.state.as_str() {
                        "prepared" => break,
                        "cancelled" | "conflict" => return Err(ControlError::Stale),
                        "failed" => {
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
                    .map_err(search_error)?;
                if result.changed_count == 0 { return Ok(None); }
                let byte = result.anchors.first().copied().unwrap_or(0);
                let loaded = port.load_group(result.status.revision, byte, None, &request)?;
                Ok::<_, ControlError>(Some((loaded, byte, result.changed_count)))
            })();
            port.manager.release_search(&port.owner, &identity.task_id);
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                host.queries.search.replacing = false;
                let same_query = serial == host.queries.search.serial;
                match result {
                    Ok(Some((loaded, byte, count))) => {
                        if let Err(e) =
                            host.install(loaded, Some(DocumentSelection::caret(byte)), cx)
                        {
                            host.action_failed(e, cx);
                        } else {
                            host.focus_pending = false;
                            host.message = crate::ui::toolbar_i18n::ToolbarLocale::current(cx).app_format("host.replaced", &[("count", &count.to_string())]);
                            if same_query { host.queries.search.feedback = Some(host.message.clone()); }
                        }
                    }
                    Ok(None) => {
                        host.control.update(cx, |view, _| view.host.set_readonly(false));
                        host.message.clear();
                        if same_query { host.queries.search.feedback = Some(crate::ui::toolbar_i18n::ToolbarLocale::current(cx).text("search_unchanged").into()); }
                        host.schedule_viewport(16, cx);
                    }
                    Err(e) => host.action_failed(e, cx),
                }
                cx.emit(DocumentEvent::Status);
                cx.notify();
            });
        })
        .detach();
    }
}
