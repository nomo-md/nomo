//! 文档级键盘导航沿用会话请求、取消和窗口加载，不建立全文编辑镜像。
use super::*;
use crate::nomo_mark::{navigation::CursorUnit, session_window::WindowRequest};

impl DocumentHost {
    pub(super) fn keyboard_boundary(&mut self, unit: CursorUnit, forward: bool, extend: bool, delete: bool, cx: &mut Context<Self>) {
        if (delete && self.editing_locked()) || !self.ready_for_session_action(cx) { return; }
        let before = self.control.read(cx).host.selection;
        if (forward && before.head_byte >= self.total_bytes) || (!forward && before.head_byte == 0) { return; }
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let semantic = self.control.read(cx).effective_mode() == ViewMode::Semantic;
        // 语义布局耗尽可见落点后只能进入邻窗，逐字查询会倒退到本窗隐藏的 #、> 或列表标记里。
        let semantic_target = if semantic && !delete {
            let identity = &self.control.read(cx).host.parsed.identity;
            if forward {
                if identity.to_byte >= self.total_bytes { return; }
                Some(identity.to_byte)
            } else {
                if identity.from_byte == 0 { return; }
                Some(identity.from_byte - 1)
            }
        } else { None };
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let next = match semantic_target {
                Some(next) => next,
                None => port.cursor_boundary(revision, before.head_byte, unit, forward, &request)?,
            };
            let mut after = if delete { DocumentSelection::caret(before.head_byte.min(next)) }
                else { DocumentSelection { anchor_byte: if extend { before.anchor_byte } else { next }, head_byte: next } };
            let revision = if delete && next != before.head_byte {
                let selection = DocumentSelection { anchor_byte: before.head_byte, head_byte: next };
                let loaded = port.load_group(revision, selection.range().start, None, &request)?;
                let source_only = matches!(&loaded.context, WindowContext::Resolved(context) if context.source_only);
                let (edits,template_update) = if semantic && !source_only {
                    let parsed = crate::nomo_mark::parse::ParsedWindow::parse(loaded.input, "keyboard-delete", 0, loaded.context)?;
                    let edit = crate::nomo_mark::commands::apply_semantic(&parsed, selection, EditIntent::Backspace)?;
                    after = edit.selection;
                    (edit.edits,edit.template_update)
                } else {
                    (vec![SourceEdit { from_byte: selection.range().start, to_byte: selection.range().end, inserted_text: String::new() }],None)
                };
                request.check()?;
                let operation = format!("keyboard-{}-{}",port.document_id,std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
                port.submit(DocumentTransaction { base_revision: revision, operation_id: operation.clone(), history_group_id: operation,
                    edits, selection_before: before, selection_after: after, template_update })?
            } else { revision };
            Ok::<_,ControlError>((port.load_group(revision,after.head_byte,None,&request)?,after))
        });
        cx.spawn(async move |this,cx| {
            let result = task.await;
            let _ = this.update(cx,|host,cx| {
                host.navigating = false;
                match result.and_then(|(loaded,after)|host.install(loaded,Some(after),cx)) {
                    Ok(()) if semantic && !delete => host.control.update(cx, |view, cx| {
                        view.defer_navigation_caret(cx);
                        if !extend { view.pending_block_entry = Some(forward); }
                    }),
                    Ok(()) => {}
                    Err(error) => host.action_failed(error,cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn keyboard_page(&mut self, down: bool, extend: bool, page: bool, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) { return; }
        let Some((x,y,line_height)) = self.control.read(cx).page_origin() else { return; };
        let pane = &self.panes[self.active_pane];
        let identity = &self.control.read(cx).host.parsed.identity;
        let (byte, limit, fraction) = if page {
            let Ok(current) = pane.viewport.groups.binary_search_by_key(&identity.from_byte, |group| group.from) else {
                self.adjacent(down,extend,cx); return;
            };
            let distance = (f32::from(pane.scroll.bounds().size.height) - line_height).max(line_height);
            let target = (pane.viewport.heights.top(current) + y + if down { distance } else { -distance }).clamp(0.0,(pane.viewport.heights.total()-line_height).max(0.0));
            let index = pane.viewport.heights.at(target);
            let local_y = target - pane.viewport.heights.top(index);
            if index == current {
                self.control.update(cx,|view,cx| view.move_page_to(x,local_y,extend,cx));
                return;
            }
            let Some(group) = pane.viewport.groups.get(index) else { return; };
            let height = (pane.viewport.heights.top(index+1)-pane.viewport.heights.top(index)).max(1.0);
            (group.from, None, (local_y/height).clamp(0.0,1.0))
        } else {
            // 本片段的视觉行已耗尽。估算高度可能仍含尾部留白，不能用它再次选中当前片段。
            let edge = if down { identity.to_byte } else { identity.from_byte };
            let probe = if down {
                if edge >= self.total_bytes { return; }
                edge
            } else {
                let Some(previous) = edge.checked_sub(1) else { return; };
                previous
            };
            let index = pane.viewport.groups.partition_point(|group| group.to <= probe);
            let neighbor = pane.viewport.groups.get(index).filter(|g| !g.is_pending() && g.contains(probe)
                && if down { g.from == edge } else { g.to == edge });
            let (byte, limit) = neighbor.map_or((probe, None), |g| (g.from, Some(g.to)));
            (byte, limit, if down { 0.0 } else { 1.0 })
        };
        let current_start = identity.from_byte;
        let anchor = extend.then_some(self.control.read(cx).host.selection.anchor_byte);
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let request: WindowRequest = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let loaded = port.load_group(revision, byte, limit, &request)?;
            if !page && !down && loaded.input.to_byte > current_start {
                return port.load_group(revision, loaded.input.from_byte, Some(current_start), &request);
            }
            Ok(loaded)
        });
        cx.spawn(async move |this,cx| {
            let result = task.await;
            let _ = this.update(cx,|host,cx| {
                host.navigating = false;
                match result.and_then(|loaded|host.install(loaded,None,cx)) {
                    Ok(()) => host.control.update(cx,|view,cx| {
                        view.defer_page_cursor(x,fraction,anchor,cx);
                        if !extend && !page { view.pending_block_entry = Some(down); }
                    }),
                    Err(error) => host.action_failed(error,cx),
                }
                cx.notify();
            });
        }).detach();
    }
}
