//! 跨窗口块包裹：分片扫描围栏，语义格式只记录局部标记补丁，不物化选区正文。
use super::*;
use crate::nomo_mark::session_window::WindowRequest;

fn normalize_edits(mut edits: Vec<SourceEdit>) -> Result<Vec<SourceEdit>, ControlError> {
    edits.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
    let mut result: Vec<SourceEdit> = Vec::new();
    for edit in edits {
        if let Some(previous) = result.last_mut() {
            if previous.from_byte == edit.from_byte && previous.to_byte == edit.to_byte && previous.inserted_text.is_empty() && edit.inserted_text.is_empty() { continue; }
            if previous.from_byte == edit.from_byte && previous.from_byte == previous.to_byte {
                previous.to_byte = edit.to_byte;
                previous.inserted_text.push_str(&edit.inserted_text);
                continue;
            }
            if edit.from_byte < previous.to_byte || edit.from_byte == previous.from_byte { return Err(ControlError::InvalidRange); }
        }
        result.push(edit);
    }
    Ok(result)
}

fn ends_with_newline(port: &NativeWindowPort, revision: u64, range: &std::ops::Range<u64>, edits: &[SourceEdit]) -> Result<bool, ControlError> {
    let mut cursor = range.end;
    let mut ordered: Vec<_> = edits.iter().collect();
    ordered.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
    for edit in ordered.into_iter().rev() {
        if edit.to_byte < cursor { break; }
        if edit.to_byte == cursor {
            if !edit.inserted_text.is_empty() { return Ok(edit.inserted_text.ends_with('\n')); }
            cursor = edit.from_byte;
        }
    }
    if cursor <= range.start { return Ok(false); }
    Ok(port.read_window(revision, cursor.saturating_sub(8).max(range.start)..cursor)?.text.ends_with('\n'))
}

struct FenceScan { run: usize, longest: usize, previous_dollar: bool, math: bool }
impl FenceScan {
    fn add(&mut self, bytes: &[u8]) -> Result<(), ControlError> {
        for byte in bytes {
            if *byte == b'`' { self.run += 1; self.longest = self.longest.max(self.run); } else { self.run = 0; }
            if self.longest >= 4096 { return Err(ControlError::NeedsContext("代码围栏超过预算；选区原文已保留".into())); }
            if self.math && *byte == b'$' && self.previous_dollar { return Err(ControlError::Unsupported("选区包含公式块围栏；原文已保留".into())); }
            self.previous_dollar = *byte == b'$';
        }
        Ok(())
    }
}

fn scan_source(port: &NativeWindowPort, request: &WindowRequest, revision: u64, range: std::ops::Range<u64>, scan: &mut FenceScan) -> Result<(), ControlError> {
    let mut cursor = range.start;
    while cursor < range.end {
        request.check()?;
        let end = (cursor + 64 * 1024).min(range.end);
        let window = port.read_window(revision, cursor..end)?;
        let begin = (cursor - window.from_byte) as usize;
        let stop = (end - window.from_byte) as usize;
        scan.add(&window.text.as_bytes()[begin..stop])?;
        cursor = end;
    }
    Ok(())
}

fn for_each_semantic_group(
    port: &NativeWindowPort,
    request: &WindowRequest,
    revision: u64,
    range: std::ops::Range<u64>,
    mut visit: impl FnMut(&crate::nomo_mark::parse::ParsedWindow, DocumentSelection) -> Result<(), ControlError>,
) -> Result<(), ControlError> {
    let mut cursor = range.start;
    while cursor < range.end {
        request.check()?;
        let loaded = port.load_group(revision, cursor, None, request)?;
        let from = loaded.input.from_byte;
        let to = loaded.input.to_byte;
        if from > cursor || to <= cursor { return Err(ControlError::NeedsContext("语义窗口没有覆盖完整选区".into())); }
        let parsed = crate::nomo_mark::parse::ParsedWindow::parse(loaded.input, "toolbar-format", 0, loaded.context)?;
        let local = range.start.saturating_sub(from) as usize..(range.end.min(to) - from) as usize;
        if !parsed.context.ancestors.is_empty() || parsed.nodes.iter().any(|node| {
            matches!(node.kind, "html" | "image" | "footnote" | "code")
                && node.source.start < local.end && local.start < node.source.end
        }) {
            return Err(ControlError::NeedsContext("跨窗口选区包含尚不能安全格式化的结构容器".into()));
        }
        visit(&parsed, DocumentSelection { anchor_byte: cursor, head_byte: to.min(range.end) })?;
        cursor = to.min(range.end);
    }
    Ok(())
}

