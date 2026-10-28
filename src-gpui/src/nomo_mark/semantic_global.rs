//! 跨窗口语义选区：逐组解析，记录格式标记补丁，不拼接或持有完整正文。
use super::{
    mapping::text_segments,
    native_port::NativeWindowPort,
    parse::{InlineFormat, ParsedWindow},
    protocol::{ControlError, SourceEdit},
    session_window::WindowRequest,
};
use pulldown_cmark::Event;
use std::ops::Range;

const MAX_EDITS: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;

struct Span {
    open: Range<u64>,
    content: Range<u64>,
    close: Range<u64>,
    open_text: String,
    close_text: String,
}

pub(super) struct SemanticRewrite {
    pub from: u64,
    pub to: u64,
    pub close_before: String,
    pub open_after: String,
    pub edits: Vec<SourceEdit>,
}

fn push_edit(edits: &mut Vec<SourceEdit>, budget: &mut usize, edit: SourceEdit) -> Result<(), ControlError> {
    *budget = budget.saturating_add(edit.inserted_text.len() + std::mem::size_of::<SourceEdit>());
    if edits.len() >= MAX_EDITS || *budget > MAX_BYTES {
        return Err(ControlError::NeedsContext("语义选区的转换补丁超过处理预算".into()));
    }
    edits.push(edit);
    Ok(())
}

fn text_edits(parsed: &ParsedWindow, selection: &Range<u64>, edits: &mut Vec<SourceEdit>, budget: &mut usize) -> Result<(), ControlError> {
    let base = parsed.identity.from_byte;
    for (event, source) in &parsed.events {
        let global = base + source.start as u64..base + source.end as u64;
        if global.end <= selection.start || selection.end <= global.start { continue; }
        let (text, source) = match event {
            Event::Text(text) => (text.as_ref(), source.clone()),
            Event::Code(text) => {
                let span = parsed.inline.iter().find(|span| span.format == InlineFormat::Code && span.open.start == source.start && span.close.end == source.end)
                    .ok_or_else(|| ControlError::NeedsContext("行内代码边界不完整".into()))?;
                (text.as_ref(), span.content.clone())
            }
            Event::InlineMath(text) => {
                if selection.start > global.start || selection.end < global.end {
                    return Err(ControlError::NeedsContext("行内公式的部分选区需要完整公式边界".into()));
                }
                push_edit(edits, budget, SourceEdit { from_byte: global.start, to_byte: global.end, inserted_text: text.to_string() })?;
                continue;
            }
            Event::InlineHtml(html) => {
                if !parsed.inline.iter().any(|span| span.open == *source || span.close == *source) {
                    if html.trim().eq_ignore_ascii_case("<br>") || html.trim().eq_ignore_ascii_case("<br />") {
                        push_edit(edits, budget, SourceEdit { from_byte: global.start, to_byte: global.end, inserted_text: super::commands::line_ending(&parsed.source, source.start).into() })?;
                    } else { return Err(ControlError::NeedsContext("选区包含不能安全拆分的 HTML".into())); }
                }
                continue;
            }
            Event::FootnoteReference(_) | Event::Rule | Event::DisplayMath(_) => {
                return Err(ControlError::NeedsContext("选区包含需要单独处理的结构对象".into()));
            }
            _ => continue,
        };
        let raw = parsed.source.get(source.clone()).ok_or(ControlError::InvalidRange)?;
        for segment in text_segments(text, raw, source.start) {
            let from = base + segment.source.start as u64;
            let to = base + segment.source.end as u64;
            if to <= selection.start || selection.end <= from || segment.exact { continue; }
            if selection.start > from || selection.end < to {
                return Err(ControlError::NeedsContext("选区截断了转义或实体字符".into()));
            }
            let shown = text.get(segment.display).ok_or(ControlError::InvalidRange)?;
            let eol = super::commands::line_ending(&parsed.source, segment.source.start);
            let replacement = if eol == "\r\n" { shown.replace("\r\n", "\n").replace('\n', "\r\n") } else { shown.to_string() };
            if parsed.source.get(segment.source.clone()) != Some(replacement.as_str()) {
                push_edit(edits, budget, SourceEdit { from_byte: from, to_byte: to, inserted_text: replacement })?;
            }
        }
    }
    Ok(())
}

