//! 链接标记只覆盖选中的行内正文；段落、格式和未选中链接仍保留各自边界。
use super::{LinkDraft, escape_label, escape_table_pipes, plain_text, serialize_destination};
use super::super::{
    commands::EditResult,
    inline_commands::{self, Changes},
    mapping::{local_range, text_segments},
    parse::{InlineFormat, InlineSpan, ParsedWindow},
    protocol::{ControlError, DocumentSelection, SourceEdit},
    table_inline,
};
use pulldown_cmark::Event;
use std::{collections::{BTreeMap, BTreeSet}, ops::Range};

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

pub(super) fn begin(parsed: &ParsedWindow, selection: DocumentSelection, range: Range<usize>, links: &[LinkDraft]) -> Result<LinkDraft, ControlError> {
    let regions = inline_commands::blocks(parsed, &range);
    if regions.is_empty() {
        return Err(ControlError::Unsupported("选区没有可应用链接的行内正文".into()));
    }
    let markers = parsed.inline.iter().flat_map(|span| [span.open.clone(), span.close.clone()])
        .chain(links.iter().flat_map(|link| [link.range.start..link.content.start, link.content.end..link.range.end]));
    if markers.into_iter().any(|marker| [range.start, range.end].iter().any(|at| marker.start < *at && *at < marker.end)) {
        return Err(ControlError::Unsupported("链接选区不能截断语法标记".into()));
    }
    for (event, source) in &parsed.events {
        if let Event::Text(text) = event {
            if !overlaps(source, &range) { continue; }
            if text_segments(text, &parsed.source[source.clone()], source.start).iter().any(|segment| !segment.exact
                && [range.start, range.end].iter().any(|at| segment.source.start < *at && *at < segment.source.end)) {
                return Err(ControlError::Unsupported("链接选区不能截断转义或实体字符".into()));
            }
        }
    }
    for cell in parsed.html_cells() {
        for at in [range.start, range.end] {
            if cell.content.contains(&at) && !super::super::commands::html_text_boundary(parsed, cell, at) {
                return Err(ControlError::Unsupported("链接选区不能截断 HTML 标记或实体".into()));
            }
        }
    }
    let first = links.iter().find(|link| overlaps(&link.content, &range));
    Ok(LinkDraft {
        text: regions.iter().map(|region| plain_text(parsed, region)).collect::<Vec<_>>().join("\n"),
        href: first.map_or_else(String::new, |link| link.href.clone()),
        active: first.is_some(), range: range.clone(), content: range,
        title: String::new(), html: false, attrs: BTreeMap::new(), selection: Some(selection), reversed: selection.anchor_byte > selection.head_byte,
    })
}

fn pair(parsed: &ParsedWindow, range: &Range<usize>, link: Option<&LinkDraft>, href: &str, anchor: bool) -> (String, String) {
    let table = table_inline::markdown_cell(parsed, range);
    let html = parsed.html_cell_at(range.start).is_some() || link.is_some_and(|link| link.html);
    let title = link.map_or("", |link| link.title.as_str());
    if html {
        let mut attrs = link.map_or_else(BTreeMap::new, |link| link.attrs.clone());
        if !anchor { attrs.remove("id"); attrs.remove("name"); }
        attrs.insert("href".into(), href.into());
        if !title.is_empty() { attrs.insert("title".into(), title.into()); }
        let attrs: String = attrs.into_iter().map(|(name, value)| format!(" {name}=\"{}\"", html_escape::encode_double_quoted_attribute(&value))).collect();
        let open = format!("<a{attrs}>");
        (if table { escape_table_pipes(&open) } else { open }, "</a>".into())
    } else {
        let title = if title.is_empty() { String::new() } else {
            format!(" \"{}\"", title.replace('&', "&amp;").replace('\\', "\\\\").replace('"', "\\\"").replace('\r', "&#13;").replace('\n', "&#10;"))
        };
        let close = format!("]({}{title})", serialize_destination(href));
        ("[".into(), if table { escape_table_pipes(&close) } else { close })
    }
}

