//! 按原版行内源码命令的边界拆分补丁；局部取消格式不改写选区之外的正文。
use super::{
    commands::{EditResult, map_selection},
    mapping::local_range,
    parse::{InlineFormat, InlineSpan, ParsedWindow},
    protocol::{ControlError, DocumentSelection, SourceEdit},
};
use pulldown_cmark::{Event, Tag};
use std::{collections::BTreeMap, ops::Range};

pub fn html_pair(format: InlineFormat) -> (String, String) {
    let tag = match format {
        InlineFormat::Strong => "strong",
        InlineFormat::Emphasis => "em",
        InlineFormat::Code => "code",
        InlineFormat::Strike => "del",
        InlineFormat::Underline => "u",
        InlineFormat::Highlight => "mark",
        InlineFormat::Superscript => "sup",
        InlineFormat::Subscript => "sub",
        InlineFormat::Link => "a",
    };
    (format!("<{tag}>"), format!("</{tag}>"))
}

pub fn opposite(format: InlineFormat) -> Option<InlineFormat> {
    match format {
        InlineFormat::Subscript => Some(InlineFormat::Superscript),
        InlineFormat::Superscript => Some(InlineFormat::Subscript),
        _ => None,
    }
}

/// 表格、标题和列表正文分别处理；围栏、列表标记和块之间的换行不进入格式命令。
fn blocks(parsed: &ParsedWindow, selection: &Range<usize>) -> Vec<Range<usize>> {
    let mut stack = Vec::new();
    let mut ranges = BTreeMap::<usize, Range<usize>>::new();
    for (event, range) in &parsed.events {
        match event {
            Event::Start(tag) => stack.push((tag, range)),
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(_)
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::SoftBreak
            | Event::HardBreak => {
                if stack
                    .iter()
                    .any(|(tag, _)| matches!(tag, Tag::CodeBlock(_) | Tag::HtmlBlock))
                {
                    continue;
                }
                if let Some((_, block)) = stack.iter().rev().find(|(tag, _)| {
                    matches!(
                        tag,
                        Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell | Tag::Item
                    )
                }) {
                    // 包括行内起止标记，但不要包括块级前缀。
                    let outer = stack
                        .iter()
                        .rev()
                        .take_while(|(_, r)| r.start != block.start || r.end != block.end)
                        .map(|(_, r)| *r)
                        .fold(range.clone(), |a, b| a.start.min(b.start)..a.end.max(b.end));
                    ranges
                        .entry(block.start)
                        .and_modify(|r| {
                            r.start = r.start.min(outer.start);
                            r.end = r.end.max(outer.end);
                        })
                        .or_insert(outer);
                }
            }
            _ => {}
        }
    }
    for cell in parsed.html_cells() {
            // 段落分隔符不属于可格式化文字；每段分别生成命令。
            let mut from = None;
            let mut end = cell.content.start;
            for run in &cell.runs {
                if run.text == "\n" && parsed.source[run.source.clone()].starts_with("</p>") {
                    if let Some(start) = from.take() {
                        ranges.insert(start, start..end);
                    }
                } else {
                    from.get_or_insert(run.source.start);
                    end = run.source.end;
                }
            }
            if let Some(start) = from {
                ranges.insert(start, start..end);
            }
    }
    ranges
        .into_values()
        .filter_map(|block| {
            let from = block.start.max(selection.start);
            let to = block.end.min(selection.end);
            (from < to).then_some(from..to)
        })
        .collect()
}

