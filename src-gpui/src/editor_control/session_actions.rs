//! 真实文件宿主的平台连接：后台读取、全局选区/剪贴板、历史、定位和回收门禁。
//! 控件只接收 LoadedWindow；此处是文档服务调用方，不给控件提供反向全文读取能力。
use super::*;
use crate::editor_control::session_window::{LoadedWindow, WindowRequest};
use std::path::PathBuf;

impl Drop for DocumentHost {
    fn drop(&mut self) {
        self.request.cancel();
        if !self.closed {
            // 意外释放保留恢复数据；正常关闭由 Workspace 显式 close。
            let _ = self.port.manager.detach(&self.port.owner, &self.port.document_id);
        }
    }
}
impl DocumentHost {
    pub(super) fn ready_for_session_action(&mut self, cx: &mut Context<Self>) -> bool {
        if self.hidden || self.closed || self.navigating || self.saving || self.close_prompt {
            return false;
        }
        if let Err(error) = self.control.read(cx).prepare_unmount() {
            self.message = error.to_string();
            cx.notify();
            return false;
        }
        true
    }
    pub(super) fn begin_session_action(&mut self, cx: &mut Context<Self>) -> WindowRequest {
        self.pause_viewport(cx);
        self.request.cancel();
        self.request = WindowRequest::default();
        self.navigating = true;
        self.control.update(cx, |v, _| v.host.set_readonly(true));
        self.message = "正在准备原版会话窗口…".into();
        cx.notify();
        self.request.clone()
    }
    pub(super) fn install(
        &mut self,
        loaded: LoadedWindow,
        selection: Option<DocumentSelection>,
        cx: &mut Context<Self>,
    ) -> Result<(), ControlError> {
        let generation = self.control.read(cx).host.parsed.identity.view_generation + 1;
        let at = loaded.input.from_byte;
        let selection = selection.map(|mut selection| {
            // 全局锚点可留在屏外；活动端必须在新窗口的真实 UTF-8 / CRLF 边界。
            let mut head = selection
                .head_byte
                .clamp(at, loaded.input.to_byte)
                .saturating_sub(at) as usize;
            while head > 0 && !loaded.input.text.is_char_boundary(head) {
                head -= 1;
            }
            if head > 0
                && loaded.input.text.as_bytes().get(head - 1) == Some(&b'\r')
                && loaded.input.text.as_bytes().get(head) == Some(&b'\n')
            {
                head -= 1;
            }
            if selection.anchor_byte == selection.head_byte {
                selection.anchor_byte = at + head as u64;
            }
            selection.head_byte = at + head as u64;
            selection
        });
        self.total_bytes = loaded.total_bytes;
        self.control.update(cx, |v, cx| {
            v.update_window(loaded.input, generation, loaded.context, cx)?;
            v.host.set_readonly(false);
            v.select(selection.unwrap_or(DocumentSelection::caret(at)), cx);
            Ok::<_, ControlError>(())
        })?;
        self.panes[self.active_pane].scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.focus_pending = true;
        self.panes[self.active_pane].reveal_selection = true;
        self.message = format!("原版窗口会话 · 索引代数 {}", loaded.index.index_generation);
        self.reset_viewport(cx);
        self.document_changed(cx);
        Ok(())
    }
    pub(super) fn action_failed(&mut self, error: ControlError, cx: &mut Context<Self>) {
        // 历史/全局编辑可能已提交、但后续片段尚不可提供。旧视图必须继续只读。
        let current = self.port.status().ok().map(|s| s.revision);
        self.control.update(cx, |v, _| {
            v.host
                .set_readonly(current != Some(v.host.parsed.identity.revision))
        });
        self.report(error, cx);
        self.schedule_viewport(16,cx);
    }
    pub(super) fn load_at(
        &mut self,
        byte: u64,
        selection: Option<DocumentSelection>,
        cx: &mut Context<Self>,
    ) {
        self.load_at_focused(byte, selection, true, cx);
    }
    pub(super) fn navigate_outline_source(&mut self, byte: u64, revision: u64, cx: &mut Context<Self>) {
        if self.status().revision != revision || !self.ready_for_session_action(cx) { return; }
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let loaded = port.load_group(revision, byte, None, &request)?;
            let local = usize::try_from(byte.saturating_sub(loaded.input.from_byte)).map_err(|_| ControlError::InvalidRange)?;
            let tail = loaded.input.text.get(local..).ok_or(ControlError::InvalidRange)?;
            let end = match tail.find(['\r', '\n']) {
                Some(end) => end,
                None if loaded.input.to_byte == loaded.total_bytes => tail.len(),
                None => return Err(ControlError::NeedsContext("标题行超出当前窗口，无法安全定位行末。".into())),
            };
            Ok::<_, ControlError>((loaded, byte + end as u64))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if host.status().revision != revision { host.action_failed(ControlError::Stale, cx); return; }
                match result.and_then(|(loaded, end)| host.install(loaded, Some(DocumentSelection::caret(end)), cx)) {
                    Ok(()) => {}, Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn load_at_focused(
        &mut self,
        byte: u64,
        selection: Option<DocumentSelection>,
        focus: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let revision = port.status()?.revision;
            port.load_group(revision, byte, None, &request)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                match result.and_then(|loaded| host.install(loaded, selection, cx)) {
                    Ok(()) => { host.focus_pending = focus; }
                    Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn adjacent(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let view = self.control.read(cx);
        let id = &view.host.parsed.identity;
        let byte = if forward {
            if id.to_byte >= self.total_bytes {
                return;
            }
            id.to_byte
        } else {
            if id.from_byte == 0 {
                return;
            }
            id.from_byte - 1
        };
        let selection = if extend {
            DocumentSelection {
                anchor_byte: view.host.selection.anchor_byte,
                head_byte: byte,
            }
        } else {
            DocumentSelection::caret(byte)
        };
        self.load_at(byte, Some(selection), cx);
    }
    pub(super) fn choose_file(&mut self, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        if self.port.status().map_or(true, |s| s.dirty) {
            self.message = "请先保存当前文档，再打开另一份文件。".into();
            cx.notify();
            return;
        }
        let response = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("打开 Markdown".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = response.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |host, cx| host.open_path(path, cx));
                }
            }
        })
        .detach();
    }
    pub(super) fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        if self.port.status().map_or(true, |s| s.dirty) {
            self.message = "当前文档尚未保存。".into();
            cx.notify();
            return;
        }
        let request = self.begin_session_action(cx);
        let task = cx.background_executor().spawn(async move {
            let port = NativeWindowPort::open(
                std::env::temp_dir().join("nomo-gpui-control-sessions"),
                "gpui-native-editor".into(),
                Some(path),
                None,
            )?;
            match port.load_group(port.status()?.revision, 0, None, &request) {
                Ok(loaded) => Ok((port, loaded)),
                Err(error) => {
                    let _ = port.close(false);
                    Err(error)
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                match result {
                    Ok((port, loaded)) => {
                        let from = loaded.input.from_byte;
                        let total = loaded.total_bytes;
                        let adapter = match WindowHostAdapter::new(
                            loaded.input,
                            "native-window",
                            0,
                            loaded.context,
                        ) {
                            Ok(adapter) => adapter,
                            Err(error) => {
                                let _ = port.close(false);
                                host.action_failed(error, cx);
                                cx.notify();
                                return;
                            }
                        };
                        // 新会话全部准备成功才释放原视图；失败时原文档仍可保存/操作。
                        match host.port.close(false) {
                            Ok(()) => {
                                let _ = host.control.update(cx, |v, cx| v.detach(false, cx));
                                host.total_bytes = total;
                                {
                                        host.port = port;
                                        let base_dir = host
                                            .port
                                            .status()
                                            .ok()
                                            .and_then(|s| s.path)
                                            .and_then(|p| {
                                                PathBuf::from(p).parent().map(|p| p.to_path_buf())
                                            });
                                        host.control = cx.new(|cx| {
                                            let mut v = EditorControl::new(adapter, cx);
                                            v.base_dir = base_dir;
                                            v.host.selection = DocumentSelection::caret(from);
                                            v
                                        });
                                        host.reset_viewport(cx);
                                        host.focus_pending = true;
                                        host.panes[host.active_pane].scroll.set_offset(gpui::point(px(0.0), px(0.0)));
                                        host.message = "真实文件已连接原版窗口会话".into();
                                }
                            }
                            Err(error) => {
                                let _ = port.close(false);
                                host.action_failed(error, cx);
                            }
                        }
                    }
                    Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn session_history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        let fallback = self.control.read(cx).host.selection.head_byte;
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let (revision, selection) = port.history(port.status()?.revision, redo)?;
            let byte = selection.map_or(fallback, |s| s.head_byte);
            let loaded = port.load_group(revision, byte, None, &request)?;
            Ok::<_, ControlError>((loaded, selection))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if let Err(error) =
                    result.and_then(|(loaded, selection)| host.install(loaded, selection, cx))
                {
                    host.action_failed(error, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn global_edit(
        &mut self,
        selection: DocumentSelection,
        intent: EditIntent,
        cx: &mut Context<Self>,
    ) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        if self.control.read(cx).mode == ViewMode::Semantic {
            match &intent {
                EditIntent::ToggleInline(format) => { self.format_global_selection(selection, Some(*format), cx); return; }
                EditIntent::ClearFormatting => { self.format_global_selection(selection, None, cx); return; }
                _ => {}
            }
        }
        if matches!(&intent, EditIntent::InsertCodeBlock(_) | EditIntent::InsertMathBlock | EditIntent::InsertDiagram(_)) {
            let semantic = self.control.read(cx).mode == ViewMode::Semantic;
            self.wrap_global_selection(selection, intent, semantic, cx);
            return;
        }
        let text = match intent {
            EditIntent::Replace(text) => text,
            EditIntent::Backspace | EditIntent::Delete if !selection.range().is_empty() => {
                String::new()
            }
            _ => {
                self.message = "此跨窗口结构命令需要宿主提供完整可编辑结构；选区已保留。".into();
                cx.notify();
                return;
            }
        };
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let range = selection.range();
        let after = DocumentSelection::caret(range.start + text.len() as u64);
        let operation = format!(
            "gpui-global-{}-{}",
            self.port.document_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let tx = DocumentTransaction {
            base_revision: revision,
            operation_id: operation.clone(),
            history_group_id: operation,
            edits: vec![SourceEdit {
                from_byte: range.start,
                to_byte: range.end,
                inserted_text: text,
            }],
            selection_before: selection,
            selection_after: after,
            template_update: None,
        };
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let revision = port.submit(tx)?;
            port.load_group(revision, after.head_byte, None, &request)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if let Err(error) = result.and_then(|loaded| host.install(loaded, Some(after), cx))
                {
                    host.action_failed(error, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn clipboard(
        &mut self,
        selection: DocumentSelection,
        cut: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let port = self.port.clone();
        self.begin_session_action(cx);
        let task = cx
            .background_executor()
            .spawn(async move { port.copy_range(revision, selection.range()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                host.control.update(cx, |v, _| v.host.set_readonly(false));
                match result {
                    Ok(text) => {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                        if cx.read_from_clipboard().and_then(|c| c.text()) != Some(text) {
                            host.message = "剪贴板写入未确认，正文已保留。".into();
                        } else if cut {
                            host.global_edit(selection, EditIntent::Replace(String::new()), cx);
                        } else {
                            host.message = "完整选区已复制".into();
                        }
                    }
                    Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn anchor(&mut self, anchor: String, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) {
            return;
        }
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let revision = port.status()?.revision;
            let byte = port.resolve_anchor(revision, &anchor, &request)?;
            Ok::<_, ControlError>((port.load_group(revision, byte, None, &request)?, byte))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if let Err(error) = result.and_then(|(loaded, byte)| {
                    host.install(loaded, Some(DocumentSelection::caret(byte)), cx)
                }) {
                    host.action_failed(error, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
