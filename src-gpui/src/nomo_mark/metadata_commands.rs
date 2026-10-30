//! 文档属性菜单按需加载文首，新增模板和后续编辑都沿用主会话历史。
use super::*;
use crate::nomo_mark::{nomo_syntax, session_window::{LoadedWindow, WindowRequest}};

fn prepare_front_matter(
    port: &NativeWindowPort,
    revision: u64,
    selection: DocumentSelection,
    request: &WindowRequest,
) -> Result<(LoadedWindow, bool), ControlError> {
    let loaded = port.load_group(revision, 0, None, request)?;
    if loaded.input.from_byte != 0 { return Err(ControlError::InvalidRange); }
    if nomo_syntax::front_matter_end(&loaded.input.text).is_some() {
        return Ok((loaded, false));
    }
    // 截断的属性块不能被当成“没有属性”，否则会在原属性前重复插入模板。
    if loaded.input.text.starts_with("---")
        && !matches!(&loaded.context, WindowContext::Complete | WindowContext::Resolved(ParseContext { structural_boundary: true, source_only: false, .. }))
    {
        return Err(ControlError::NeedsContext("文档属性超出当前语义窗口或边界尚不完整，请在源码模式编辑；原文已保留".into()));
    }
    let eol = if loaded.input.text.contains("\r\n") { "\r\n" } else { "\n" };
    // 复用已链接的 SQLite 本地日期函数；连接只存活于本次显式插入，不落盘。
    let date: String = rusqlite::Connection::open_in_memory()
        .and_then(|connection| connection.query_row("SELECT date('now', 'localtime')", [], |row| row.get(0)))
        .map_err(|error| ControlError::HostFailure(format!("无法生成文档属性日期：{error}")))?;
    let mut template = ["---", "title: 文档标题", &format!("created: {date}"), &format!("updated: {date}"),
        "tags:", "  - 笔记", "  - Markdown", "status: draft", "---", ""].join(eol);
    if loaded.total_bytes > 0 { template.push_str(eol); }
    request.check()?;
    let operation = format!("front-matter-{}-{}", port.document_id,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let revision = port.submit(DocumentTransaction {
        base_revision: revision, operation_id: operation.clone(), history_group_id: operation,
        edits: vec![SourceEdit { from_byte: 0, to_byte: 0, inserted_text: template }],
        selection_before: selection, selection_after: DocumentSelection::caret(3 + eol.len() as u64),
        template_update: None,
    })?;
    Ok((port.load_group(revision, 0, None, request)?, true))
}

impl DocumentHost {
    pub(crate) fn edit_front_matter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ControlError> {
        self.ready(cx)?;
        if self.hidden { return Err(ControlError::Busy); }
        let revision = self.status().revision;
        let selection = self.selection(cx);
        let request = self.begin_session_action(cx);
        let worker = request.clone();
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            prepare_front_matter(&port, revision, selection, &worker)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |host, _, cx| {
                if request.is_cancelled() || host.closed { return; }
                host.navigating = false;
                let result = result.and_then(|(loaded, inserted)| {
                    if host.hidden || host.port.status()?.revision != loaded.input.revision { return Err(ControlError::Stale); }
                    host.install(loaded, Some(DocumentSelection::caret(0)), cx)?;
                    // 新窗口先完成真实布局，再进入对象编辑；不能在 Preparing 状态强行打开草稿。
                    host.metadata.front_matter_entry = Some((host.control.read(cx).host.parsed.identity.clone(), inserted));
                    Ok(())
                });
                if let Err(error) = result { host.action_failed(error, cx); }
                cx.notify();
            });
        }).detach();
        Ok(())
    }

    pub(super) fn open_pending_front_matter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((identity, _)) = &self.metadata.front_matter_entry else { return; };
        let view = self.control.read(cx);
        if self.hidden || self.closed || *identity != view.host.parsed.identity || view.host.selection != DocumentSelection::caret(0) {
            self.metadata.front_matter_entry = None;
            return;
        }
        if self.navigating || view.host.pending_count() > 0 || view.host.presentation != Presentation::Editable { return; }
        let (_, inserted) = self.metadata.front_matter_entry.take().unwrap();
        match self.control.update(cx, |view, cx| view.begin_front_matter(inserted, window, cx)) {
            Ok(()) => self.focus_pending = false,
            Err(error) => self.report(error, cx),
        }
    }
}
