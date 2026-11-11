//! 语义块的键盘规则；只对当前完整结构提交局部补丁。
use super::{commands::{self, EditIntent, EditResult}, mapping::local_range, parse::ParsedWindow, protocol::*};
use std::ops::Range;

fn replace(parsed: &ParsedWindow, range: Range<usize>, text: String, caret: usize) -> EditResult {
    let base = parsed.identity.from_byte;
    EditResult { edits: vec![SourceEdit { from_byte: base + range.start as u64, to_byte: base + range.end as u64, inserted_text: text }],
        selection: DocumentSelection::caret(base + caret as u64), template_update: None }
}

/// 文件尾部的编辑落点不改写原文；首次输入时才把必要分隔符并入同一事务。
pub(super) fn trailing_prefix(parsed: &ParsedWindow) -> Option<String> {
    use pulldown_cmark::{Event, Tag};
    if parsed.context.has_following_block || parsed.context.source_only || parsed.source.is_empty() { return None; }
    let source = &*parsed.source;
    let mut depth = 0usize;
    let mut last = None;
    for (event, range) in &parsed.events {
        if depth == 0 && matches!(event, Event::Start(_) | Event::Rule | Event::DisplayMath(_)) { last = Some((event, range)); }
        match event { Event::Start(_) => depth += 1, Event::End(_) => depth = depth.saturating_sub(1), _ => {} }
    }
    let (event, range) = last?;
    if parsed.blank_paragraphs.iter().any(|at| *at >= range.end) || !source[range.end..].trim().is_empty() { return None; }
    let special = matches!(event, Event::Rule | Event::DisplayMath(_) | Event::Start(Tag::Table(_) | Tag::HtmlBlock | Tag::FootnoteDefinition(_)))
        || matches!(event, Event::Start(Tag::CodeBlock(_)))
        || (matches!(event, Event::Start(Tag::Paragraph)) && {
            let raw = source[range.clone()].trim();
            if raw.starts_with("![") {
                let mut end = range.start;
                for node in parsed.nodes.iter().filter(|node| node.kind == "image"
                    && range.start <= node.source.start && node.source.end <= range.end)
                {
                    if node.source.start < end { continue; }
                    if !source[end..node.source.start].trim().is_empty() { return None; }
                    let suffix = &source[node.source.end..range.end];
                    let attributes = super::image_properties::markdown_attributes(suffix);
                    end = node.source.end + attributes.map_or(0, |(length, _, _)| length);
                }
                end > range.start && source[end..range.end].trim().is_empty()
            } else {
                super::image_properties::html_image_tag(source, range.clone()).is_some_and(|(image, _)| {
                    source[range.start..image.start].trim().is_empty() && source[image.end..range.end].trim().is_empty()
                })
            }
        });
    if !special { return None; }
    // CommonMark 会把独占一行的 <font>/<span> 开标签也解析成 HtmlBlock。
    // 行内 HTML 和尚未闭合的容器仍在输入中，不能把后续字符移到新段落。
    if matches!(event, Event::Start(Tag::HtmlBlock)) && !completed_html_block(&source[range.clone()]) { return None; }
    if matches!(event, Event::Start(Tag::CodeBlock(_))) {
        let raw = source[range.clone()].trim_end();
        let first = raw.lines().next()?.trim_start();
        let marker = first.chars().next()?;
        let count = first.chars().take_while(|c| *c == marker).count();
        if !matches!(marker, '`' | '~') || count < 3 || !raw.lines().last()?.trim().starts_with(&marker.to_string().repeat(count)) || !raw.contains('\n') { return None; }
    }
    let tail = &source[source.trim_end_matches(['\r', '\n']).len()..];
    Some(commands::line_ending(source, source.len()).repeat(2usize.saturating_sub(tail.bytes().filter(|b| *b == b'\n').count())))
}
/// 空续写段落与紧邻的独立图片之间至多隔着一组段落分隔符。
pub(super) fn image_before_empty_paragraph(parsed: &ParsedWindow, at: usize) -> Option<Range<usize>> {
    let source = &*parsed.source;
    if parsed.blank_paragraphs.binary_search(&at).is_err()
        && (at != source.len() || parsed.context.has_following_block)
    {
        return None;
    }
    let before = source[..at].trim_end_matches([' ', '\t', '\r', '\n']);
    let end = before.len();
    if !before.ends_with([')', ']', '}', '>'])
        || source[end..at].bytes().filter(|byte| *byte == b'\n').take(3).count() > 2
    {
        return None;
    }
    if end < at
        && let Some(image) = super::table_inline::atomic_range_at(parsed, end - 1)
        && image.end == end
        && source[commands::line_start(source, image.start)..image.start].trim().is_empty()
    {
        let markup = &source[image.clone()];
        if markup.starts_with("![") || markup.as_bytes().get(..4).is_some_and(|tag| tag.eq_ignore_ascii_case(b"<img")) {
            return Some(image);
        }
    }

    // 对齐的 HTML 图片落在段落标签内，空续写行位于结束标签之外。
    let fragment = parsed.html_fragments.iter().find(|fragment| fragment.source.end == end && fragment.label == "<p>")?;
    let [cell] = fragment.cells.as_slice() else { return None; };
    let [run] = cell.runs.as_slice() else { return None; };
    matches!(run.atom, Some(super::html_table::InlineAtom::Image { .. })).then(|| run.source.clone())
}

