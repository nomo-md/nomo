//! 将局部语义选区转成块时，拆开跨边界的行内格式并保留两侧源码。
use super::{
    commands::{EditIntent, EditResult, line_ending},
    mapping::text_segments,
    parse::{InlineFormat, ParsedWindow},
    protocol::{ControlError, DocumentSelection, SourceEdit},
};
use pulldown_cmark::{Event, TagEnd};
use std::ops::Range;

fn selected_text(parsed: &ParsedWindow, selection: &Range<usize>, eol: &str) -> Result<String, ControlError> {
    let mut output = String::new();
    for (event, source) in &parsed.events {
        if source.end <= selection.start || selection.end <= source.start { continue; }
        let (text, source) = match event {
            Event::Text(text) => (text.as_ref(), source.clone()),
            Event::Code(text) => {
                let span = parsed.inline.iter().find(|span| span.format == InlineFormat::Code && span.open.start == source.start && span.close.end == source.end)
                    .ok_or_else(|| ControlError::NeedsContext("行内代码边界不完整".into()))?;
                (text.as_ref(), span.content.clone())
            }
            Event::InlineMath(text) => {
                if selection.start > source.start || selection.end < source.end {
                    return Err(ControlError::NeedsContext("行内公式的部分选区需要完整公式边界".into()));
                }
                output.push_str(text);
                continue;
            }
            Event::SoftBreak | Event::HardBreak => { output.push_str(eol); continue; }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) if source.end < selection.end => {
                if !output.is_empty() && !output.ends_with(eol) { output.push_str(eol); }
                continue;
            }
            Event::FootnoteReference(_) | Event::Rule | Event::DisplayMath(_) => {
                return Err(ControlError::NeedsContext("选区包含需要单独处理的结构对象".into()));
            }
            Event::InlineHtml(html) => {
                let known_marker = parsed.inline.iter().any(|span| span.open == *source || span.close == *source);
                if !known_marker {
                    if html.trim().eq_ignore_ascii_case("<br>") || html.trim().eq_ignore_ascii_case("<br />") {
                        output.push_str(eol);
                    } else {
                        return Err(ControlError::NeedsContext("选区包含不能安全拆分的 HTML".into()));
                    }
                }
                continue;
            }
            _ => continue,
        };
        if source.end <= selection.start || selection.end <= source.start { continue; }
        let raw = parsed.source.get(source.clone()).ok_or(ControlError::InvalidRange)?;
        for segment in text_segments(text, raw, source.start) {
            let start = selection.start.max(segment.source.start);
            let end = selection.end.min(segment.source.end);
            if start >= end { continue; }
            if !segment.exact && (start != segment.source.start || end != segment.source.end) {
                return Err(ControlError::NeedsContext("选区截断了转义或实体字符".into()));
            }
            let shown = if segment.exact {
                segment.display.start + start - segment.source.start..segment.display.start + end - segment.source.start
            } else { segment.display.clone() };
            output.push_str(text.get(shown).ok_or(ControlError::InvalidRange)?);
        }
    }
    if eol == "\r\n" { output = output.replace("\r\n", "\n").replace('\n', "\r\n"); }
    Ok(output)
}

