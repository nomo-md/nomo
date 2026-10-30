//! 窗口大纲的有界分页投影。正文和折叠树仍由文档会话及结构索引持有。
use super::*;
use crate::text_document::{StructureIdentity, StructureQuery};
use std::{collections::VecDeque, rc::Rc, time::{Duration, Instant}};

const MAX_PAGES: usize = 8;
const MAX_BYTES: usize = 4 * 1024 * 1024;
// 在总预算内预留候选页、反序列化与换帧期间的旧页引用。
const CACHE_BYTES: usize = MAX_BYTES - 1024 * 1024;

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineHeading {
    pub position: usize,
    pub block_id: String,
    pub block_heading_index: u64,
    pub byte: u64,
    pub to_byte: u64,
    pub line: u64,
    pub level: u8,
    #[serde(rename = "title")]
    pub text: String,
    pub collapsed: bool,
    pub has_children: bool,
}
impl OutlineHeading {
    pub fn key(&self) -> String { format!("{}:{}", self.block_id, self.block_heading_index) }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    view_id: String,
    view_generation: u64,
    visible_count: usize,
    projection_complete: bool,
    from_index: usize,
    has_collapsed: Option<bool>,
    has_expandable: Option<bool>,
    rows: Vec<OutlineHeading>,
}

#[derive(Clone)]
struct CachedPage { from: usize, rows: Rc<Vec<OutlineHeading>>, bytes: usize }

#[derive(Clone, Default)]
pub(crate) struct OutlineSnapshot {
    pages: Vec<CachedPage>,
    pub count: usize,
    pub complete: bool,
    pub pending: bool,
    pub incomplete: bool,
    pub error: Option<String>,
    pub active_byte: Option<u64>,
    pub revision: u64,
    pub view_generation: u64,
    pub has_collapsed: bool,
    pub has_expandable: bool,
}
impl OutlineSnapshot {
    pub fn row(&self, index: usize) -> Option<&OutlineHeading> {
        self.pages.iter().find_map(|page| index.checked_sub(page.from).and_then(|i| page.rows.get(i)))
    }
    pub fn position(&self, key: &str) -> Option<usize> {
        self.pages.iter().flat_map(|page| page.rows.iter()).find(|row| row.key() == key).map(|row| row.position)
    }
}

#[derive(Default)]
pub(super) struct OutlineState {
    enabled: bool,
    default_level: u8,
    reset_level: bool,
    identity: Option<StructureIdentity>,
    view_id: Option<String>,
    view_generation: u64,
    pages: VecDeque<CachedPage>,
    bytes: usize,
    count: usize,
    complete: bool,
    pending: bool,
    incomplete: bool,
    error: Option<String>,
    active_byte: Option<u64>,
    anchor: Option<u64>,
    active_serial: u64,
    active_task: Option<gpui::Task<()>>,
    serial: u64,
    has_collapsed: bool,
    has_expandable: bool,
    request: super::super::session_window::WindowRequest,
    refresh_task: Option<gpui::Task<()>>,
    requested_revision: Option<u64>,
    dirty_since: Option<Instant>,
    visible_from: usize,
    restore_scroll: Option<usize>,
    navigation_request: super::super::session_window::WindowRequest,
}
impl Drop for OutlineState { fn drop(&mut self) { self.request.cancel(); self.navigation_request.cancel(); } }
impl OutlineState {
    fn clear_pages(&mut self) { self.pages.clear(); self.bytes = 0; self.count = 0; self.complete = false; self.has_collapsed = false; self.has_expandable = false; }
    fn insert(&mut self, page: Page) -> Result<(), ControlError> {
        let bytes = std::mem::size_of::<CachedPage>() + page.rows.capacity() * std::mem::size_of::<OutlineHeading>()
            + page.rows.iter().map(|h| h.text.capacity() + h.block_id.capacity()).sum::<usize>();
        if bytes > CACHE_BYTES { return Err(ControlError::NeedsContext("单页大纲超过缓存预算".into())); }
        self.view_generation = page.view_generation;
        self.count = page.visible_count;
        self.complete = page.projection_complete;
        if let Some(has_collapsed) = page.has_collapsed { self.has_collapsed = has_collapsed; }
        if let Some(has_expandable) = page.has_expandable { self.has_expandable = has_expandable; }
        if !page.rows.is_empty() {
            if let Some(i) = self.pages.iter().position(|p| p.from == page.from_index) {
                if let Some(old) = self.pages.remove(i) { self.bytes = self.bytes.saturating_sub(old.bytes); }
            }
            while self.pages.len() >= MAX_PAGES || self.bytes + bytes > CACHE_BYTES {
                if let Some(old) = self.pages.pop_front() { self.bytes = self.bytes.saturating_sub(old.bytes); } else { break; }
            }
            self.bytes += bytes;
            self.pages.push_back(CachedPage { from: page.from_index, rows: Rc::new(page.rows), bytes });
        } else if page.from_index < page.visible_count {
            return Err(ControlError::NeedsContext("大纲页没有返回可显示的标题".into()));
        }
        Ok(())
    }
    fn replace_page(&mut self, page: Page) -> Result<(), ControlError> {
        let mut next = Self::default();
        next.insert(page)?;
        // 候选页全部验证后交换；不把不同 revision 的分页混在同一列表。
        self.pages = std::mem::take(&mut next.pages);
        self.bytes = next.bytes;
        self.count = next.count;
        self.complete = next.complete;
        self.view_generation = next.view_generation;
        self.has_collapsed = next.has_collapsed;
        self.has_expandable = next.has_expandable;
        Ok(())
    }
}