fn completed_html_block(source: &str) -> bool {
    let raw = source.trim();
    if raw.starts_with("<!--") { return raw.ends_with("-->"); }
    let Some(first) = super::nomo_syntax::html_tag(raw, 0) else { return false; };
    if first.closing { return false; }
    if matches!(first.name.as_str(), "hr" | "img") { return first.end == raw.len(); }
    if !matches!(first.name.as_str(), "table" | "details" | "div" | "p" | "blockquote" | "pre" | "ul" | "ol"
        | "section" | "article" | "aside" | "figure" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") { return false; }
    let mut depth = 1usize;
    let mut cursor = first.end;
    while let Some(relative) = raw[cursor..].find('<') {
        let at = cursor + relative;
        if raw[at..].starts_with("<!--") {
            let Some(end) = raw[at..].find("-->") else { return false; };
            cursor = at + end + 3;
        } else if let Some(tag) = super::nomo_syntax::html_tag(raw, at) {
            cursor = tag.end;
            if tag.name == first.name {
                if tag.closing {
                    depth -= 1;
                    if depth == 0 { return cursor == raw.len(); }
                } else { depth += 1; }
            }
        } else { cursor = at + 1; }
    }
    false
}

fn move_to(parsed: &ParsedWindow, at: usize) -> EditResult {
    EditResult { edits: Vec::new(), selection: DocumentSelection::caret(parsed.identity.from_byte + at as u64), template_update: None }
}

pub(super) fn semantic(parsed: &ParsedWindow, selection: DocumentSelection, intent: &EditIntent) -> Option<EditResult> {
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range()).ok()?;
    if !range.is_empty() { return None; }
    let at = range.start;
    let source = &*parsed.source;
    let eol = commands::line_ending(source, at);
    if at == source.len() {
        if let Some(prefix) = trailing_prefix(parsed) {
            let text = match intent {
                EditIntent::Replace(text) if !text.is_empty() => Some(format!("{prefix}{text}")),
                EditIntent::ParagraphBreak | EditIntent::Newline | EditIntent::SoftBreak => Some(format!("{prefix}{eol}{eol}")),
                EditIntent::Backspace | EditIntent::Delete => return Some(move_to(parsed, at)),
                _ => None,
            };
            if let Some(text) = text { let caret = at + text.len(); return Some(replace(parsed, at..at, text, caret)); }
        }
    }
    // 从内向外处理折叠块，退出只离开当前这一层。
    if let Some(block) = parsed.details.iter().filter(|b| b.source.start <= at && at < b.source.end).min_by_key(|b| b.source.len()) {
        let summary = block.title(source);
        let in_summary = summary.start <= at && at <= summary.end;
        if in_summary && matches!(intent, EditIntent::Newline | EditIntent::ParagraphBreak | EditIntent::SoftBreak) {
            let first = parsed.nodes.iter().filter(|n| n.source.start >= block.body.start && n.source.end <= block.body.end && matches!(n.kind, "paragraph" | "heading" | "code"))
                .min_by_key(|n| n.source.start);
            return Some(move_to(parsed, first.map_or_else(|| block.empty_body_caret(source).unwrap_or(block.body.start), |n| {
                if n.kind == "code" { source[n.source.clone()].find('\n').map_or(n.source.start, |p| n.source.start + p + 1) } else { n.source.start }
            })));
        }
        if in_summary && at == summary.start && matches!(intent, EditIntent::Backspace) {
            if source[summary.clone()].trim().is_empty() && source[block.body.clone()].trim().is_empty() {
                return Some(replace(parsed, block.source.clone(), String::new(), block.source.start));
            }
            return Some(move_to(parsed, at));
        }
        if in_summary && at == summary.end && matches!(intent, EditIntent::Delete) { return Some(move_to(parsed, at)); }
        if block.body.start <= at && at <= block.body.end {
            if source[block.body.start..at].trim().is_empty() && matches!(intent, EditIntent::Backspace) {
                return Some(move_to(parsed, summary.end));
            }
            let line = commands::line_start(source, at).max(block.body.start);
            if matches!(intent, EditIntent::ParagraphBreak | EditIntent::Newline)
                && source[line..at].trim().is_empty() && source[at..block.body.end].trim().is_empty() {
                let tail = &source[block.source.end..];
                let mut breaks = 0;
                let mut existing = 0;
                for (offset, byte) in tail.bytes().enumerate() {
                    if byte == b'\n' {
                        breaks += 1;
                        existing = offset + 1;
                        if breaks == 2 { break; }
                    } else if byte != b'\r' { break; }
                }
                let text = eol.repeat(2usize.saturating_sub(breaks));
                // 复用的换行也属于退出落点，不能把后续输入留在 </details> 后。
                let caret = block.source.end + text.len() + existing;
                return Some(replace(parsed, block.source.end..block.source.end, text, caret));
            }
        }
    }
    if matches!(intent, EditIntent::ParagraphBreak | EditIntent::Newline | EditIntent::Backspace | EditIntent::Delete) {
        let quote = parsed.events.iter().filter_map(|(event, span)| {
            let pulldown_cmark::Event::Start(pulldown_cmark::Tag::BlockQuote(kind)) = event else { return None; };
            let end = span.start + source[span.clone()].trim_end_matches(['\r', '\n']).len();
            (span.start <= at && at <= end).then_some((kind, span.start..end))
        }).min_by_key(|(_, span)| span.len());
        if let Some((kind, span)) = quote {
            if kind.is_some() && at == span.end && !source[span.clone()].contains('\n')
                && matches!(intent, EditIntent::ParagraphBreak | EditIntent::Newline)
            {
                // 手写提示标记后的第一次 Enter 进入正文，已有空正文行才执行退出。
                let before = &source[commands::line_start(source, at)..at];
                let text = format!("{eol}{}", commands::continuation(before));
                let caret = at + text.len();
                return Some(replace(parsed, at..at, text, caret));
            }
            // 提示类型是容器标记；空正文退出或删除时一并移除，避免留下不可删除的标题。
            let body = if kind.is_some() {
                source[span.clone()].find('\n').map_or(span.end, |line| span.start + line + 1)
            } else { span.start };
            // 检查原文，保留解析事件中不可见的引用定义、注释等用户内容。
            if source[body..span.end].bytes().all(|byte| matches!(byte, b'>' | b' ' | b'\t' | b'\r' | b'\n')) {
                let caret = span.start;
                return Some(replace(parsed, span, String::new(), caret));
            }
        }
    }
    if matches!(intent, EditIntent::ParagraphBreak | EditIntent::Newline) {
        if let Some(node) = parsed.nodes.iter().find(|n| n.kind == "heading" && n.source.start <= at && at <= n.source.end) {
            let raw = &source[node.source.clone()];
            let marker = raw.bytes().take_while(|b| *b == b'#').count();
            let content = node.source.start + marker + raw[marker..].len() - raw[marker..].trim_start_matches([' ', '\t']).len();
            if marker > 0 && at == content {
                return Some(replace(parsed, node.source.start..node.source.start, eol.repeat(2), node.source.start));
            }
        }
    }
    None
}