pub(super) fn wrap_formatted(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    range: Range<usize>,
    intent: &EditIntent,
) -> Result<Option<EditResult>, ControlError> {
    if range.is_empty() { return Ok(None); }
    if parsed.nodes.iter().any(|node| node.kind == "code" && node.source.contains(&range.start))
        || parsed.events.iter().any(|(event, source)| matches!(event, Event::DisplayMath(_)) && source.contains(&range.start))
    {
        return Ok(None);
    }
    if !parsed.context.ancestors.is_empty() || parsed.nodes.iter().any(|node| {
        matches!(node.kind, "quote" | "list" | "item" | "table" | "html" | "image" | "footnote" | "heading" | "code")
            && node.source.start < range.end && range.start < node.source.end
    }) {
        return Err(ControlError::NeedsContext("此容器中的格式选区不能安全转换为块".into()));
    }
    for span in &parsed.inline {
        for marker in [&span.open, &span.close] {
            if [range.start, range.end].iter().any(|at| marker.start < *at && *at < marker.end) {
                return Err(ControlError::NeedsContext("选区截断了行内格式标记".into()));
            }
        }
    }
    let source = &*parsed.source;
    let eol = line_ending(source, range.start);
    let mut body = selected_text(parsed, &range, eol)?;
    if body.is_empty() && source[range.clone()].trim().is_empty() { body.push_str(&source[range.clone()]); }
    let mut replace = range.clone();
    loop {
        let old = replace.clone();
        for span in &parsed.inline {
            if span.open.start < replace.start && replace.start <= span.content.start && replace.end > span.content.start {
                replace.start = span.open.start;
            }
            if span.content.end <= replace.end && replace.end < span.close.end && replace.start < span.content.end {
                replace.end = span.close.end;
            }
        }
        if replace == old { break; }
    }
    let mut close_before = String::new();
    for span in parsed.inline.iter().rev().filter(|span| span.content.start < replace.start && replace.start < span.content.end) {
        close_before.push_str(&source[span.close.clone()]);
    }
    let mut open_after = String::new();
    for span in parsed.inline.iter().filter(|span| span.content.start < replace.end && replace.end < span.content.end) {
        open_after.push_str(&source[span.open.clone()]);
    }
    let before_source = &source[..replace.start];
    let after_source = &source[replace.end..];
    let before_breaks = if close_before.is_empty() { before_source[before_source.trim_end_matches(['\r', '\n']).len()..].bytes().filter(|byte| *byte == b'\n').count() } else { 0 };
    let after_breaks = if open_after.is_empty() { after_source[..after_source.len() - after_source.trim_start_matches(['\r', '\n']).len()].bytes().filter(|byte| *byte == b'\n').count() } else { 0 };
    let before = eol.repeat(if before_source.is_empty() { 0 } else { 2usize.saturating_sub(before_breaks) });
    let after = eol.repeat(if after_source.is_empty() { 0 } else { 2usize.saturating_sub(after_breaks) });
    let (opening, closing) = match intent {
        EditIntent::InsertCodeBlock(language) | EditIntent::InsertDiagram(language) => {
            let ticks = body.split(|ch| ch != '`').map(str::len).max().unwrap_or(0).saturating_add(1).max(3);
            let fence = "`".repeat(ticks);
            let language = if matches!(intent, EditIntent::InsertDiagram(_)) { "mermaid" } else if language.len() <= 32 && language.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_+#.-".contains(&byte)) { language } else { "" };
            (format!("{fence}{language}{eol}"), fence)
        }
        EditIntent::InsertMathBlock => {
            if body.contains("$$") { return Err(ControlError::Unsupported("选区包含公式块围栏".into())); }
            (format!("$${eol}"), "$$".to_string())
        }
        _ => return Ok(None),
    };
    let ending = if body.ends_with('\n') { "" } else { eol };
    if body.len().saturating_add(opening.len()).saturating_add(ending.len()).saturating_add(closing.len()) > crate::text_document::MAX_SEMANTIC_BYTES {
        return Err(ControlError::NeedsContext("转换后的块超过语义窗口预算；选区原文已保留".into()));
    }
    let inserted = format!("{close_before}{before}{opening}{body}{ending}{closing}{after}{open_after}");
    let content_start = parsed.identity.from_byte + (replace.start + close_before.len() + before.len() + opening.len()) as u64;
    let content_end = content_start + body.len() as u64;
    let after_selection = if selection.anchor_byte <= selection.head_byte {
        DocumentSelection { anchor_byte: content_start, head_byte: content_end }
    } else { DocumentSelection { anchor_byte: content_end, head_byte: content_start } };
    Ok(Some(EditResult {
        edits: vec![SourceEdit { from_byte: parsed.identity.from_byte + replace.start as u64, to_byte: parsed.identity.from_byte + replace.end as u64, inserted_text: inserted }],
        selection: after_selection,
        template_update: None,
    }))
}