#[derive(Default)]
struct Changes {
    deletions: Vec<Range<usize>>,
    literal_edits: Vec<SourceEdit>,
    // order keeps closing delimiters before opening delimiters at a shared endpoint.
    insertions: BTreeMap<usize, Vec<(i64, String)>>,
}
impl Changes {
    fn insert(&mut self, at: usize, order: i64, text: String) {
        self.insertions.entry(at).or_default().push((order, text));
    }
    fn remove(&mut self, span: &InlineSpan) {
        self.deletions
            .extend([span.open.clone(), span.close.clone()]);
    }
    fn wrap(&mut self, range: Range<usize>, pair: (String, String)) {
        self.wrap_at_order(range.clone(), pair, range.start);
    }
    fn wrap_at_order(&mut self, range: Range<usize>, pair: (String, String), outer_start: usize) {
        let order = outer_start.min(i64::MAX as usize / 4) as i64;
        self.insert(range.start, i64::MAX / 2 + order, pair.0);
        self.insert(range.end, -order - 1, pair.1);
    }
    fn finish(mut self, base: u64) -> Result<Vec<SourceEdit>, ControlError> {
        self.deletions.sort_by_key(|r| (r.start, r.end));
        self.deletions.dedup();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for range in self.deletions {
            if let Some(last) = ranges.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                ranges.push(range);
            }
        }
        let mut edits = Vec::new();
        for range in ranges {
            let mut text = String::new();
            // 嵌套定界符会相邻合并；保留其间新标签的原始顺序，而不是丢掉内层格式。
            let positions: Vec<_> = self
                .insertions
                .range(range.start..=range.end)
                .map(|(at, _)| *at)
                .collect();
            for at in positions {
                if let Some(mut values) = self.insertions.remove(&at) {
                    values.sort_by_key(|(order, _)| *order);
                    for (_, value) in values {
                        text.push_str(&value);
                    }
                }
            }
            edits.push(SourceEdit {
                from_byte: base + range.start as u64,
                to_byte: base + range.end as u64,
                inserted_text: text,
            });
        }
        for (at, mut values) in self.insertions {
            values.sort_by_key(|(order, _)| *order);
            edits.push(SourceEdit {
                from_byte: base + at as u64,
                to_byte: base + at as u64,
                inserted_text: values.into_iter().map(|(_, s)| s).collect(),
            });
        }
        edits.extend(self.literal_edits);
        edits.sort_by_key(|e| (e.from_byte, e.to_byte));
        edits.dedup();
        if edits
            .windows(2)
            .any(|pair| pair[0].to_byte > pair[1].from_byte)
        {
            return Err(ControlError::Unsupported("格式补丁范围发生重叠".into()));
        }
        Ok(edits)
    }
}

fn remove_selected(changes: &mut Changes, span: &InlineSpan, range: &Range<usize>) {
    changes.remove(span);
    if range.start > span.content.start {
        changes.wrap_at_order(
            span.content.start..range.start.min(span.content.end),
            html_pair(span.format),
            span.open.start,
        );
    }
    if range.end < span.content.end {
        changes.wrap_at_order(
            range.end.max(span.content.start)..span.content.end,
            html_pair(span.format),
            span.open.start,
        );
    }
}

fn selection_regions(parsed: &ParsedWindow, selection: DocumentSelection, format: Option<InlineFormat>) -> Result<Vec<Range<usize>>, ControlError> {
    let selected = local_range(&parsed.source, parsed.identity.from_byte, selection.range())?;
    let mut regions = blocks(parsed, &selected);
    if regions.iter().any(|range| {
        parsed.inline.iter().any(|span| {
            [span.open.clone(), span.close.clone()]
                .iter()
                .any(|marker| {
                    marker.start < range.start && range.start < marker.end
                        || marker.start < range.end && range.end < marker.end
                })
        })
    }) {
        return Err(ControlError::Unsupported("格式选区截断语法标记".into()));
    }
    // 选区跨出链接标签时拆开格式边界，不能把目标 URL 包进 Markdown 定界符。
    for span in parsed
        .inline
        .iter()
        .filter(|s| s.format == InlineFormat::Link)
    {
        let mut next = Vec::new();
        for range in regions {
            if span.intersects(&range)
                && !(range.start <= span.open.start && range.end >= span.close.end)
            {
                for part in [
                    range.start..range.end.min(span.open.start),
                    range.start.max(span.content.start)..range.end.min(span.content.end),
                    range.start.max(span.close.end)..range.end,
                ] {
                    if part.start < part.end {
                        next.push(part);
                    }
                }
            } else {
                next.push(range);
            }
        }
        regions = next;
    }
    for range in &mut regions {
        if format != Some(InlineFormat::Code) {
            for span in parsed
                .inline
                .iter()
                .filter(|span| span.format == InlineFormat::Code)
            {
                if range.start < span.content.end && range.end > span.content.start {
                    if range.start > span.content.start || range.end < span.content.end {
                        return Err(ControlError::Unsupported(
                            "不能在部分行内代码中插入格式标记".into(),
                        ));
                    }
                    range.start = range.start.min(span.open.start);
                    range.end = range.end.max(span.close.end);
                }
            }
        }
    }
    Ok(regions)
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|range| !range.is_empty());
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

