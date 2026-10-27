//! 窗口大纲的有界分页投影。正文和折叠树仍由文档会话及结构索引持有。
use super::*;
use crate::text_document::{StructureIdentity, StructureQuery};
use std::{collections::VecDeque, rc::Rc, time::Duration};

const MAX_PAGES: usize = 8;
const MAX_BYTES: usize = 4 * 1024 * 1024;

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
}
impl Drop for OutlineState { fn drop(&mut self) { self.request.cancel(); } }
impl OutlineState {
    fn clear_pages(&mut self) { self.pages.clear(); self.bytes = 0; self.count = 0; self.complete = false; self.has_collapsed = false; self.has_expandable = false; }
    fn insert(&mut self, page: Page) -> Result<(), ControlError> {
        let bytes = std::mem::size_of::<CachedPage>() + page.rows.capacity() * std::mem::size_of::<OutlineHeading>()
            + page.rows.iter().map(|h| h.text.capacity() + h.block_id.capacity()).sum::<usize>();
        if bytes > MAX_BYTES { return Err(ControlError::NeedsContext("单页大纲超过缓存预算".into())); }
        self.view_generation = page.view_generation;
        self.count = page.visible_count;
        self.complete = page.projection_complete;
        if let Some(has_collapsed) = page.has_collapsed { self.has_collapsed = has_collapsed; }
        if let Some(has_expandable) = page.has_expandable { self.has_expandable = has_expandable; }
        if !page.rows.is_empty() {
            if let Some(i) = self.pages.iter().position(|p| p.from == page.from_index) {
                if let Some(old) = self.pages.remove(i) { self.bytes = self.bytes.saturating_sub(old.bytes); }
            }
            while self.pages.len() >= MAX_PAGES || self.bytes + bytes > MAX_BYTES {
                if let Some(old) = self.pages.pop_front() { self.bytes = self.bytes.saturating_sub(old.bytes); } else { break; }
            }
            self.bytes += bytes;
            self.pages.push_back(CachedPage { from: page.from_index, rows: Rc::new(page.rows), bytes });
        } else if page.from_index < page.visible_count {
            return Err(ControlError::NeedsContext("大纲页没有返回可显示的标题".into()));
        }
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
        self.refresh_outline(cx);
    }
    pub(super) fn close_outline(&mut self, cx: &mut Context<Self>) {
        self.cancel_outline_navigation(cx);
        let state = &mut self.queries.outline_state;
        state.request.cancel(); state.serial += 1; state.active_serial += 1;
        state.pending = false; state.identity = None; state.error = None; state.active_byte = None; state.anchor = None; state.active_task = None; state.reset_level = false;
        state.clear_pages();
        if let Some(view) = state.view_id.take() {
            let port = self.port.clone();
            cx.background_executor().spawn(async move { let _ = port.manager.close_structure_outline(&port.owner, &port.document_id, &view); }).detach();
        }
    }
    pub(super) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        if self.hidden || self.closed || !self.queries.outline_state.enabled { return; }
        let state = &mut self.queries.outline_state;
        state.request.cancel(); state.request = Default::default(); state.serial += 1;
        state.active_serial += 1; state.active_byte = None; state.anchor = None; state.error = None; state.active_task = None;
        state.clear_pages(); state.pending = true;
        let request = state.request.clone(); let serial = state.serial;
        let old_view = state.view_id.clone(); let level = state.default_level;
        let revision = self.status().revision; let port = self.port.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let worker_port = port.clone();
            let task = cx.background_executor().spawn(async move {
                request.check()?;
                let identity = worker_port.ready_index(revision, &request)?;
                let created = old_view.is_none();
                let view = if let Some(view) = old_view { view } else {
                    let response = worker_port.manager.query_structure(&worker_port.owner, identity.clone(), StructureQuery::OpenOutline { default_level: level }).map_err(query_error)?;
                    response.items.first().and_then(|item| item["viewId"].as_str()).ok_or_else(|| query_error("大纲视图缺少身份"))?.to_owned()
                };
                let result = (|| { request.check()?; let (page, incomplete) = read_page(&worker_port, &identity, &view, 0, None)?; request.check()?; Ok::<_, ControlError>((identity, page, incomplete)) })();
                if result.is_err() && created { let _ = worker_port.manager.close_structure_outline(&worker_port.owner, &worker_port.document_id, &view); }
                result.map(|value| (value, created))
            });
            let result = task.await;
            let cleanup = result.as_ref().ok().filter(|(_, created)| *created).map(|((_, page, _), _)| page.view_id.clone());
            let accepted = this.update(cx, |host, cx| {
                if host.hidden || host.closed || !host.queries.outline_state.enabled || host.queries.outline_state.serial != serial || host.status().revision != revision { return false; }
                let state = &mut host.queries.outline_state; state.pending = false;
                match result {
                    Ok(((identity, page, incomplete), _)) => {
                        state.view_id = Some(page.view_id.clone()); state.identity = Some(identity); state.incomplete = incomplete;
                        if let Err(error) = state.insert(page) { state.error = Some(error.to_string()); }
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
        cx.notify();
    }
    pub(crate) fn request_outline_page(&mut self, from: usize, cx: &mut Context<Self>) {
        let s = &mut self.queries.outline_state;
        if s.pending || !s.enabled || s.error.is_some() || self.hidden { return; }
        let cached = s.pages.iter().any(|p| from >= p.from && from < p.from + p.rows.len());
        if cached && (s.complete || from + 20 < s.count) { return; }
        let (Some(identity), Some(view)) = (s.identity.clone(), s.view_id.clone()) else { return; };
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
        let s = &mut self.queries.outline_state;
        if s.pending || self.hidden { return; }
        let (Some(identity), Some(view)) = (s.identity.clone(), s.view_id.clone()) else { return; };
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
                match result { Ok((page, incomplete)) => { s.clear_pages(); s.incomplete = incomplete; if let Err(error) = s.insert(page) { s.error = Some(error.to_string()); } }, Err(error) => s.error = Some(error.to_string()) }
                let reset = s.reset_level; let level = s.default_level; s.reset_level = false;
                if reset { host.set_outline_level(level, cx); }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn update_outline_anchor(&mut self, byte: u64, cx: &mut Context<Self>) {
        let s = &mut self.queries.outline_state;
        if !s.enabled || s.anchor == Some(byte) { return; }
        let Some(identity) = s.identity.clone() else { return; };
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
                if host.hidden || host.closed || s.active_serial != serial { return; }
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