fn query_error(error: impl std::fmt::Display) -> ControlError { ControlError::HostFailure(error.to_string()) }
fn read_page(port: &NativeWindowPort, identity: &StructureIdentity, view: &str, from: usize, generation: Option<u64>) -> Result<(Page, bool), ControlError> {
    let response = port.manager.query_structure(&port.owner, identity.clone(), StructureQuery::OutlineView {
        view_id: view.into(), from_index: from as u64, view_generation: generation,
    }).map_err(query_error)?;
    let incomplete = !response.semantic_complete;
    let page = response.items.into_iter().next().ok_or_else(|| query_error("大纲页缺少结果"))?;
    Ok((serde_json::from_value(page).map_err(query_error)?, incomplete))
}

fn read_refresh_page(port: &NativeWindowPort, identity: &StructureIdentity, view: &str, from: usize,
    anchor: Option<(String, u64)>, request: &super::super::session_window::WindowRequest) -> Result<(Page, bool), ControlError> {
    let (page, incomplete) = read_page(port, identity, view, 0, None)?;
    if from == 0 || page.visible_count == 0 { return Ok((page, incomplete)); }
    let flags = (page.has_collapsed, page.has_expandable);
    drop(page);
    let mut target = from;
    let started = Instant::now();
    loop {
        request.check()?;
        if started.elapsed() > Duration::from_secs(30) { return Err(ControlError::NeedsContext("大纲定位尚未完成，可重试".into())); }
        if let Some((key, byte)) = &anchor {
            let located = port.manager.query_structure(&port.owner, identity.clone(), StructureQuery::LocateOutline {
                view_id: view.into(), heading_id: key.clone(), byte: *byte, parent: false,
            }).map_err(query_error)?;
            if let Some(index) = located.items.first().and_then(|item| item["position"].as_u64()) {
                target = index as usize;
            } else if !located.items.first().is_some_and(|item| item["complete"] == true) { continue; }
        }
        let (mut page, incomplete) = read_page(port, identity, view, target, None)?;
        if target >= page.visible_count && page.visible_count > 0 && page.projection_complete {
            target = page.visible_count - 1;
            continue;
        }
        if page.rows.is_empty() && !page.projection_complete { continue; }
        page.has_collapsed = flags.0; page.has_expandable = flags.1;
        return Ok((page, incomplete));
    }
}