fn regions_formatted(parsed: &ParsedWindow, regions: &[Range<usize>], format: InlineFormat) -> bool {
    let markers = merge_ranges(parsed.inline.iter().flat_map(|span| [span.open.clone(), span.close.clone()]).collect());
    let formatted = merge_ranges(parsed.inline.iter().filter(|span| span.format == format).map(|span| span.content.clone()).collect());
    let contains = |ranges: &[Range<usize>], at: usize| {
        let index = ranges.partition_point(|range| range.end <= at);
        ranges.get(index).is_some_and(|range| range.start <= at && at < range.end)
    };
    regions.iter().all(|range| {
        let mut has_content = false;
        let covered = parsed.source[range.clone()].char_indices().all(|(offset, _)| {
            let at = range.start + offset;
            if contains(&markers, at) { return true; }
            has_content = true;
            contains(&formatted, at)
        });
        has_content && covered
    })
}

pub(crate) fn selection_formatted(parsed: &ParsedWindow, selection: DocumentSelection, format: InlineFormat) -> Result<Option<bool>, ControlError> {
    let regions = selection_regions(parsed, selection, Some(format))?;
    Ok((!regions.is_empty()).then(|| regions_formatted(parsed, &regions, format)))
}

pub fn selected(parsed: &ParsedWindow, selection: DocumentSelection, format: Option<InlineFormat>) -> Result<EditResult, ControlError> {
    selected_with_toggle(parsed, selection, format, None)
}

pub(crate) fn selected_with_toggle(parsed: &ParsedWindow, selection: DocumentSelection, format: Option<InlineFormat>, remove: Option<bool>) -> Result<EditResult, ControlError> {
    let base = parsed.identity.from_byte;
    let regions = selection_regions(parsed, selection, format)?;
    let all_formatted = format.is_some_and(|format| remove.unwrap_or_else(|| regions_formatted(parsed, &regions, format)));
    let mut changes = Changes::default();
    for range in regions {
        if format.is_none() || all_formatted {
            for span in parsed.inline.iter().filter(|span| {
                span.intersects(&range)
                    && format.map_or(span.format != InlineFormat::Link, |f| f == span.format)
            }) {
                if span.format == InlineFormat::Code {
                    if range.start > span.content.start || range.end < span.content.end {
                        return Err(ControlError::Unsupported(
                            "部分清除行内代码需要保留字面内容".into(),
                        ));
                    }
                    changes.literal_edits.extend(release_code(parsed, span));
                    continue;
                }
                remove_selected(&mut changes, span, &range);
            }
        } else if let Some(format) = format {
            if let Some(other) = opposite(format) {
                for span in parsed
                    .inline
                    .iter()
                    .filter(|s| s.format == other && s.intersects(&range))
                {
                    remove_selected(&mut changes, span, &range);
                }
            }
            let mut range = range;
            let mut merged = Vec::new();
            loop {
                let next = parsed.inline.iter().enumerate().find(|(i, s)| {
                    s.format == format && s.intersects(&range) && !merged.contains(i)
                });
                let Some((index, span)) = next else {
                    break;
                };
                changes.remove(span);
                merged.push(index);
                range = range.start.min(span.open.start)..range.end.max(span.close.end);
            }
            let in_html = parsed.html_cell_at(range.start).is_some();
            let text = &parsed.source[range.clone()];
            let pair = if in_html
                || !merged.is_empty()
                || text.starts_with(char::is_whitespace)
                || text.ends_with(char::is_whitespace)
                || matches!(format, InlineFormat::Superscript | InlineFormat::Subscript)
                    && text.chars().any(char::is_whitespace)
            {
                html_pair(format)
            } else {
                let (a, b) = format.markers();
                (a.into(), b.into())
            };
            changes.wrap(range, pair);
        }
    }
    let edits = changes.finish(base)?;
    let mut after = map_selection(selection, &edits);
    // 选区右端保持在闭合标签之前。
    let end = selection.range().end;
    let closing = edits
        .iter()
        .find(|e| e.from_byte == end && e.to_byte == end)
        .map_or(0, |e| e.inserted_text.len() as u64);
    if selection.anchor_byte <= selection.head_byte {
        after.head_byte -= closing;
    } else {
        after.anchor_byte -= closing;
    }
    Ok(EditResult {
        template_update: None,
        edits,
        selection: after,
    })
}