/// 格式完整位于链接内时原样保留；只有选区跨出格式时才在其内容边界分段。
fn pieces(parsed: &ParsedWindow, range: Range<usize>, links: &[&LinkDraft]) -> Vec<Range<usize>> {
    let mut pieces = vec![range];
    for span in parsed.inline.iter().filter(|span| span.format != InlineFormat::Link) {
        pieces = pieces.into_iter().flat_map(|range| {
            if !span.intersects(&range)
                || range.start <= span.open.start && span.close.end <= range.end
                || span.content.start <= range.start && range.end <= span.content.end {
                return vec![range];
            }
            [range.start..range.end.min(span.open.start),
                range.start.max(span.content.start)..range.end.min(span.content.end),
                range.start.max(span.close.end)..range.end]
                .into_iter().filter(|part| part.start < part.end).collect()
        }).collect();
    }
    let mut markers = inline_commands::marker_ranges(parsed);
    markers.extend(links.iter().flat_map(|link| [link.range.start..link.content.start, link.content.end..link.range.end]));
    markers.sort_by_key(|marker| (marker.start, marker.end));
    pieces.into_iter().filter_map(|mut range| {
        // 旧链接定界符即将删除，不让新标签嵌进旧 URL，也不生成空链接。
        loop {
            let before = range.clone();
            for link in links {
                for marker in [link.range.start..link.content.start, link.content.end..link.range.end] {
                    if marker.start <= range.start && range.start < marker.end { range.start = marker.end.min(range.end); }
                    if marker.start < range.end && range.end <= marker.end { range.end = marker.start.max(range.start); }
                }
            }
            if range == before { break; }
        }
        let mut at = range.start;
        for marker in &markers {
            if marker.end <= at { continue; }
            if at < marker.start { break; }
            at = marker.end;
        }
        (at < range.end).then_some(range)
    }).collect()
}

/// 部分代码选区改用 HTML 代码标记，避免新链接成为反引号中的字面内容。
/// 原始代码中的标点转为实体；两侧代码的显示文字和格式保持不变。
fn split_code(parsed: &ParsedWindow, span: &InlineSpan, deleted: Option<&Range<usize>>, changes: &mut Changes) {
    let base = parsed.identity.from_byte;
    if deleted.is_none() {
        for (range, text) in [(span.open.clone(), "<code>"), (span.close.clone(), "</code>")] {
            changes.literal_edits.push(SourceEdit { from_byte: base + range.start as u64, to_byte: base + range.end as u64, inserted_text: text.into() });
        }
    }
    let raw = &parsed.source[span.content.clone()];
    let padded = raw.starts_with(' ') && raw.ends_with(' ') && raw.len() > 2 && raw.chars().any(|ch| ch != ' ');
    for (offset, ch) in raw.char_indices() {
        let at = span.content.start + offset;
        if deleted.is_some_and(|range| range.contains(&at)) { continue; }
        let text = if padded && (offset == 0 || offset + 1 == raw.len()) || ch == '\r' && raw.as_bytes().get(offset + 1) == Some(&b'\n') {
            String::new()
        } else if ch == '\r' || ch == '\n' { " ".into() }
        else if ch.is_ascii_punctuation() { format!("&#{};", ch as u32) }
        else { continue; };
        changes.literal_edits.push(SourceEdit { from_byte: base + at as u64, to_byte: base + (at + ch.len_utf8()) as u64, inserted_text: text });
    }
}