fn format_selection(port: &NativeWindowPort, request: &WindowRequest, revision: u64, selection: DocumentSelection, format: Option<InlineFormat>) -> Result<(Vec<SourceEdit>, DocumentSelection), ControlError> {
    let range = selection.range();
    if range.is_empty() || range.end > port.status()?.byte_length { return Err(ControlError::InvalidRange); }
    if range.end - range.start > 4 * 1024 * 1024 { return Err(ControlError::NeedsContext("跨窗口选区超过 4 MiB 处理预算".into())); }
    if format == Some(InlineFormat::Code) { return Err(ControlError::NeedsContext("跨窗口行内代码格式需要完整代码边界".into())); }
    let mut remove = true;
    let mut content = false;
    if let Some(format) = format {
        for_each_semantic_group(port, request, revision, range.clone(), |parsed, part| {
            if let Some(covered) = crate::nomo_mark::inline_commands::selection_formatted(parsed, part, format)? {
                remove &= covered;
                content = true;
            }
            Ok(())
        })?;
        if !content { return Ok((Vec::new(), selection)); }
    }
    let mut edits = Vec::new();
    let mut bytes = 0usize;
    for_each_semantic_group(port, request, revision, range.clone(), |parsed, part| {
        let result = crate::nomo_mark::inline_commands::selected_with_toggle(parsed, part, format, format.map(|_| remove))?;
        for node in parsed.nodes.iter().filter(|node| node.parent.is_none() && matches!(node.kind, "paragraph" | "heading" | "quote" | "list" | "table")) {
            let from = parsed.identity.from_byte + node.source.start as u64;
            let to = parsed.identity.from_byte + node.source.end as u64;
            let growth = result.edits.iter().filter(|edit| edit.from_byte >= from && edit.to_byte <= to)
                .map(|edit| edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128).sum::<i128>();
            if (to - from) as i128 + growth > crate::text_document::MAX_SEMANTIC_BYTES as i128 {
                return Err(ControlError::NeedsContext("格式化后的单个结构块超过语义窗口预算".into()));
            }
        }
        for edit in result.edits {
            bytes = bytes.saturating_add(edit.inserted_text.len() + std::mem::size_of::<SourceEdit>());
            if edits.len() >= 4096 || bytes > 4 * 1024 * 1024 { return Err(ControlError::NeedsContext("格式补丁超过跨窗口处理预算".into())); }
            edits.push(edit);
        }
        Ok(())
    })?;
    let edits = normalize_edits(edits)?;
    let mut after = crate::nomo_mark::commands::map_selection(selection, &edits);
    let closing = edits.iter().find(|edit| edit.from_byte == range.end && edit.to_byte == range.end).map_or(0, |edit| edit.inserted_text.len() as u64);
    if selection.anchor_byte <= selection.head_byte { after.head_byte = after.head_byte.saturating_sub(closing); }
    else { after.anchor_byte = after.anchor_byte.saturating_sub(closing); }
    Ok((edits, after))
}