pub fn release_code(parsed: &ParsedWindow, span: &InlineSpan) -> Vec<SourceEdit> {
    let base = parsed.identity.from_byte;
    if parsed.html_cell_at(span.open.start).is_some()
    {
        return [span.open.clone(), span.close.clone()]
            .into_iter()
            .map(|range| SourceEdit {
                from_byte: base + range.start as u64,
                to_byte: base + range.end as u64,
                inserted_text: String::new(),
            })
            .collect();
    }
    let body = &parsed.source[span.content.clone()];
    let padded = body.starts_with(' ')
        && body.ends_with(' ')
        && body.len() > 2
        && body.chars().any(|c| c != ' ');
    let content = span.content.start + usize::from(padded)..span.content.end - usize::from(padded);
    let mut edits = vec![SourceEdit {
        from_byte: base + span.open.start as u64,
        to_byte: base + content.start as u64,
        inserted_text: String::new(),
    }];
    for (offset, ch) in parsed.source[content.clone()].char_indices() {
        if ch.is_ascii_punctuation() {
            edits.push(SourceEdit {
                from_byte: base + (content.start + offset) as u64,
                to_byte: base + (content.start + offset + 1) as u64,
                inserted_text: format!("\\{ch}"),
            });
        }
    }
    edits.push(SourceEdit {
        from_byte: base + content.end as u64,
        to_byte: base + span.close.end as u64,
        inserted_text: String::new(),
    });
    edits
}