pub(super) fn indent(parsed: &ParsedWindow, selection: DocumentSelection, outdent: bool, unit: &str) -> Result<EditResult, ControlError> {
    if !matches!(unit, "  " | "    " | "\t") { return Err(ControlError::InvalidRange); }
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range())?;
    let edits = commands::selected_lines(&parsed.source, range).into_iter().map(|line| {
        let text = &parsed.source[line.clone()];
        let trim = if !outdent { 0 } else if text.starts_with('\t') { 1 } else { text.bytes().take_while(|b| *b == b' ').count().min(unit.len()) };
        SourceEdit { from_byte: parsed.identity.from_byte + line.start as u64, to_byte: parsed.identity.from_byte + (line.start + trim) as u64,
            inserted_text: if outdent { String::new() } else { unit.into() } }
    }).collect::<Vec<_>>();
    Ok(EditResult { selection: commands::map_selection(selection, &edits), edits, template_update: None })
}

pub(super) fn adjust_heading(parsed: &ParsedWindow, selection: DocumentSelection, increase: bool) -> Result<EditResult, ControlError> {
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range())?;
    let mut edits = Vec::new();
    for line in commands::selected_lines(&parsed.source, range) {
        if parsed.nodes.iter().any(|n| matches!(n.kind, "code" | "table" | "html") && n.source.contains(&line.start)) { continue; }
        let raw = &parsed.source[line.clone()];
        let hashes = raw.bytes().take_while(|b| *b == b'#').count();
        let level = if (1..=6).contains(&hashes) && raw.as_bytes().get(hashes).is_none_or(|b| b.is_ascii_whitespace()) { hashes } else { 0 };
        if !increase && level == 0 { continue; }
        let next = if increase { if level == 0 { 1 } else { level.saturating_sub(1).max(1) } } else { (level + 1).min(6) };
        if next == level { continue; }
        let consumed = if level > 0 { hashes + raw[hashes..].len() - raw[hashes..].trim_start_matches([' ', '\t']).len() } else { 0 };
        edits.push(SourceEdit { from_byte: parsed.identity.from_byte + line.start as u64, to_byte: parsed.identity.from_byte + (line.start + consumed) as u64, inserted_text: format!("{} ", "#".repeat(next)) });
    }
    Ok(EditResult { selection: commands::map_selection(selection, &edits), edits, template_update: None })
}