pub(super) fn edit(draft: &LinkDraft, parsed: &ParsedWindow, selection: DocumentSelection, shown: &str, href: Option<&str>) -> Result<EditResult, ControlError> {
    let base = parsed.identity.from_byte;
    let selected = local_range(&parsed.source, base, selection.range())?;
    let regions = inline_commands::blocks(parsed, &selected);
    let links = LinkDraft::links(parsed)?;
    let touched: Vec<_> = links.iter().filter(|link| regions.iter().any(|range| overlaps(&link.content, range))).collect();
    let replacing = href.is_some() && shown != draft.text;
    if replacing && (regions.len() != 1 || regions[0] != selected) {
        return Err(ControlError::Unsupported("跨段落或单元格只能修改链接地址，不能替换显示文字".into()));
    }
    if !replacing && touched.iter().any(|link| link.content.start <= selected.start && selected.end <= link.content.end && href == Some(link.href.as_str())) {
        return Ok(EditResult { edits: Vec::new(), selection, template_update: None });
    }
    let mut changes = Changes::default();
    let mut wraps = Vec::new();
    for link in &touched {
        changes.deletions.extend([link.range.start..link.content.start, link.content.end..link.range.end]);
        for range in [link.content.start..selected.start.min(link.content.end), selected.end.max(link.content.start)..link.content.end] {
            if range.start < range.end { wraps.push((range, Some(*link), link.href.as_str())); }
        }
    }
    if replacing {
        let markers = inline_commands::marker_ranges(parsed);
        for span in parsed.inline.iter().filter(|span| span.format != InlineFormat::Link && span.intersects(&selected)) {
            inline_commands::remove_selected(&mut changes, span, &selected, &markers);
            if span.format == InlineFormat::Code && parsed.source[span.open.clone()].starts_with('`') {
                // 标签已由 remove_selected 重建，只处理未替换的字面代码。
                split_code(parsed, span, Some(&selected), &mut changes);
            }
        }
        changes.deletions.push(selected.clone());
    } else if href.is_some() {
        for range in &regions {
            let mut at = range.start;
            for link in touched.iter().filter(|link| overlaps(&link.range, range)) {
                let start = link.range.start.max(range.start);
                if at < start { wraps.push((at..start, None, href.unwrap())); }
                let end = link.range.end.min(range.end);
                wraps.push((start..end, Some(*link), href.unwrap()));
                at = end;
            }
            if at < range.end { wraps.push((at..range.end, None, href.unwrap())); }
        }
    }
    if !replacing {
        for span in parsed.inline.iter().filter(|span| span.format == InlineFormat::Code && parsed.source[span.open.clone()].starts_with('`')) {
            if overlaps(&span.content, &selected) && (span.open.start < selected.start || selected.end < span.close.end) {
                split_code(parsed, span, None, &mut changes);
            }
        }
    }
    wraps.sort_by_key(|(range, _, _)| (range.start, range.end));
    let mut used_anchors = BTreeSet::new();
    let mut merged: Vec<(Range<usize>, Option<&LinkDraft>, &str, (String, String))> = Vec::new();
    for (range, link, target) in wraps {
        let wrapper = pair(parsed, &range, link, target, true);
        if let Some((previous, _, _, _)) = merged.last_mut().filter(|(previous, _, _, previous_pair)| previous.end == range.start && *previous_pair == wrapper) {
            previous.end = range.end;
        } else {
            merged.push((range, link, target, wrapper));
        }
    }
    // 同一原链接拆成多个格式片段时，id/name 只留在第一个片段。
    for (range, link, target, _) in merged {
        let parts = pieces(parsed, range, &touched);
        for part in parts {
            let anchor = link.is_none_or(|link| used_anchors.insert(link.range.start));
            let wrapper = pair(parsed, &part, link, target, anchor);
            if wrapper.0 == "[" {
                // 拆开原来配对的字面方括号后，各标签必须独立合法；不触碰格式或图片标记。
                for (event, source) in &parsed.events {
                    if !matches!(event, Event::Text(_)) || !overlaps(source, &part) { continue; }
                    for (offset, ch) in parsed.source[source.clone()].char_indices() {
                        let at = source.start + offset;
                        if !part.contains(&at) || !matches!(ch, '[' | ']') { continue; }
                        let slashes = parsed.source[..at].bytes().rev().take_while(|byte| *byte == b'\\').count();
                        if slashes % 2 == 0 {
                            changes.literal_edits.push(SourceEdit { from_byte: base + at as u64, to_byte: base + at as u64 + 1, inserted_text: format!("\\{ch}") });
                        }
                    }
                }
            }
            changes.wrap(part, wrapper);
        }
    }
    let (edits, result_range) = if let Some(href) = href.filter(|_| replacing) {
        let first = touched.first().copied();
        let anchor = first.is_none_or(|link| !used_anchors.contains(&link.range.start));
        let (open, close) = pair(parsed, &selected, first, href, anchor);
        let body = if parsed.html_cell_at(selected.start).is_some() { html_escape::encode_text(shown).into_owned() } else { escape_label(shown) };
        let body = if table_inline::markdown_cell(parsed, &selected) { escape_table_pipes(&body) } else { body };
        changes.insert(selected.start, 0, format!("{open}{body}{close}"));
        let (edits, at) = changes.finish_at(base, Some((selected.start, 0)))?;
        let start = at.ok_or(ControlError::InvalidRange)? + open.len() as u64;
        (edits, start..start + body.len() as u64)
    } else {
        let edits = changes.finish(base)?;
        let map = |point: u64, after: bool| {
            let mut shift = 0i128;
            for edit in &edits {
                if point < edit.from_byte { break; }
                if point == edit.to_byte && edit.from_byte < point {
                    shift += edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128;
                    continue;
                }
                if point <= edit.to_byte {
                    return (edit.from_byte as i128 + shift + if after { edit.inserted_text.len() as i128 } else { 0 }) as u64;
                }
                shift += edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128;
            }
            (point as i128 + shift) as u64
        };
        let first = pieces(parsed, regions[0].clone(), &touched);
        let last = pieces(parsed, regions.last().unwrap().clone(), &touched);
        let start = map(base + first.first().map_or(regions[0].start, |range| range.start) as u64, true);
        let end = map(base + last.last().map_or(regions.last().unwrap().end, |range| range.end) as u64, false);
        (edits, start..end)
    };
    super::super::commands::validate_html_edits(parsed, &edits)?;
    let selection = if selection.anchor_byte <= selection.head_byte {
        DocumentSelection { anchor_byte: result_range.start, head_byte: result_range.end }
    } else { DocumentSelection { anchor_byte: result_range.end, head_byte: result_range.start } };
    Ok(EditResult { edits, selection, template_update: None })
}