impl DocumentHost {
    pub(crate) fn outline_snapshot(&self) -> OutlineSnapshot {
        let s = &self.queries.outline_state;
        OutlineSnapshot { pages: s.pages.iter().cloned().collect(), count: s.count, complete: s.complete,
            pending: s.pending, incomplete: s.incomplete, error: s.error.clone(), active_byte: s.active_byte,
            revision: s.identity.as_ref().map_or(0, |id| id.revision), view_generation: s.view_generation,
            has_collapsed: s.has_collapsed, has_expandable: s.has_expandable }
    }
    pub(crate) fn set_outline_enabled(&mut self, enabled: bool, level: u8, cx: &mut Context<Self>) {
        let changed = self.queries.outline_state.enabled != enabled;
        let level_changed = self.queries.outline_state.default_level != level;
        self.queries.outline_state.enabled = enabled;
        self.queries.outline_state.default_level = level.clamp(1, 6);
        if !enabled {
            if changed { self.close_outline(cx); }
        } else if !self.hidden && !self.queries.outline_state.pending && (changed || (self.queries.outline_state.identity.is_none() && self.queries.outline_state.error.is_none())) {
            self.refresh_outline(cx);
        } else if level_changed {
            if self.queries.outline_state.pending { self.queries.outline_state.reset_level = true; }
            else if self.queries.outline_state.view_id.is_some() { self.set_outline_level(level, cx); }
        }
    }
    pub(crate) fn retry_outline(&mut self, cx: &mut Context<Self>) {
        self.queries.outline_state.error = None;
        self.queries.outline_state.requested_revision = None;
        self.refresh_outline(cx);
    }
    pub(crate) fn take_outline_scroll(&mut self) -> Option<usize> { self.queries.outline_state.restore_scroll.take() }
    pub(super) fn suspend_outline_for_mini(&mut self, active: bool, cx: &mut Context<Self>) {
        let revision = self.status().revision;
        let state = &mut self.queries.outline_state;
        if active {
            // 保留原有页数/字节预算内的投影，隐藏期间取消查询与定位，不额外复制正文。
            if state.pending { state.requested_revision = None; }
            state.request.cancel(); state.request = Default::default(); state.serial += 1;
            state.navigation_request.cancel(); state.navigation_request = Default::default();
            state.refresh_task = None; state.pending = false;
            state.active_serial += 1; state.active_task = None; state.anchor = None;
        } else if state.enabled && (state.identity.is_none() || state.requested_revision != Some(revision)) {
            self.refresh_outline(cx);
        }
    }
    pub(super) fn close_outline(&mut self, cx: &mut Context<Self>) {
        self.cancel_outline_navigation(cx);
        let state = &mut self.queries.outline_state;
        state.request.cancel(); state.serial += 1; state.active_serial += 1;
        state.navigation_request.cancel(); state.refresh_task = None; state.requested_revision = None;
        state.dirty_since = None; state.visible_from = 0; state.restore_scroll = None;
        state.pending = false; state.identity = None; state.error = None; state.active_byte = None; state.anchor = None; state.active_task = None; state.reset_level = false;
        state.clear_pages();
        if let Some(view) = state.view_id.take() {
            let port = self.port.clone();
            cx.background_executor().spawn(async move { let _ = port.manager.close_structure_outline(&port.owner, &port.document_id, &view); }).detach();
        }
    }
    pub(super) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        if self.hidden || self.closed || self.mini_layout || !self.queries.outline_state.enabled { return; }
        let revision = self.status().revision;
        if self.queries.outline_state.requested_revision == Some(revision) { return; }
        let waiting = self.control.read(cx).host.parsed.context.pending_references;
        let state = &mut self.queries.outline_state;
        state.request.cancel(); state.request = Default::default(); state.serial += 1;
        state.navigation_request.cancel(); state.refresh_task = None;
        state.active_serial += 1; state.anchor = None; state.error = None; state.active_task = None;
        state.pending = true;
        // 旧投影继续显示；只有首次加载才等待首屏引用完成。
        if waiting && state.identity.is_none() { state.requested_revision = None; return; }
        state.requested_revision = Some(revision);
        let serial = state.serial;
        let dirty = *state.dirty_since.get_or_insert_with(Instant::now);
        let delay = Duration::from_millis(300).min(Duration::from_millis(900).saturating_sub(dirty.elapsed()));
        state.refresh_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden || host.closed || host.status().revision != revision || host.queries.outline_state.serial != serial { return; }
                host.queries.outline_state.dirty_since = Some(Instant::now());
                host.load_outline_refresh(cx);
            });
        }));
        cx.notify();
    }
    fn load_outline_refresh(&mut self, cx: &mut Context<Self>) {
        let state = &self.queries.outline_state;
        let request = state.request.clone(); let serial = state.serial;
        let old_view = state.view_id.clone(); let level = state.default_level;
        let from = state.visible_from;
        let anchor = state.pages.iter().find_map(|page| from.checked_sub(page.from).and_then(|i| page.rows.get(i)))
            .map(|row| (row.key(), row.byte));
        let active = state.active_byte;
        let active_key = state.pages.iter().flat_map(|page| page.rows.iter()).find(|row| Some(row.byte) == active).map(OutlineHeading::key);
        let revision = self.status().revision; let port = self.port.clone();
        cx.spawn(async move |this, cx| {
            let worker_port = port.clone();
            let task = cx.background_executor().spawn(async move {
                request.check()?;
                let identity = worker_port.ready_index(revision, &request)?;
                let created = old_view.is_none();
                let view = if let Some(view) = old_view { view } else {
                    let response = worker_port.manager.query_structure(&worker_port.owner, identity.clone(), StructureQuery::OpenOutline { default_level: level }).map_err(query_error)?;
                    response.items.first().and_then(|item| item["viewId"].as_str()).ok_or_else(|| query_error("大纲视图缺少身份"))?.to_owned()
                };
                let result = (|| { request.check()?; let (page, incomplete) = read_refresh_page(&worker_port, &identity, &view, from, anchor, &request)?; request.check()?; Ok::<_, ControlError>((identity, page, incomplete)) })();
                if result.is_err() && created { let _ = worker_port.manager.close_structure_outline(&worker_port.owner, &worker_port.document_id, &view); }
                result.map(|value| (value, created))
            });
            let result = task.await;
            let cleanup = result.as_ref().ok().filter(|(_, created)| *created).map(|((_, page, _), _)| page.view_id.clone());
            let accepted = this.update(cx, |host, cx| {
                if host.hidden || host.closed || !host.queries.outline_state.enabled || host.queries.outline_state.serial != serial || host.status().revision != revision { return false; }
                if host.queries.outline_state.visible_from != from {
                    host.queries.outline_state.requested_revision = None;
                    host.refresh_outline(cx);
                    return false;
                }
                let state = &mut host.queries.outline_state; state.pending = false;
                match result {
                    Ok(((identity, page, incomplete), _)) => {
                        let view = page.view_id.clone(); let restored = page.from_index;
                        let active = active_key.as_ref().and_then(|key| page.rows.iter().find(|row| row.key() == *key)).map(|row| row.byte);
                        state.view_id = Some(view);
                        match state.replace_page(page) {
                            Ok(()) => {
                                state.identity = Some(identity); state.incomplete = incomplete;
                                state.active_byte = active; state.anchor = None; state.dirty_since = None;
                                if restored != from { state.restore_scroll = Some(restored); }
                            }
                            Err(error) => state.error = Some(error.to_string()),
                        }
                    }
                    Err(ControlError::Stale) => {},
                    Err(error) => state.error = Some(error.to_string()),
                }
                let reset = state.reset_level && state.identity.is_some();
                let level = state.default_level; state.reset_level = false;
                if reset { host.set_outline_level(level, cx); }
                cx.notify(); true
            }).unwrap_or(false);
            if !accepted { if let Some(view) = cleanup { cx.background_executor().spawn(async move { let _ = port.manager.close_structure_outline(&port.owner, &port.document_id, &view); }).await; } }
        }).detach();
    }
    pub(crate) fn request_outline_page(&mut self, from: usize, cx: &mut Context<Self>) {
        let revision = self.status().revision;
        let s = &mut self.queries.outline_state;
        s.visible_from = from;
        if s.pending || !s.enabled || s.error.is_some() || self.hidden || self.mini_layout { return; }
        let cached = s.pages.iter().any(|p| from >= p.from && from < p.from + p.rows.len());
        if cached && (s.complete || from + 20 < s.count) { return; }
        let (Some(identity), Some(view)) = (s.identity.clone(), s.view_id.clone()) else { return; };
        if identity.revision != revision { return; }
        let from = if cached && from + 20 >= s.count && !s.complete { s.count } else { from };
        let serial = s.serial; let generation = s.view_generation; let request = s.request.clone(); let revision = identity.revision; s.pending = true;
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move { request.check()?; let result = read_page(&port, &identity, &view, from, Some(generation))?; request.check()?; Ok::<_, ControlError>(result) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden || host.closed || host.status().revision != revision || host.queries.outline_state.serial != serial { return; }
                let s = &mut host.queries.outline_state; s.pending = false;
                match result { Ok((page, incomplete)) => { s.incomplete = incomplete; if let Err(error) = s.insert(page) { s.error = Some(error.to_string()); } }, Err(error) => s.error = Some(error.to_string()) }
                let reset = s.reset_level; let level = s.default_level; s.reset_level = false;
                if reset { host.set_outline_level(level, cx); }
                cx.notify();
            });
        }).detach();
    }
    pub(crate) fn set_outline_fold(&mut self, heading: &OutlineHeading, cx: &mut Context<Self>) {
        self.mutate_outline(StructureQuery::SetOutlineFold { view_id: self.queries.outline_state.view_id.clone().unwrap_or_default(), heading_id: heading.key(), collapsed: !heading.collapsed }, cx);
    }
    pub(crate) fn navigate_outline_entry(&mut self, heading: &OutlineHeading, revision: u64, cx: &mut Context<Self>) {
        let state = &mut self.queries.outline_state;
        state.navigation_request.cancel(); state.navigation_request = Default::default();
        if self.status().revision == revision { self.reveal_outline_heading(heading.byte, revision, cx); return; }
        if self.hidden || self.closed || !self.ready_for_session_action(cx) { return; }
        // 旧投影解析会复核滚动位置，先停下上次动画，避免把程序滚动误判为用户取消。
        self.cancel_outline_navigation(cx);
        // 刷新期间仍可点击旧标题，但必须在当前索引按身份解析，绝不沿用旧字节坐标。
        let request = self.queries.outline_state.navigation_request.clone();
        let completion = request.clone();
        let revision = self.status().revision; let key = heading.key(); let port = self.port.clone();
        let pane = self.active_pane; let pane_id = self.panes[pane].id;
        let selection = self.selection(cx); let offset = self.panes[pane].scroll.offset();
        let task = cx.background_executor().spawn(async move {
            let identity = port.ready_index(revision, &request)?;
            request.check()?;
            let response = port.manager.query_structure(&port.owner, identity,
                StructureQuery::Resolve { namespace: "heading".into(), key }).map_err(query_error)?;
            request.check()?;
            let byte = response.items.first().and_then(|item| item["fromByte"].as_u64());
            Ok::<_, ControlError>(byte)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if completion.check().is_err() || host.hidden || host.closed || host.status().revision != revision || host.active_pane != pane
                    || host.panes[pane].id != pane_id || host.panes[pane].scroll.offset() != offset || host.selection(cx) != selection { return; }
                match result {
                    Ok(Some(byte)) => host.reveal_outline_heading(byte, revision, cx),
                    Ok(None) => host.report(ControlError::NeedsContext("标题已变化，请在更新后的大纲中选择".into()), cx),
                    Err(ControlError::Stale) => {},
                    Err(error) => host.report(error, cx),
                }
            });
        }).detach();
    }
    pub(crate) fn move_outline_section(&mut self, source: &OutlineHeading, target: &OutlineHeading, placement: crate::text_document::markdown_session::OutlinePlacement, revision: u64, cx: &mut Context<Self>) {
        if self.status().revision != revision { self.report(ControlError::Stale, cx); return; }
        if !self.ready_for_session_action(cx) { self.report(ControlError::NeedsContext("编辑器正忙，暂时不能移动章节。".into()), cx); return; }
        if self.control.read(cx).host.presentation != Presentation::Editable { self.report(ControlError::ReadOnly, cx); return; }
        let Some(identity) = self.queries.outline_state.identity.clone().filter(|id| id.revision == revision) else { self.report(ControlError::Stale, cx); return; };
        let selected = self.selection(cx);
        let operation = format!("outline-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let move_request = crate::text_document::markdown_session::OutlineMove { identity, source: source.key(), target: target.key(), placement, operation,
            selection: Some(crate::text_document::MarkdownSelection { anchor_byte: selected.anchor_byte, head_byte: selected.head_byte }) };
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            request.check()?;
            let moved = port.manager.move_outline_section(&port.owner, move_request).map_err(query_error)?;
            let loaded = port.load_group(moved.status.revision, moved.byte, None, &request)?;
            Ok::<_, ControlError>((loaded, moved.byte))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                match result.and_then(|(loaded, byte)| host.install(loaded, Some(DocumentSelection::caret(byte)), cx)) {
                    Ok(()) => host.message = "章节已移动".into(),
                    Err(error) => { host.refresh_status(cx); host.action_failed(error, cx); },
                }
                cx.notify();
            });
        }).detach();
    }
    pub(crate) fn set_outline_level(&mut self, level: u8, cx: &mut Context<Self>) {
        self.mutate_outline(StructureQuery::SetOutlineLevel { view_id: self.queries.outline_state.view_id.clone().unwrap_or_default(), level: level.clamp(1, 6) }, cx);
    }
    fn mutate_outline(&mut self, query: StructureQuery, cx: &mut Context<Self>) {
        let revision = self.status().revision;
        let s = &mut self.queries.outline_state;
        if s.pending || self.hidden { return; }
        let (Some(identity), Some(view)) = (s.identity.clone(), s.view_id.clone()) else { return; };
        if identity.revision != revision { return; }
        // 折叠查询期间继续显示旧投影，成功后再一次性交换，避免整张大纲闪为空状态。
        s.serial += 1; let serial = s.serial; let request = s.request.clone(); let revision = identity.revision; s.pending = true; s.error = None;
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            request.check()?;
            port.manager.query_structure(&port.owner, identity.clone(), query).map_err(query_error)?;
            request.check()?;
            let result = read_page(&port, &identity, &view, 0, None)?;
            request.check()?;
            Ok::<_, ControlError>(result)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                if host.hidden || host.closed || host.status().revision != revision || host.queries.outline_state.serial != serial { return; }
                let s = &mut host.queries.outline_state; s.pending = false;
                match result { Ok((page, incomplete)) => { if let Err(error) = s.replace_page(page) { s.error = Some(error.to_string()); } else { s.incomplete = incomplete; } }, Err(error) => s.error = Some(error.to_string()) }
                let reset = s.reset_level; let level = s.default_level; s.reset_level = false;
                if reset { host.set_outline_level(level, cx); }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn update_outline_anchor(&mut self, byte: u64, cx: &mut Context<Self>) {
        let revision = self.status().revision;
        let s = &mut self.queries.outline_state;
        if !s.enabled || self.mini_layout || s.anchor == Some(byte) { return; }
        let Some(identity) = s.identity.clone() else { return; };
        if identity.revision != revision { return; }
        s.anchor = Some(byte); s.active_serial += 1; let serial = s.active_serial;
        let port = self.port.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(30)).await;
            let task = cx.background_executor().spawn(async move {
                port.manager.query_structure(&port.owner, identity, StructureQuery::HeadingAtByte { byte }).map_err(query_error)
            });
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                let s = &mut host.queries.outline_state;
                if host.hidden || host.closed || host.metadata.status.revision != revision || s.active_serial != serial { return; }
                if let Ok(response) = result { s.active_byte = response.items.first().and_then(|item| item["byte"].as_u64()); cx.notify(); }
            });
        });
        self.queries.outline_state.active_task = Some(task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(index: usize, title_bytes: usize) -> Page {
        Page { view_id: "test".into(), view_generation: 1, visible_count: 20,
            projection_complete: true, from_index: index, has_collapsed: None, has_expandable: None,
            rows: vec![OutlineHeading { position: index, block_id: format!("block-{index}"), block_heading_index: 0,
                byte: index as u64, to_byte: index as u64 + 1, line: index as u64 + 1, level: 1,
                text: "a".repeat(title_bytes), collapsed: false, has_children: false }] }
    }

    #[test]
    fn outline_cache_enforces_page_and_byte_limits() {
        let mut state = OutlineState::default();
        for index in 0..9 { state.insert(page(index, 16)).unwrap(); }
        assert_eq!(state.pages.len(), MAX_PAGES);
        assert_eq!(state.pages.front().unwrap().from, 1);
        for index in 9..18 { state.insert(page(index, 600_000)).unwrap(); }
        assert!(state.pages.len() <= MAX_PAGES);
        assert!(state.bytes <= MAX_BYTES);
        assert_eq!(state.pages.back().unwrap().from, 17);
        assert!(state.insert(page(19, MAX_BYTES)).is_err());
        state.clear_pages();
        assert!(state.pages.is_empty());
        assert_eq!(state.bytes, 0);
    }
}