pub(super) fn toggle_rule(parsed: &ParsedWindow, selection: DocumentSelection) -> Result<EditResult, ControlError> {
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range())?;
    for (event, span) in &parsed.events {
        if !matches!(event, pulldown_cmark::Event::Rule) { continue; }
        let selected = range.start <= span.start && span.end <= range.end;
        let adjacent = range.is_empty() && if range.start >= span.end { parsed.source[span.end..range.start].trim().is_empty() }
            else if range.start <= span.start { parsed.source[range.start..span.start].trim().is_empty() } else { true };
        if selected || adjacent { return Ok(replace(parsed, span.clone(), String::new(), span.start)); }
    }
    commands::apply(parsed, selection, EditIntent::InsertBlock("---".into()))
}

pub(super) fn triple_selection(parsed: &ParsedWindow, at: usize) -> Range<usize> {
    if let Some(node) = parsed.nodes.iter().filter(|n| n.source.start <= at && at < n.source.end
        && matches!(n.kind, "paragraph" | "heading" | "code" | "table" | "image" | "html")).min_by_key(|n| n.source.len()) {
        if matches!(node.kind, "paragraph" | "heading") {
            return text_block_selection(parsed, &node.source, false);
        }
        return node.source.clone();
    }
    commands::line_start(&parsed.source, at)..parsed.source[at..].find('\n').map_or(parsed.source.len(), |p| at + p)
}

pub(super) fn text_block_selection(
    parsed: &ParsedWindow, source: &Range<usize>, include_inline_objects: bool,
) -> Range<usize> {
    use pulldown_cmark::{Event, Tag};
    let mut spans = parsed.events.iter().filter_map(|(event, range)| {
        if range.start < source.start || range.end > source.end { return None; }
        let text = matches!(event, Event::Text(_) | Event::Code(_) | Event::InlineMath(_));
        let object = include_inline_objects && matches!(event,
            Event::Start(Tag::Image { .. }) | Event::FootnoteReference(_) | Event::InlineHtml(_));
        (text || object).then_some(range)
    });
    let Some(first) = spans.next() else { return source.clone(); };
    first.start..spans.fold(first.end, |end, range| end.max(range.end))
}
