//! 文档级键盘导航沿用会话请求、取消和窗口加载，不建立全文编辑镜像。
use super::*;
use crate::nomo_mark::{navigation::CursorUnit, session_window::WindowRequest};

impl DocumentHost {
    pub(super) fn keyboard_boundary(&mut self, unit: CursorUnit, forward: bool, extend: bool, delete: bool, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) { return; }
        let before = self.control.read(cx).host.selection;
        if (forward && before.head_byte >= self.total_bytes) || (!forward && before.head_byte == 0) { return; }
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let semantic = self.control.read(cx).effective_mode() == ViewMode::Semantic;
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let next = port.cursor_boundary(revision, before.head_byte, unit, forward, &request)?;
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
                if let Err(error) = result.and_then(|(loaded,after)|host.install(loaded,Some(after),cx)) { host.action_failed(error,cx); }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn keyboard_page(&mut self, down: bool, extend: bool, page: bool, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) { return; }
        let Some((x,y,line_height)) = self.control.read(cx).page_origin() else { return; };
        let pane = &self.panes[self.active_pane];
        let identity = &self.control.read(cx).host.parsed.identity;
        let Some(current) = pane.viewport.groups.iter().position(|group| group.from == identity.from_byte && group.to == identity.to_byte) else {
            self.adjacent(down,extend,cx); return;
        };
        let distance = if page { (f32::from(pane.scroll.bounds().size.height) - line_height).max(line_height) } else { line_height };
        let target = (pane.viewport.heights.top(current) + y + if down { distance } else { -distance }).clamp(0.0,(pane.viewport.heights.total()-line_height).max(0.0));
        let index = pane.viewport.heights.at(target);
        let local_y = target - pane.viewport.heights.top(index);
        if index == current { self.control.update(cx,|view,cx|view.move_page_to(x,local_y,extend,cx)); return; }
        let Some(group) = pane.viewport.groups.get(index) else { return; };
        let byte = group.from;
        let group_height = (pane.viewport.heights.top(index+1)-pane.viewport.heights.top(index)).max(1.0);
        let fraction = (local_y/group_height).clamp(0.0,1.0);
        let anchor = extend.then_some(self.control.read(cx).host.selection.anchor_byte);
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let request: WindowRequest = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move { port.load_group(revision,byte,None,&request) });
        cx.spawn(async move |this,cx| {
            let result = task.await;
            let _ = this.update(cx,|host,cx| {
                host.navigating = false;
                match result.and_then(|loaded|host.install(loaded,None,cx)) {
                    Ok(()) => host.control.update(cx,|view,cx|view.defer_page_cursor(x,fraction,anchor,cx)),
                    Err(error) => host.action_failed(error,cx),
                }
                cx.notify();
            });
        }).detach();
    }
}
