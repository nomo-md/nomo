//! 工具栏全文结构命令：后台分页读取既有索引，只提交局部事务，不复制完整正文。
use super::*;
use crate::nomo_mark::session_window::WindowRequest;
use crate::text_document::StructureQuery;

#[derive(Clone, Copy)]
enum Tool { Footnote, Toc }
use crate::nomo_mark::session_window::structure_error as error;

fn next_number(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    for byte in bytes.iter_mut().rev() {
        if *byte != b'9' { *byte += 1; return String::from_utf8(bytes).expect("ASCII digits"); }
        *byte = b'0';
    }
    format!("1{}", String::from_utf8(bytes).expect("ASCII digits"))
}
fn escape_title(text: &str) -> String {
    let mut result = String::new();
    for (at, ch) in text.char_indices() {
        let entity = ch == '&' && text[at + 1..].find(';').is_some_and(|end| {
            let body = &text[at + 1..at + 1 + end];
            !body.is_empty() && body.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '#')
        });
        if "\\`*_~^<>[]$".contains(ch) || entity { result.push('\\'); }
        result.push(ch);
    }
    result
}
fn make_edits(port: &NativeWindowPort, request: &WindowRequest, revision: u64, selection: DocumentSelection, tool: Tool, eol: &str) -> Result<(Vec<SourceEdit>, DocumentSelection), ControlError> {
    let status = port.status()?;
    if status.revision != revision { return Err(ControlError::Stale); }
    let length = status.byte_length;
    let range = selection.range();
    if range.end > length { return Err(ControlError::InvalidRange); }
    let index = if length == 0 { None } else { Some(port.ready_index(revision, request)?) };
    let mut cursor = None;
    match tool {
        Tool::Footnote => {
            let mut max = String::from("0");
            if let Some(index) = index {
                loop {
                    request.check()?;
                    let page = port.manager.query_structure(&port.owner, index.clone(), StructureQuery::Blocks { from_byte: 0, to_byte: length, cursor }).map_err(error)?;
                    if page.fragment.is_some() { return Err(ControlError::NeedsContext("脚注索引不完整；原文已保留".into())); }
                    for block in page.items {
                        if block.get("semanticComplete").and_then(|v| v.as_bool()) != Some(true) { return Err(ControlError::NeedsContext("文档结构尚未完整索引；原文已保留".into())); }
                        if let Some(notes) = block.get("footnotes").and_then(|value| value.as_array()) {
                            for note in notes {
                                let Some(label) = note.get("label").and_then(|value| value.as_str()) else { continue; };
                                if label.len() > 4096 { return Err(ControlError::NeedsContext("脚注编号超过预算；原文已保留".into())); }
                                if label.is_empty() || !label.bytes().all(|byte| byte.is_ascii_digit()) { continue; }
                                let label = label.trim_start_matches('0');
                                let label = if label.is_empty() { "0" } else { label };
                                if label.len() > max.len() || (label.len() == max.len() && label > max.as_str()) { max = label.to_string(); }
                            }
                        }
                    }
                    cursor = page.next;
                    if cursor.is_none() { break; }
                }
            }
            let id = next_number(&max);
            let reference = format!("[^{id}]");
            let trailing = if range.end == length { 0 } else {
                let tail = port.read_window(revision, length.saturating_sub(8)..length)?;
                tail.text.chars().rev().take_while(|ch| *ch == '\n' || *ch == '\r').filter(|ch| *ch == '\n').count().min(2)
            };
            let footer = format!("{}[^{id}]: ", eol.repeat(2usize.saturating_sub(trailing)));
            let final_offset = length - (range.end - range.start) + reference.len() as u64 + footer.len() as u64;
            let edits = if range.start == length {
                vec![SourceEdit { from_byte: range.start, to_byte: range.end, inserted_text: format!("{reference}{footer}") }]
            } else {
                vec![SourceEdit { from_byte: range.start, to_byte: range.end, inserted_text: reference }, SourceEdit { from_byte: length, to_byte: length, inserted_text: footer }]
            };
            Ok((edits, DocumentSelection::caret(final_offset)))
        }
        Tool::Toc => {
            let mut contents = String::new();
            if let Some(index) = index {
                loop {
                    request.check()?;
                    let page = port.manager.query_structure(&port.owner, index.clone(), StructureQuery::Outline { cursor }).map_err(error)?;
                    if page.fragment.is_some() { return Err(ControlError::NeedsContext("标题索引不完整；原文已保留".into())); }
                    for heading in page.items {
                        let title = heading.get("title").and_then(|value| value.as_str()).ok_or(ControlError::InvalidRange)?;
                        let anchor = heading.get("id").and_then(|value| value.as_str()).ok_or(ControlError::InvalidRange)?;
                        let level = heading.get("level").and_then(|value| value.as_u64()).unwrap_or(1).clamp(1, 6) as usize;
                        if !contents.is_empty() { contents.push_str(eol); }
                        contents.push_str(&format!("{}- [{}](#{anchor})", "  ".repeat(level - 1), escape_title(title)));
                        if contents.len() > 4 * 1024 * 1024 { return Err(ControlError::NeedsContext("目录内容超过 4 MiB 预算；原文已保留".into())); }
                    }
                    cursor = page.next;
                    if cursor.is_none() { break; }
                }
            }
            let before = if range.start == 0 { String::new() } else {
                let window = port.read_window(revision, range.start.saturating_sub(8)..range.start)?;
                let breaks = window.text.chars().rev().take_while(|ch| *ch == '\n' || *ch == '\r').filter(|ch| *ch == '\n').count().min(2);
                eol.repeat(2usize.saturating_sub(breaks))
            };
            let after = if range.end == length { String::new() } else {
                let window = port.read_window(revision, range.end..(range.end + 8).min(length))?;
                let breaks = window.text.chars().take_while(|ch| *ch == '\n' || *ch == '\r').filter(|ch| *ch == '\n').count().min(2);
                eol.repeat(2usize.saturating_sub(breaks))
            };
            let middle = if contents.is_empty() { String::new() } else { format!("{contents}{eol}") };
            let inserted = format!("{before}<!-- toc -->{eol}{middle}<!-- /toc -->{after}");
            let caret = range.start + inserted.len() as u64;
            Ok((vec![SourceEdit { from_byte: range.start, to_byte: range.end, inserted_text: inserted }], DocumentSelection::caret(caret)))
        }
    }
}
impl DocumentHost {
    pub(crate) fn insert_footnote(&mut self, cx: &mut Context<Self>) { self.start_toolbar_document_tool(Tool::Footnote, cx); }
    pub(crate) fn insert_toc(&mut self, cx: &mut Context<Self>) { self.start_toolbar_document_tool(Tool::Toc, cx); }
    fn start_toolbar_document_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if !self.ready_for_session_action(cx) { return; }
        let view = self.control.read(cx);
        let selection = view.host.selection;
        let parsed = &view.host.parsed;
        let range = match crate::nomo_mark::mapping::local_range(&parsed.source, parsed.identity.from_byte, selection.range()) {
            Ok(range) => range,
            Err(_) => { self.report(ControlError::NeedsContext("跨窗口选区的结构插入尚不可用；原文已保留".into()), cx); return; }
        };
        if parsed.nodes.iter().any(|node| matches!(node.kind, "code" | "table" | "html") && node.source.contains(&range.start)) {
            self.report(ControlError::NeedsContext("当前容器内的结构插入尚不可用；原文已保留".into()), cx); return;
        }
        let eol = crate::nomo_mark::commands::line_ending(&parsed.source, range.start).to_string();
        let revision = parsed.identity.revision;
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let (edits, after) = make_edits(&port, &request, revision, selection, tool, &eol)?;
            request.check()?;
            let operation = format!("toolbar-{}-{}", port.document_id, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            let tx = DocumentTransaction { base_revision: revision, operation_id: operation.clone(), history_group_id: operation, edits, selection_before: selection, selection_after: after, template_update: None };
            let revision = port.submit(tx)?;
            let loaded = port.load_group(revision, after.head_byte, None, &request)?;
            Ok::<_, ControlError>((loaded, after))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                if let Err(error) = result.and_then(|(loaded, selection)| host.install(loaded, Some(selection), cx)) { host.action_failed(error, cx); }
                cx.notify();
            });
        }).detach();
    }
}
