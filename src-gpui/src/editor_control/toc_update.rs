//! 自动目录在后台准备补丁，通过控件事务队列提交，避免与乐观输入竞争。
use super::*;
use crate::editor_control::session_window::WindowRequest;
use std::time::Duration;

#[derive(Default)]
pub(super) struct TocState {
    request: WindowRequest,
    task: Option<gpui::Task<()>>,
    serial: u64,
    checked_revision: Option<u64>,
    for_save: bool,
    deferred_selection: bool,
}

impl Drop for TocState {
    fn drop(&mut self) { self.request.cancel(); }
}

impl DocumentHost {
    pub(super) fn cancel_toc_when_hidden(&mut self) {
        if !self.queries.toc.for_save {
            self.cancel_toc();
        }
    }

    pub(super) fn cancel_toc(&mut self) {
        self.queries.toc.request.cancel();
        self.queries.toc.task = None;
        self.queries.toc.serial = self.queries.toc.serial.wrapping_add(1);
        self.queries.toc.for_save = false;
    }

    pub(super) fn toc_selection_changed(&mut self, cx: &mut Context<Self>) {
        if self.queries.toc.deferred_selection {
            self.queries.toc.checked_revision = None;
            self.schedule_toc(300, false, cx);
        }
    }

    /// 保存调用方先等待当前已确认修订的目录检查；失败时仍允许保存原文并提示。
    pub(crate) fn toc_ready_for_save(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.production || (self.queries.toc.checked_revision == Some(self.status().revision) && self.ready(cx).is_ok()) {
            return true;
        }
        if self.queries.toc.for_save { return false; }
        self.queries.toc.checked_revision = None;
        self.schedule_toc(0, true, cx);
        false
    }

    pub(super) fn schedule_toc(&mut self, delay_ms: u64, for_save: bool, cx: &mut Context<Self>) {
        if !self.production || self.closed { return; }
        let revision = self.status().revision;
        if self.queries.toc.checked_revision == Some(revision) { return; }
        self.queries.toc.for_save |= for_save;
        if self.hidden && !self.queries.toc.for_save { return; }
        self.queries.toc.request.cancel();
        self.queries.toc.request = WindowRequest::default();
        self.queries.toc.serial = self.queries.toc.serial.wrapping_add(1);
        let serial = self.queries.toc.serial;
        let request = self.queries.toc.request.clone();
        let port = self.port.clone();
        self.queries.toc.task = Some(cx.spawn(async move |this, cx| {
            if delay_ms > 0 { cx.background_executor().timer(Duration::from_millis(delay_ms)).await; }
            let can_prepare = this.update(cx, |host, cx| {
                if host.closed || host.queries.toc.serial != serial || request.is_cancelled() { return false; }
                if host.status().revision != revision || host.ready(cx).is_err()
                    || host.control.read(cx).host.parsed.identity.revision != revision
                {
                    host.schedule_toc(200, false, cx);
                    return false;
                }
                true
            }).unwrap_or(false);
            if !can_prepare { return; }
            let request_for_work = request.clone();
            let result = cx.background_executor().spawn(async move {
                request_for_work.check()?;
                if revision == 0 { return Ok(Vec::new()); }
                let identity = port.ready_index(revision, &request_for_work)?;
                let edits = port.manager.prepare_structure_toc(&port.owner, identity, 4 * 1024 * 1024, || request_for_work.is_cancelled())
                    .map_err(|error| ControlError::HostFailure(error.to_string()))?;
                request_for_work.check()?;
                Ok::<_, ControlError>(edits.into_iter().map(|edit| SourceEdit {
                    from_byte: edit.from_byte, to_byte: edit.to_byte, inserted_text: edit.inserted_text,
                }).collect::<Vec<_>>())
            }).await;
            let _ = this.update(cx, |host, cx| {
                if host.closed || host.queries.toc.serial != serial || request.is_cancelled() { return; }
                if host.status().revision != revision {
                    host.schedule_toc(if host.queries.toc.for_save { 0 } else { 300 }, false, cx);
                    return;
                }
                let edits = match result {
                    Ok(edits) => edits,
                    Err(error) => { host.finish_toc_check(revision, Some(error.to_string()), cx); return; }
                };
                let identity = &host.control.read(cx).host.parsed.identity;
                if identity.revision != revision || host.ready(cx).is_err() {
                    host.schedule_toc(200, false, cx);
                    return;
                }
                if edits.is_empty() { host.finish_toc_check(revision, None, cx); return; }
                let selection = host.control.read(cx).host.selection.range();
                let touches_selection = edits.iter().any(|edit| if selection.is_empty() {
                    edit.from_byte <= selection.start && selection.start <= edit.to_byte
                } else { edit.from_byte < selection.end && selection.start < edit.to_byte });
                if touches_selection {
                    host.queries.toc.deferred_selection = true;
                    if host.queries.toc.for_save {
                        host.finish_toc_check(revision, Some("光标或选区位于目录内，已保留当前目录内容".into()), cx);
                    }
                    return;
                }
                match host.control.update(cx, |view, cx| view.queue_derived(edits, cx)) {
                    Ok(()) => { host.queries.toc.deferred_selection = false; }
                    Err(ControlError::Busy) => host.schedule_toc(200, false, cx),
                    Err(error) => host.finish_toc_check(revision, Some(error.to_string()), cx),
                }
            });
        }));
    }

    pub(super) fn finish_toc_check(&mut self, revision: u64, warning: Option<String>, cx: &mut Context<Self>) {
        if self.status().revision != revision { return; }
        if warning.is_none() { self.queries.toc.deferred_selection = false; }
        self.queries.toc.checked_revision = Some(revision);
        self.queries.toc.for_save = false;
        cx.emit(DocumentEvent::TocSettled(warning));
    }
}