/// 部分取消代码格式时，左右两侧仍是代码，选中部分转为字面文字。
/// 重建只覆盖该代码 span；不能让其中的 `*`、链接或 HTML 意外变成新语法。
pub fn release_code_selection(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    span: &InlineSpan,
) -> Result<EditResult, ControlError> {
    let base = parsed.identity.from_byte;
    let range = local_range(&parsed.source, base, selection.range())?;
    let html_cell = parsed.html_cell_at(span.open.start).is_some();
    let markdown = parsed.source[span.open.clone()].starts_with('`');
    if !markdown && !html_cell {
        return Err(ControlError::Unsupported(
            "独立 HTML 代码的部分格式取消".into(),
        ));
    }
    let raw = &parsed.source[span.content.clone()];
    let padded = markdown
        && raw.starts_with(' ')
        && raw.ends_with(' ')
        && raw.len() > 2
        && raw.chars().any(|c| c != ' ');
    let content = span.content.start + usize::from(padded)..span.content.end - usize::from(padded);
    let selected = range.start.max(content.start).min(content.end)
        ..range.end.min(content.end).max(content.start);
    if selected.start >= selected.end {
        return Err(ControlError::InvalidRange);
    }
    if html_cell {
        for cell in parsed.html_cells() {
            if cell.runs.iter().any(|run| {
                parsed.source.get(run.source.clone()) != Some(run.text.as_str())
                    && [selected.start, selected.end]
                        .iter()
                        .any(|at| run.source.start < *at && *at < run.source.end)
            }) {
                return Err(ControlError::InvalidRange);
            }
        }
    }
    let normalize = |s: &str| s.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let code = |s: &str| {
        if s.is_empty() {
            return String::new();
        }
        if html_cell {
            return format!("<code>{s}</code>");
        }
        let s = normalize(s);
        let ticks = s.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
        let fence = "`".repeat(ticks);
        let padding = if s.starts_with('`')
            || s.ends_with('`')
            || s.starts_with(' ') && s.ends_with(' ') && s.chars().any(|c| c != ' ')
        {
            " "
        } else {
            ""
        };
        format!("{fence}{padding}{s}{padding}{fence}")
    };
    let left = code(&parsed.source[content.start..selected.start]);
    let right = code(&parsed.source[selected.end..content.end]);
    let middle = if html_cell {
        parsed.source[selected.clone()].to_string()
    } else {
        normalize(&parsed.source[selected.clone()])
            .chars()
            .map(|ch| {
                if ch.is_ascii_punctuation() {
                    format!("\\{ch}")
                } else {
                    ch.to_string()
                }
            })
            .collect::<String>()
    };
    let start = base + (span.open.start + left.len()) as u64;
    let end = start + middle.len() as u64;
    Ok(EditResult {
        edits: vec![SourceEdit {
            from_byte: base + span.open.start as u64,
            to_byte: base + span.close.end as u64,
            inserted_text: format!("{left}{middle}{right}"),
        }],
        selection: if selection.anchor_byte <= selection.head_byte {
            DocumentSelection {
                anchor_byte: start,
                head_byte: end,
            }
        } else {
            DocumentSelection {
                anchor_byte: end,
                head_byte: start,
            }
        },
        template_update: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(
        source: &str,
        range: Range<usize>,
        format: Option<InlineFormat>,
    ) -> (String, ParsedWindow) {
        let parsed = super::super::parse::fixture(source);
        let result = selected(
            &parsed,
            DocumentSelection {
                anchor_byte: 100 + range.start as u64,
                head_byte: 100 + range.end as u64,
            },
            format,
        )
        .unwrap();
        let mut text = source.to_string();
        for edit in result.edits.iter().rev() {
            text.replace_range(
                (edit.from_byte - 100) as usize..(edit.to_byte - 100) as usize,
                &edit.inserted_text,
            );
        }
        let parsed = super::super::parse::fixture(&text);
        (text, parsed)
    }
    #[test]
    fn partial_removal_preserves_both_sides_and_nested_format() {
        let (text, _) = run("**abcd**", 3..5, Some(InlineFormat::Strong));
        assert_eq!(text, "<strong>a</strong>bc<strong>d</strong>");
        let (text, parsed) = run("**a*bcde*f**", 5..7, Some(InlineFormat::Strong));
        assert!(text.contains("cd"));
        let at = text.find("cd").unwrap();
        assert!(
            !parsed
                .inline
                .iter()
                .any(|s| s.format == InlineFormat::Strong && s.content.contains(&at))
        );
        assert!(
            parsed
                .inline
                .iter()
                .any(|s| s.format == InlineFormat::Emphasis && s.content.contains(&at))
        );
    }
    #[test]
    fn whitespace_scripts_and_multiple_blocks_follow_original_rules() {
        assert_eq!(
            run("a b", 1..2, Some(InlineFormat::Strong)).0,
            "a<strong> </strong>b"
        );
        assert_eq!(
            run("a b", 0..3, Some(InlineFormat::Superscript)).0,
            "<sup>a b</sup>"
        );
        let source = "one\r\n\r\n```text\r\nraw\r\n```\r\n\r\n## two\n";
        assert_eq!(
            run(source, 0..source.len(), Some(InlineFormat::Strong)).0,
            "**one**\r\n\r\n```text\r\nraw\r\n```\r\n\r\n## **two**\n"
        );
    }
    #[test]
    fn cross_link_formatting_keeps_link_destination_and_selected_text() {
        let (text, parsed) = run(
            "[abc](https://example.com)de",
            2..27,
            Some(InlineFormat::Strong),
        );
        assert_eq!(text, "[a**bc**](https://example.com)**d**e");
        assert!(parsed.events.iter().any(|(e,_)|matches!(e,Event::Start(Tag::Link{dest_url,..})if dest_url.as_ref()=="https://example.com")));
    }
    #[test]
    fn clearing_partial_nested_formats_keeps_unselected_sides() {
        let (text, parsed) = run("***abcd***", 4..6, None);
        let middle = text.find("bc").unwrap();
        assert!(
            parsed
                .inline
                .iter()
                .all(|span| !span.content.contains(&middle))
        );
        for letter in ['a', 'd'] {
            // Locate visible text rather than the letters in HTML tag names.
            let at = parsed
                .events
                .iter()
                .find_map(|(event, range)| match event {
                    Event::Text(s) if s.contains(letter) => {
                        Some(range.start + s.find(letter).unwrap())
                    }
                    _ => None,
                })
                .unwrap();
            for format in [InlineFormat::Strong, InlineFormat::Emphasis] {
                assert!(
                    parsed
                        .inline
                        .iter()
                        .any(|span| span.format == format && span.content.contains(&at)),
                    "{text}"
                );
            }
        }
    }
    #[test]
    fn clearing_code_preserves_literal_markup() {
        let (text, parsed) = run("`**literal** [x](url)`", 0..22, None);
        assert!(parsed.inline.is_empty(), "{text}");
        let visible: String = parsed
            .events
            .iter()
            .filter_map(|(event, _)| match event {
                Event::Text(s) => Some(s.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(visible, "**literal** [x](url)");
    }
}