fn wrap(port: &NativeWindowPort, request: &WindowRequest, revision: u64, selection: DocumentSelection, intent: EditIntent, semantic: bool) -> Result<(Vec<SourceEdit>, DocumentSelection), ControlError> {
    let status = port.status()?;
    if status.revision != revision { return Err(ControlError::Stale); }
    let range = selection.range();
    if range.is_empty() || range.end > status.byte_length { return Err(ControlError::InvalidRange); }
    let source_bytes = range.end - range.start;
    if (!semantic && source_bytes > crate::text_document::MAX_SEMANTIC_BYTES as u64)
        || (semantic && source_bytes > 4 * 1024 * 1024)
    {
        return Err(ControlError::NeedsContext("选区超过块转换预算；原文已保留".into()));
    }
    let (language, math) = match intent {
        EditIntent::InsertCodeBlock(language) => (Some(language), false),
        EditIntent::InsertDiagram(_) => (Some("mermaid".to_string()), false),
        EditIntent::InsertMathBlock => (None, true),
        _ => return Err(ControlError::InvalidRange),
    };
    let (from, to, close_before, open_after, mut edits) = if semantic {
        let rewrite = crate::nomo_mark::semantic_global::analyze(port, request, revision, range.clone())?;
        (rewrite.from, rewrite.to, rewrite.close_before, rewrite.open_after, rewrite.edits)
    } else {
        (range.start, range.end, String::new(), String::new(), Vec::new())
    };
    edits = normalize_edits(edits)?;
    let delta = edits.iter().map(|edit| edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128).sum::<i128>();
    let body_len = u64::try_from((range.end - range.start) as i128 + delta).map_err(|_| ControlError::InvalidRange)?;
    if body_len > crate::text_document::MAX_SEMANTIC_BYTES as u64 {
        return Err(ControlError::NeedsContext("转换后的块超过语义窗口预算；选区原文已保留".into()));
    }
    let mut scan = FenceScan { run: 0, longest: 0, previous_dollar: false, math };
    let mut cursor = range.start;
    for edit in &edits {
        if edit.from_byte < cursor || edit.to_byte > range.end { return Err(ControlError::InvalidRange); }
        scan_source(port, request, revision, cursor..edit.from_byte, &mut scan)?;
        scan.add(edit.inserted_text.as_bytes())?;
        cursor = edit.to_byte;
    }
    scan_source(port, request, revision, cursor..range.end, &mut scan)?;
    let sample = port.read_window(revision, range.start.saturating_sub(4096)..(range.start + 4096).min(status.byte_length))?;
    let bytes = sample.text.as_bytes();
    let eol = bytes.iter().position(|byte| *byte == b'\n').map_or("\n", |at| if at > 0 && bytes[at-1] == b'\r' { "\r\n" } else { "\n" });
    let left = port.read_window(revision, from.saturating_sub(8)..from)?;
    let right = port.read_window(revision, to..(to + 8).min(status.byte_length))?;
    let before_breaks = if close_before.is_empty() { left.text.chars().rev().take_while(|ch| *ch == '\r' || *ch == '\n').filter(|ch| *ch == '\n').count().min(2) } else { 0 };
    let after_breaks = if open_after.is_empty() { right.text.chars().take_while(|ch| *ch == '\r' || *ch == '\n').filter(|ch| *ch == '\n').count().min(2) } else { 0 };
    let before = if from == 0 { String::new() } else { eol.repeat(2usize.saturating_sub(before_breaks)) };
    let after = if to == status.byte_length { String::new() } else { eol.repeat(2usize.saturating_sub(after_breaks)) };
    let fence = if math { "$$".to_string() } else { "`".repeat((scan.longest + 1).max(3)) };
    let language = language.filter(|value| value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_+#.-".contains(&byte))).unwrap_or_default();
    let block_bytes = body_len.saturating_add((fence.len() * 2 + language.len() + eol.len() * 2) as u64);
    if block_bytes > crate::text_document::MAX_SEMANTIC_BYTES as u64 {
        return Err(ControlError::NeedsContext("转换后的块超过语义窗口预算；选区原文已保留".into()));
    }
    let opening = format!("{close_before}{before}{fence}{language}{eol}");
    let closing = format!("{}{fence}{after}{open_after}", if ends_with_newline(port, revision, &range, &edits)? { "" } else { eol });
    let content_start = from + opening.len() as u64;
    let content_end = content_start + body_len;
    let after_selection = if selection.anchor_byte <= selection.head_byte {
        DocumentSelection { anchor_byte: content_start, head_byte: content_end }
    } else { DocumentSelection { anchor_byte: content_end, head_byte: content_start } };
    edits.push(SourceEdit { from_byte: from, to_byte: range.start, inserted_text: opening });
    edits.push(SourceEdit { from_byte: range.end, to_byte: to, inserted_text: closing });
    Ok((normalize_edits(edits)?, after_selection))
}
impl DocumentHost {
    pub(super) fn format_global_selection(&mut self, selection: DocumentSelection, format: Option<InlineFormat>, cx: &mut Context<Self>) {
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let (edits, after) = format_selection(&port, &request, revision, selection, format)?;
            request.check()?;
            if edits.is_empty() { return Ok(None); }
            let operation = format!("global-format-{}-{}", port.document_id, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            let tx = DocumentTransaction { base_revision: revision, operation_id: operation.clone(), history_group_id: operation, edits, selection_before: selection, selection_after: after, template_update: None };
            let revision = port.submit(tx)?;
            let loaded = port.load_group(revision, after.head_byte, None, &request)?;
            Ok::<_, ControlError>(Some((loaded, after)))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |host, cx| {
                host.navigating = false;
                match result {
                    Ok(Some((loaded, selection))) => if let Err(error) = host.install(loaded, Some(selection), cx) { host.action_failed(error, cx); },
                    Ok(None) => { host.control.update(cx, |view, _| view.host.set_readonly(false)); host.schedule_viewport(16, cx); },
                    Err(error) => host.action_failed(error, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn wrap_global_selection(&mut self, selection: DocumentSelection, intent: EditIntent, semantic: bool, cx: &mut Context<Self>) {
        let revision = self.control.read(cx).host.parsed.identity.revision;
        let request = self.begin_session_action(cx);
        let port = self.port.clone();
        let task = cx.background_executor().spawn(async move {
            let (edits, after) = wrap(&port, &request, revision, selection, intent, semantic)?;
            request.check()?;
            let operation = format!("global-block-{}-{}", port.document_id, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
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