pub(super) fn analyze(
    port: &NativeWindowPort,
    request: &WindowRequest,
    revision: u64,
    selection: Range<u64>,
) -> Result<SemanticRewrite, ControlError> {
    let mut cursor = selection.start;
    let mut spans = Vec::new();
    let mut edits = Vec::new();
    let mut budget = 0usize;
    while cursor < selection.end {
        request.check()?;
        let loaded = port.load_group(revision, cursor, None, request)?;
        let from = loaded.input.from_byte;
        let to = loaded.input.to_byte;
        if from > cursor || to <= cursor { return Err(ControlError::NeedsContext("语义窗口没有覆盖完整选区".into())); }
        let parsed = ParsedWindow::parse(loaded.input, "toolbar-semantic", 0, loaded.context)?;
        let local = selection.start.saturating_sub(from) as usize..(selection.end.min(to) - from) as usize;
        if !parsed.context.ancestors.is_empty() || parsed.nodes.iter().any(|node| {
            matches!(node.kind, "quote" | "list" | "item" | "table" | "html" | "image" | "footnote" | "heading" | "code")
                && node.source.start < local.end && local.start < node.source.end
        }) {
            return Err(ControlError::NeedsContext("跨窗口选区包含尚不能安全拆分的结构容器".into()));
        }
        text_edits(&parsed, &selection, &mut edits, &mut budget)?;
        for span in &parsed.inline {
            let global = |range: &Range<usize>| from + range.start as u64..from + range.end as u64;
            let open = global(&span.open);
            let content = global(&span.content);
            let close = global(&span.close);
            if open.start >= selection.end || close.end <= selection.start { continue; }
            for marker in [&open, &close] {
                if [selection.start, selection.end].iter().any(|at| marker.start < *at && *at < marker.end) {
                    return Err(ControlError::NeedsContext("选区截断了行内格式标记".into()));
                }
            }
            let open_text = parsed.source[span.open.clone()].to_string();
            let close_text = parsed.source[span.close.clone()].to_string();
            budget += open_text.len() + close_text.len() + std::mem::size_of::<Span>();
            if budget > MAX_BYTES || spans.len() >= MAX_EDITS { return Err(ControlError::NeedsContext("语义选区的格式边界超过处理预算".into())); }
            spans.push(Span { open, content, close, open_text, close_text });
        }
        if edits.len() > MAX_EDITS { return Err(ControlError::NeedsContext("语义选区的转换补丁超过处理预算".into())); }
        cursor = to.min(selection.end);
    }
    let (mut from, mut to) = (selection.start, selection.end);
    loop {
        let old = (from, to);
        for span in &spans {
            if span.open.start < from && from <= span.content.start && to > span.content.start { from = span.open.start; }
            if span.content.end <= to && to < span.close.end && from < span.content.end { to = span.close.end; }
        }
        if old == (from, to) { break; }
    }
    let mut close_before = String::new();
    for span in spans.iter().rev().filter(|span| span.content.start < from && from < span.content.end) { close_before.push_str(&span.close_text); }
    let mut open_after = String::new();
    for span in spans.iter().filter(|span| span.content.start < to && to < span.content.end) { open_after.push_str(&span.open_text); }
    for span in spans {
        for marker in [span.open, span.close] {
            if selection.start <= marker.start && marker.end <= selection.end {
                push_edit(&mut edits, &mut budget, SourceEdit { from_byte: marker.start, to_byte: marker.end, inserted_text: String::new() })?;
            }
        }
    }
    if edits.len() > MAX_EDITS || edits.iter().map(|edit| edit.inserted_text.len() + std::mem::size_of::<SourceEdit>()).sum::<usize>() > MAX_BYTES {
        return Err(ControlError::NeedsContext("语义选区的转换补丁超过处理预算".into()));
    }
    Ok(SemanticRewrite { from, to, close_before, open_after, edits })
}
