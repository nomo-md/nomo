//! 仅已挂载且可见的正文请求命中区间，扫描复用同一搜索快照的稀疏检查点。
use super::*;

pub(crate) struct VisibleMatches {
    pub(super) revision: u64,
    pub(crate) task_id: String,
    pub(crate) current: Option<Range<u64>>,
    requested: Range<u64>,
    pub(crate) ranges: Vec<Range<u64>>,
    generation: u64,
    job: Option<gpui::Task<()>>,
    hint: RefCell<Option<(std::rc::Weak<BlockLayout>, Range<usize>, Option<&'static str>)>>,
}

impl VisibleMatches {
    pub(crate) fn new(task_id: String, revision: u64) -> Self {
        Self { task_id, revision, current: None, requested: 0..0, ranges: Vec::new(), generation: 0, job: None, hint: Default::default() }
    }
}

impl NomoMark {
    pub(crate) fn hidden_search_hit(&self, range: Range<u64>) -> Option<&'static str> {
        if self.effective_mode() != ViewMode::Semantic { return None; }
        let parsed = &self.host.parsed;
        let local = local_range(&parsed.source, parsed.identity.from_byte, range).ok()?;
        let layout = self.layout.as_ref()?;
        let marks = self.find_marks.as_ref()?;
        let key = Rc::downgrade(layout);
        if let Some((last, found, hint)) = &*marks.hint.borrow() {
            if last.ptr_eq(&key) && *found == local { return *hint; }
        }
        let hint = if layout.source_rects_clipped(local.clone(), 0.0..layout.height).is_empty() {
            let link = parsed.events.iter().any(|(event, r)| matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. }))
                && r.start <= local.start && local.end <= r.end);
            Some(if link { "search_hidden_link" } else { "search_hidden" })
        } else { None };
        *marks.hint.borrow_mut() = Some((key, local, hint));
        hint
    }

    pub(super) fn update_visible_matches(&mut self, cx: &mut Context<Self>) {
        let Some(marks) = self.find_marks.as_mut() else { return };
        let id = &self.host.parsed.identity;
        if id.revision != marks.revision || self.host.pending_count() != 0 { return; }
        let (Some(visible), Some(bounds), Some(layout)) = (self.visible_bounds, self.bounds, &self.layout) else { return };
        if visible.is_empty() {
            if !marks.requested.is_empty() {
                marks.generation += 1;
                marks.requested = 0..0;
                marks.ranges = Vec::new();
                marks.job = None;
            }
            return;
        }
        // 可见行两侧留一个屏的上下文，滚动时在边缘按需替换这一份结果。
        let top = f32::from(visible.top() - bounds.top());
        let bottom = f32::from(visible.bottom() - bounds.top());
        let h = (bottom - top).max(1.);
        let source_at = |x, y, fallback| id.from_byte + layout.source_at(x, y).unwrap_or(fallback) as u64;
        let from = source_at(0., top, 0);
        let to = source_at(f32::from(bounds.size.width), bottom, self.host.parsed.source.len());
        let visible = from.min(to)..from.max(to).saturating_add(1).min(id.to_byte);
        if marks.requested.start <= visible.start && visible.end <= marks.requested.end { return; }
        let start = if top <= h { id.from_byte } else { source_at(0., top-h, 0).min(visible.start) };
        let end = if bottom + h >= layout.height { id.to_byte }
            else { source_at(f32::from(bounds.size.width), bottom+h, self.host.parsed.source.len()).max(visible.end) };
        // 长源码行或隐藏标记可能跨过很大的字节区间，预取两端各至多 2 KiB。
        let start = start.max(visible.start.saturating_sub(2048));
        let end = end.min(visible.end.saturating_add(2048));
        let Some(port) = self.code_view_port.clone() else { return };
        marks.requested = start..end;
        marks.generation += 1;
        let generation = marks.generation;
        let task_id = marks.task_id.clone();
        let expected = task_id.clone();
        let task = cx.background_executor().spawn(async move {
            port.manager.search_region(&port.owner, &task_id, start, end)
        });
        marks.job = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                let Some(marks) = &mut view.find_marks else { return };
                if marks.task_id != expected || marks.generation != generation { return; }
                match result {
                    Ok((_, ranges)) => { marks.ranges = ranges; cx.notify(); }
                    Err(e) => {
                        marks.ranges.clear();
                        if !matches!(e.code, "search-expired" | "task-cancelled" | "revision-conflict" | "document-closed") {
                            cx.emit(ControlEvent::Failure(e.to_string()));
                        }
                        cx.notify();
                    }
                }
            });
        }));
    }
}
