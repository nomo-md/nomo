//! 工具栏、菜单、键盘和对象编辑共用源码补丁；未触及的源码保持原样。
use super::{
    mapping::local_range,
    parse::{InlineFormat, ParsedWindow},
    protocol::*,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditIntent {
    Replace(String),
    ToggleInline(InlineFormat),
    ClearFormatting,
    Heading(u8),
    LinePrefix(String),
    Indent { outdent: bool },
    Newline,
    ParagraphBreak,
    SoftBreak,
    Backspace,
    Delete,
    InsertBlock(String),
    InsertBlockAt { block: String, caret: usize },
    InsertCodeBlock(String),
    InsertMathBlock,
    InsertDetails,
    InsertDiagram(String),
    InsertParagraph { before: bool },
    Link { label: String, target: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditResult {
    pub edits: Vec<SourceEdit>,
    pub selection: DocumentSelection,
    pub template_update: Option<TemplateUpdate>,
}

/// HTML 表格里的文字是 HTML 文本节点。输入必须转义，格式命令使用其原生标签。
/// 源码视图仍调用 apply，保留逐字节编辑能力。
pub fn apply_semantic(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    intent: EditIntent,
) -> Result<EditResult, ControlError> {
    let large_text = matches!(&intent, EditIntent::Replace(text) if text.len() > super::session_window::policy().group_bytes as usize);
    let base = parsed.identity.from_byte;
    let range = local_range(&parsed.source, base, selection.range())?;
    if range.is_empty() && matches!(&intent, EditIntent::Backspace | EditIntent::Delete) {
        let at = if matches!(&intent, EditIntent::Backspace) { range.start.checked_sub(1) } else { Some(range.start) };
        if let Some(source) = at.and_then(|at| super::table_inline::atomic_range_at(parsed, at)) {
            return Ok(EditResult { template_update: None, edits: vec![SourceEdit {
                from_byte: base + source.start as u64, to_byte: base + source.end as u64, inserted_text: String::new(),
            }], selection: DocumentSelection::caret(base + source.start as u64) });
        }
    }
    if let EditIntent::Replace(text) = &intent {
        if super::table_inline::markdown_cell(parsed, &range) {
            let escaped = super::table_inline::markdown_input(&parsed.source, range.start, text);
            return apply(parsed, selection, EditIntent::Replace(escaped));
        }
    }
    if matches!(&intent, EditIntent::InsertCodeBlock(_) | EditIntent::InsertMathBlock | EditIntent::InsertDiagram(_)) {
        if let Some(result) = super::semantic_selection::wrap_formatted(parsed, selection, range.clone(), &intent)? {
            return Ok(result);
        }
    }
    let Some(cell) = parsed
        .html_tables
        .iter()
        .find_map(|table| table.cell_at(range.start))
        .filter(|cell| range.end <= cell.content.end)
    else {
        if let Some(result) = super::list_commands::apply(parsed, selection, &intent) {
            return result;
        }
        if let Some(result) = delete_paragraph_boundary(parsed, range, &intent) {
            return Ok(result);
        }
        return apply(parsed, selection, intent);
    };
    let result = |range: Range<usize>, text: String, caret: usize| EditResult {
        template_update: None,
        edits: vec![SourceEdit {
            from_byte: base + range.start as u64,
            to_byte: base + range.end as u64,
            inserted_text: text,
        }],
        selection: DocumentSelection::caret(base + caret as u64),
    };
    let result = match intent {
        EditIntent::Replace(text) => {
            let (text, caret) = html_replacement(parsed, range.clone(), &text)?;
            Ok(result(range, text, caret))
        }
        EditIntent::SoftBreak | EditIntent::Newline => {
            Ok(result(range.clone(), "<br>".into(), range.start + 4))
        }
        EditIntent::ParagraphBreak => {
            let active: Vec<_> = cell
                .spans
                .iter()
                .filter(|span| span.content.start <= range.start && range.start <= span.content.end)
                .collect();
            let mut text = String::new();
            for span in active.iter().rev() {
                text.push_str(&parsed.source[span.close.clone()]);
            }
            text.push_str("</p><p>");
            for span in active {
                text.push_str(&parsed.source[span.open.clone()]);
            }
            let caret = range.start + text.len();
            Ok(result(range, text, caret))
        }
        EditIntent::Backspace | EditIntent::Delete if range.is_empty() => {
            let backwards = matches!(intent, EditIntent::Backspace);
            let run = if backwards {
                cell.runs
                    .iter()
                    .rev()
                    .find(|run| run.source.start < range.start && range.start <= run.source.end)
            } else {
                cell.runs
                    .iter()
                    .find(|run| run.source.start <= range.start && range.start < run.source.end)
            };
            let Some(run) = run else {
                return Ok(EditResult {
                    template_update: None,
                    edits: Vec::new(),
                    selection,
                });
            };
            if parsed.source.get(run.source.clone()) != Some(run.text.as_str()) {
                return Ok(result(run.source.clone(), String::new(), run.source.start));
            }
            apply(parsed, selection, intent)
        }
        EditIntent::Backspace | EditIntent::Delete => {
            let (text, caret) = html_replacement(parsed, range.clone(), "")?;
            Ok(result(range, text, caret))
        }
        _ => apply(parsed, selection, intent),
    }?;
    if !large_text { validate_html_edits(parsed, &result.edits)?; }
    Ok(result)
}

/// 语义段落用两个原文换行分隔，删除时作为一个边界处理。
fn delete_paragraph_boundary(parsed: &ParsedWindow, range: Range<usize>, intent: &EditIntent) -> Option<EditResult> {
    if !range.is_empty() || !matches!(intent, EditIntent::Backspace | EditIntent::Delete) { return None; }
    let backward = matches!(intent, EditIntent::Backspace);
    let source = &parsed.source;
    let at = range.start;
    let content_end = |node: &super::parse::SemanticNode| {
        node.source.start + source[node.source.clone()].trim_end_matches(['\r', '\n']).len()
    };
    let blank = parsed.blank_paragraphs.partition_point(|anchor| *anchor <= at).checked_sub(1)
        .map(|index| parsed.blank_paragraphs[index])
        .filter(|anchor| matches!(&source[*anchor..at], "" | "\n" | "\r\n"));
    let boundary = if let Some(blank) = blank {
        blank
    } else {
        parsed.nodes.iter().find_map(|node| {
            if node.parent.is_some() || !matches!(node.kind, "paragraph" | "heading")
                || (backward && node.kind != "paragraph") || at < node.source.start || at > content_end(node)
            { return None; }
            let end = if backward { node.source.start } else { content_end(node) };
            let mut cursor = if backward { end } else { at };
            let target = if backward { at } else { end };
            // 行首/行尾可能隔着隐藏的行内定界符，只移除块间分隔，不破坏格式。
            while cursor < target {
                let span = parsed.inline.iter().find_map(|span| {
                    let marker = if backward { &span.open } else { &span.close };
                    (marker.start == cursor && marker.end > cursor && marker.end <= target).then_some(marker)
                })?;
                cursor = span.end;
            }
            (cursor == target).then_some(end)
        })?
    };
    let mut cut = boundary..boundary;
    for index in 0..2 {
        if backward {
            let prefix = source[..cut.start].strip_suffix('\n')?;
            cut.start = prefix.strip_suffix('\r').unwrap_or(prefix).len();
            if index == 0 { cut.start = source[..cut.start].trim_end_matches([' ', '\t']).len(); }
        } else {
            let suffix = source[cut.end..].strip_prefix("\r\n").or_else(|| source[cut.end..].strip_prefix('\n'))?;
            cut.end = source.len() - suffix.len();
            if index == 0 { cut.end += source[cut.end..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count(); }
        }
    }
    let other = if backward { cut.start } else { cut.end };
    if parsed.blank_paragraphs.binary_search(&other).is_err() && !parsed.nodes.iter().any(|node| {
        node.parent.is_none() && if backward {
            matches!(node.kind, "paragraph" | "heading") && content_end(node) == other
        } else {
            node.kind == "paragraph" && node.source.start == other
        }
    }) { return None; }
    let base = parsed.identity.from_byte;
    Some(EditResult {
        edits: vec![SourceEdit { from_byte: base + cut.start as u64, to_byte: base + cut.end as u64, inserted_text: String::new() }],
        selection: DocumentSelection::caret(base + if backward { cut.start } else { at.min(boundary) } as u64),
        template_update: None,
    })
}

pub(crate) fn html_text_boundary(parsed: &ParsedWindow, cell: &super::html_table::Cell, at: usize) -> bool {
    at == cell.content.start || at == cell.content.end
        || cell.spans.iter().any(|span| [span.open.start, span.open.end, span.close.start, span.close.end].contains(&at))
        || cell.runs.iter().any(|run| {
            at == run.source.start || at == run.source.end
                || (run.source.start < at && at < run.source.end
                    && parsed.source.get(run.source.clone()) == Some(run.text.as_str()))
        })
}

pub fn html_replacement(
    parsed: &ParsedWindow,
    range: Range<usize>,
    text: &str,
) -> Result<(String, usize), ControlError> {
    let cell = parsed
        .html_tables
        .iter()
        .find_map(|table| table.cell_at(range.start))
        .filter(|cell| range.end <= cell.content.end)
        .ok_or(ControlError::InvalidRange)?;
    let mut inserted = html_escape::encode_text(text).into_owned();
    let caret = range.start + inserted.len();
    let large_text = text.len() > super::session_window::policy().group_bytes as usize;
    if large_text {
        if !html_text_boundary(parsed, cell, range.start) || !html_text_boundary(parsed, cell, range.end) {
            return Err(ControlError::Unsupported("大段替换不能截断 HTML 标记或实体；原文已保留".into()));
        }
    }
    // 大文本已经转义，只用占位文字验证标签结构，避免临时拼出并解析扩张后的整张表。
    let mut probe = large_text.then(|| "x".to_string());
    let left: Vec<_> = cell
        .spans
        .iter()
        .filter(|span| span.content.start <= range.start && range.start <= span.content.end)
        .collect();
    let right: Vec<_> = cell
        .spans
        .iter()
        .filter(|span| span.content.start <= range.end && range.end <= span.content.end)
        .collect();
    for span in left.iter().rev().filter(|span| !right.contains(span)) {
        inserted.push_str(&parsed.source[span.close.clone()]);
        if let Some(probe) = &mut probe { probe.push_str(&parsed.source[span.close.clone()]); }
    }
    for span in right.iter().filter(|span| !left.contains(span)) {
        inserted.push_str(&parsed.source[span.open.clone()]);
        if let Some(probe) = &mut probe { probe.push_str(&parsed.source[span.open.clone()]); }
    }
    if let Some(probe) = probe {
        validate_html_edits(parsed, &[SourceEdit {
            from_byte: parsed.identity.from_byte + range.start as u64,
            to_byte: parsed.identity.from_byte + range.end as u64, inserted_text: probe,
        }])?;
    }
    Ok((inserted, caret))
}
/// 语义交互不得把已有表格变成残缺 HTML；源码模式保留显式修改原文的权限。
pub fn validate_html_edits(
    parsed: &ParsedWindow,
    edits: &[SourceEdit],
) -> Result<(), ControlError> {
    let base = parsed.identity.from_byte;
    for table in &parsed.html_tables {
        let inside: Vec<_> = edits
            .iter()
            .filter(|edit| {
                edit.from_byte >= base + table.source.start as u64
                    && edit.to_byte <= base + table.source.end as u64
            })
            .collect();
        if inside.is_empty()
            || inside.iter().any(|edit| {
                edit.from_byte == base + table.source.start as u64
                    && edit.to_byte == base + table.source.end as u64
            })
        {
            continue;
        }
        let mut text = parsed.source[table.source.clone()].to_string();
        for edit in inside.into_iter().rev() {
            let start = (edit.from_byte - base) as usize - table.source.start;
            let end = (edit.to_byte - base) as usize - table.source.start;
            text.replace_range(start..end, &edit.inserted_text);
        }
        if super::html_table::HtmlTable::parse(&text, 0..text.len()).is_none() {
            return Err(ControlError::Unsupported(
                "跨越 HTML 表格结构的此项操作；原文已保留".into(),
            ));
        }
    }
    Ok(())
}

pub fn apply(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    intent: EditIntent,
) -> Result<EditResult, ControlError> {
    let base = parsed.identity.from_byte;
    let range = local_range(&parsed.source, base, selection.range())?;
    let source = &*parsed.source;
    if let Some(result) = super::template_projection::command(parsed, selection, &intent) {
        return Ok(result);
    }
    let replace = |range: Range<usize>, text: String, caret: usize| EditResult {
        template_update: None,
        edits: vec![SourceEdit {
            from_byte: base + range.start as u64,
            to_byte: base + range.end as u64,
            inserted_text: text,
        }],
        selection: DocumentSelection::caret(base + caret as u64),
    };
    match intent {
        EditIntent::Replace(text) => {
            let caret = range.start + text.len();
            Ok(replace(range, text, caret))
        }
        EditIntent::ToggleInline(format) => {
            if !range.is_empty() && format != InlineFormat::Code {
                return super::inline_commands::selected(parsed, selection, Some(format));
            }
            if parsed
                .nodes
                .iter()
                .any(|node| node.kind == "code" && node.source.contains(&range.start))
            {
                return Ok(EditResult {
                    template_update: None,
                    edits: Vec::new(),
                    selection,
                });
            }
            if range.is_empty() {
                if let Some(other) = super::inline_commands::opposite(format) {
                    if let Some(span) = parsed
                        .inline
                        .iter()
                        .find(|span| span.format == other && span.intersects(&range))
                    {
                        let (open, close) = if parsed.source[span.content.clone()]
                            .chars()
                            .any(char::is_whitespace)
                        {
                            super::inline_commands::html_pair(format)
                        } else {
                            let (a, b) = format.markers();
                            (a.to_string(), b.to_string())
                        };
                        let edits = vec![
                            SourceEdit {
                                from_byte: base + span.open.start as u64,
                                to_byte: base + span.open.end as u64,
                                inserted_text: open,
                            },
                            SourceEdit {
                                from_byte: base + span.close.start as u64,
                                to_byte: base + span.close.end as u64,
                                inserted_text: close,
                            },
                        ];
                        return Ok(EditResult {
                            template_update: None,
                            selection: map_selection(selection, &edits),
                            edits,
                        });
                    }
                }
            }
            if let Some(span) = parsed
                .inline
                .iter()
                .filter(|s| {
                    s.format == format && s.open.start <= range.start && s.close.end >= range.end
                })
                .min_by_key(|s| s.range().len())
            {
                if format == InlineFormat::Code {
                    if !range.is_empty()
                        && (range.start > span.content.start || range.end < span.content.end)
                    {
                        return super::inline_commands::release_code_selection(
                            parsed, selection, span,
                        );
                    }
                    let edits = super::inline_commands::release_code(parsed, span);
                    return Ok(EditResult {
                        template_update: None,
                        selection: map_selection(selection, &edits),
                        edits,
                    });
                }
                let edits = vec![
                    SourceEdit {
                        from_byte: base + span.open.start as u64,
                        to_byte: base + span.open.end as u64,
                        inserted_text: String::new(),
                    },
                    SourceEdit {
                        from_byte: base + span.close.start as u64,
                        to_byte: base + span.close.end as u64,
                        inserted_text: String::new(),
                    },
                ];
                return Ok(EditResult {
                    template_update: None,
                    selection: map_selection(selection, &edits),
                    edits,
                });
            }
            let (mut open, mut close) = {
                let (a, b) = format.markers();
                (a.to_string(), b.to_string())
            };
            let in_html_cell = parsed.html_tables.iter().any(|table| {
                table
                    .cell_at(range.start)
                    .is_some_and(|cell| range.end <= cell.content.end)
            });
            if in_html_cell {
                let tag = match format {
                    InlineFormat::Strong => "strong",
                    InlineFormat::Emphasis => "em",
                    InlineFormat::Code => "code",
                    InlineFormat::Strike => "s",
                    InlineFormat::Underline => "u",
                    InlineFormat::Highlight => "mark",
                    InlineFormat::Superscript => "sup",
                    InlineFormat::Subscript => "sub",
                    InlineFormat::Link => "a",
                };
                open = format!("<{tag}>");
                close = format!("</{tag}>");
            }
            if format == InlineFormat::Code && !in_html_cell {
                let ticks = source[range.clone()]
                    .split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    + 1;
                open = "`".repeat(ticks);
                close = open.clone();
                if source[range.clone()].starts_with('`') || source[range.clone()].ends_with('`') {
                    open.push(' ');
                    close.insert(0, ' ');
                }
            }
            let template =
                (range.is_empty() && format != InlineFormat::Link).then(|| InlineTemplate {
                    format,
                    from_byte: base + range.start as u64,
                    open_end_byte: base + (range.start + open.len()) as u64,
                    close_start_byte: base + (range.start + open.len()) as u64,
                    to_byte: base + (range.start + open.len() + close.len()) as u64,
                    html: in_html_cell,
                });
            let text = format!("{open}{}{close}", &source[range.clone()]);
            let anchor = base + (range.start + open.len()) as u64;
            let head = anchor + range.len() as u64;
            let mut result = replace(range, text, 0);
            result.template_update = template.map(|t| TemplateUpdate { add: vec![t] });
            result.selection = if selection.anchor_byte <= selection.head_byte {
                DocumentSelection {
                    anchor_byte: anchor,
                    head_byte: head,
                }
            } else {
                DocumentSelection {
                    anchor_byte: head,
                    head_byte: anchor,
                }
            };
            Ok(result)
        }
        EditIntent::ClearFormatting => {
            if !range.is_empty() {
                return super::inline_commands::selected(parsed, selection, None);
            }
            let mut edits: Vec<_> = parsed
                .inline
                .iter()
                .filter(|span| span.intersects(&range))
                .flat_map(|span| {
                    if span.format == InlineFormat::Code {
                        super::inline_commands::release_code(parsed, span)
                    } else {
                        [span.open.clone(), span.close.clone()]
                            .into_iter()
                            .map(|r| SourceEdit {
                                from_byte: base + r.start as u64,
                                to_byte: base + r.end as u64,
                                inserted_text: String::new(),
                            })
                            .collect()
                    }
                })
                .collect();
            edits.sort_by_key(|e| (e.from_byte, e.to_byte));
            edits.dedup();
            Ok(EditResult {
                template_update: (!parsed.context.inline_templates.is_empty())
                    .then(TemplateUpdate::default),
                selection: map_selection(selection, &edits),
                edits,
            })
        }
        EditIntent::Heading(level) => {
            if level > 6 {
                return Err(ControlError::InvalidRange);
            }
            let lines = selected_lines(source, range);
            let remove = level > 0 && lines.iter().all(|line| {
                let raw = &source[line.clone()];
                let hashes = raw.bytes().take_while(|byte| *byte == b'#').count();
                hashes == level as usize && raw.as_bytes().get(hashes) == Some(&b' ')
            });
            let mut edits = Vec::new();
            for line in lines {
                let raw = &source[line.clone()];
                let hashes = raw.bytes().take_while(|b| *b == b'#').count();
                let old = if hashes > 0 && hashes <= 6 && raw.as_bytes().get(hashes) == Some(&b' ')
                {
                    hashes + 1
                } else {
                    0
                };
                edits.push(SourceEdit {
                    from_byte: base + line.start as u64,
                    to_byte: base + (line.start + old) as u64,
                    inserted_text: if level == 0 || remove {
                        String::new()
                    } else {
                        format!("{} ", "#".repeat(level as usize))
                    },
                });
            }
            Ok(EditResult {
                template_update: None,
                selection: map_selection(selection, &edits),
                edits,
            })
        }
        EditIntent::LinePrefix(prefix) => {
            let lines = selected_lines(source, range);
            if let Some(target) = list_marker(&prefix).map(|(_, kind)| kind) {
                let rows: Vec<_> = lines
                    .into_iter()
                    .filter(|line| {
                        !parsed
                            .nodes
                            .iter()
                            .any(|node| node.kind == "code" && node.source.contains(&line.start))
                    })
                    .map(|line| {
                        let raw = &source[line.clone()];
                        let outer = outer_prefix(raw);
                        let marker = list_marker(&raw[outer..]);
                        (line, outer, marker)
                    })
                    .collect();
                let remove = rows
                    .iter()
                    .all(|(_, _, marker)| marker.is_some_and(|(_, kind)| kind == target));
                let edits: Vec<_> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(index, (line, outer, marker))| SourceEdit {
                        from_byte: base + (line.start + outer) as u64,
                        to_byte: base
                            + (line.start + outer + marker.map_or(0, |(len, _)| len)) as u64,
                        inserted_text: if remove {
                            String::new()
                        } else if target == ListKind::Ordered {
                            format!("{}. ", index + 1)
                        } else {
                            prefix.clone()
                        },
                    })
                    .collect();
                return Ok(EditResult {
                    template_update: None,
                    selection: map_selection(selection, &edits),
                    edits,
                });
            }
            let remove = lines
                .iter()
                .all(|line| source[line.clone()].starts_with(&prefix));
            let edits: Vec<_> = lines
                .into_iter()
                .map(|line| SourceEdit {
                    from_byte: base + line.start as u64,
                    to_byte: base + (line.start + if remove { prefix.len() } else { 0 }) as u64,
                    inserted_text: if remove {
                        String::new()
                    } else {
                        prefix.clone()
                    },
                })
                .collect();
            Ok(EditResult {
                template_update: None,
                selection: map_selection(selection, &edits),
                edits,
            })
        }
        EditIntent::Indent { outdent } => {
            let edits: Vec<_> = selected_lines(source, range)
                .into_iter()
                .map(|line| {
                    let text = &source[line.clone()];
                    let trim = if !outdent {
                        0
                    } else if text.starts_with('\t') {
                        1
                    } else {
                        text.bytes().take_while(|c| *c == b' ').count().min(2)
                    };
                    SourceEdit {
                        from_byte: base + line.start as u64,
                        to_byte: base + (line.start + trim) as u64,
                        inserted_text: if outdent { String::new() } else { "  ".into() },
                    }
                })
                .collect();
            Ok(EditResult {
                template_update: None,
                selection: map_selection(selection, &edits),
                edits,
            })
        }
        EditIntent::Newline | EditIntent::ParagraphBreak | EditIntent::SoftBreak => {
            let start = line_start(source, range.start);
            let before = &source[start..range.start];
            let ending = line_ending(source, range.start);
            let literal = parsed
                .nodes
                .iter()
                .any(|node| node.kind == "code" && node.source.contains(&range.start));
            if matches!(intent, EditIntent::SoftBreak) {
                let text = if literal {
                    ending.to_string()
                } else {
                    // 行尾空格在段尾会被 Markdown 忽略；HTML 换行能保留空行和连续换行。
                    "<br>".into()
                };
                let caret = range.start + text.len();
                return Ok(replace(range, text, caret));
            }
            let prefix = if literal {
                before[..before.len() - before.trim_start_matches([' ', '\t']).len()].into()
            } else {
                continuation(before)
            };
            if !literal
                && structural_prefix_len(before)
                    .is_some_and(|length| before[length..].trim().is_empty())
            {
                return Ok(replace(start..range.end, String::new(), start));
            }
            let text = if matches!(intent, EditIntent::ParagraphBreak)
                && !literal
                && structural_prefix_len(before).is_none()
            {
                format!("{ending}{ending}{prefix}")
            } else {
                format!("{ending}{prefix}")
            };
            let caret = range.start + text.len();
            Ok(replace(range, text, caret))
        }
        EditIntent::Backspace | EditIntent::Delete => {
            let backward = matches!(intent, EditIntent::Backspace);
            let mut cut = range;
            if cut.is_empty() {
                if backward {
                    if cut.start == 0 {
                        return Err(ControlError::NeedsContext("需要前一窗口以继续删除".into()));
                    }
                    let start = line_start(source, cut.start);
                    if !parsed
                        .nodes
                        .iter()
                        .any(|node| node.kind == "code" && node.source.contains(&cut.start))
                        && structural_prefix_len(&source[start..cut.start])
                            == Some(cut.start - start)
                    {
                        return Ok(replace(start..cut.end, String::new(), start));
                    }
                    cut.start = source[..cut.start]
                        .grapheme_indices(true)
                        .last()
                        .map(|(at, _)| at)
                        .unwrap_or(0);
                } else {
                    if cut.end == source.len() {
                        return Err(ControlError::NeedsContext("需要后一窗口以继续删除".into()));
                    }
                    cut.end += source[cut.end..]
                        .graphemes(true)
                        .next()
                        .map(str::len)
                        .unwrap_or(0);
                }
            }
            let caret = cut.start;
            Ok(replace(cut, String::new(), caret))
        }
        EditIntent::InsertCodeBlock(language) => {
            insert_wrapped_block(parsed, selection, range, Some(&language), "", true)
        }
        EditIntent::InsertMathBlock => insert_wrapped_block(parsed, selection, range, None, "E = mc^2", true),
        EditIntent::InsertDetails => insert_details(parsed, range),
        EditIntent::InsertDiagram(template) => insert_wrapped_block(parsed, selection, range, Some("mermaid"), &template, false),
        EditIntent::InsertBlockAt { block, caret } => {
            let prefix = block.get(..caret).ok_or(ControlError::InvalidRange)?;
            let eol = line_ending(source, range.start);
            let normalized = block.replace('\n', eol);
            let caret_in_block = prefix.replace('\n', eol).len();
            let mut result = apply(parsed, selection, EditIntent::InsertBlock(block))?;
            let edit = result.edits.first().ok_or(ControlError::InvalidRange)?;
            let start = edit.inserted_text.find(&normalized).ok_or(ControlError::InvalidRange)?;
            result.selection = DocumentSelection::caret(edit.from_byte + (start + caret_in_block) as u64);
            Ok(result)
        }
        EditIntent::InsertBlock(block) => {
            let eol = line_ending(source, range.start);
            let before_source = &source[..range.start];
            let after_source = &source[range.end..];
            let before_breaks = before_source[before_source.trim_end_matches(['\r', '\n']).len()..]
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
            let after_breaks = after_source
                [..after_source.len() - after_source.trim_start_matches(['\r', '\n']).len()]
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
            // 分隔线不能误变成前一段的 Setext 标题，HTML 块也不能吞掉后一段。
            let before = eol.repeat(if before_source.is_empty() {
                0
            } else {
                2usize.saturating_sub(before_breaks)
            });
            let after = eol.repeat(if after_source.is_empty() {
                0
            } else {
                2usize.saturating_sub(after_breaks)
            });
            let text = format!("{before}{}{after}", block.replace('\n', eol));
            let caret = range.start + text.len();
            Ok(replace(range, text, caret))
        }
        EditIntent::InsertParagraph { before } => {
            if !parsed.context.ancestors.is_empty() {
                return Err(ControlError::NeedsContext(
                    "在片段外层容器前后插入段落需要宿主定位完整顶层块".into(),
                ));
            }
            let block = parsed
                .nodes
                .iter()
                .filter(|node| node.parent.is_none())
                .find(|node| node.source.start <= range.start && range.start <= node.source.end)
                .map(|node| node.source.clone())
                .unwrap_or(range.start..range.start);
            let at = if before { block.start } else { block.end };
            let eol = line_ending(source, at);
            let text = eol.repeat(2);
            let caret = if before {
                at
            } else if at < source.len() {
                at + eol.len()
            } else {
                at + text.len()
            };
            Ok(replace(at..at, text, caret))
        }
        EditIntent::Link { label, target } => {
            let label = label
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]");
            let target = target
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            let text = format!("[{label}]({target})");
            let caret = range.start + text.len();
            Ok(replace(range, text, caret))
        }
    }
}

pub fn map_selection(selection: DocumentSelection, edits: &[SourceEdit]) -> DocumentSelection {
    let map = |point: u64| {
        let mut shift: i128 = 0;
        for edit in edits {
            if point < edit.from_byte {
                break;
            }
            if point <= edit.to_byte {
                return (edit.from_byte as i128 + shift + edit.inserted_text.len() as i128) as u64;
            }
            shift += edit.inserted_text.len() as i128 - (edit.to_byte - edit.from_byte) as i128;
        }
        (point as i128 + shift) as u64
    };
    DocumentSelection {
        anchor_byte: map(selection.anchor_byte),
        head_byte: map(selection.head_byte),
    }
}
pub fn line_start(source: &str, at: usize) -> usize {
    source[..at].rfind('\n').map_or(0, |p| p + 1)
}
fn insert_details(parsed: &ParsedWindow, range: Range<usize>) -> Result<EditResult, ControlError> {
    let source = &*parsed.source;
    if !parsed.context.ancestors.is_empty() || parsed.nodes.iter().any(|node| matches!(node.kind, "code" | "table" | "html") && node.source.contains(&range.start)) {
        return Err(ControlError::NeedsContext("当前容器内的折叠块需要完整结构；原文已保留".into()));
    }
    let eol = line_ending(source, range.start);
    let before_source = &source[..range.start];
    let after_source = &source[range.end..];
    let before_breaks = before_source[before_source.trim_end_matches(['\r', '\n']).len()..].bytes().filter(|byte| *byte == b'\n').count();
    let after_breaks = after_source[..after_source.len() - after_source.trim_start_matches(['\r', '\n']).len()].bytes().filter(|byte| *byte == b'\n').count();
    let before = if before_source.is_empty() { String::new() } else { eol.repeat(2usize.saturating_sub(before_breaks)) };
    let after = if after_source.is_empty() { String::new() } else { eol.repeat(2usize.saturating_sub(after_breaks)) };
    let header = format!("<details>{eol}<summary></summary>{eol}{eol}");
    let selected = &source[range.clone()];
    let ending = if selected.ends_with('\n') { "" } else { eol };
    let inserted = format!("{before}{header}{selected}{ending}</details>{after}");
    let caret = parsed.identity.from_byte + (range.start + before.len() + "<details>".len() + eol.len() + "<summary>".len()) as u64;
    Ok(EditResult { edits: vec![SourceEdit { from_byte: parsed.identity.from_byte + range.start as u64, to_byte: parsed.identity.from_byte + range.end as u64, inserted_text: inserted }], selection: DocumentSelection::caret(caret), template_update: None })
}

/// 保留选区原文和方向，围栏长度根据正文计算，避免内容中的反引号提前结束代码块。
fn insert_wrapped_block(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    range: Range<usize>,
    language: Option<&str>,
    default_body: &str,
    toggle_existing: bool,
) -> Result<EditResult, ControlError> {
    let source = &*parsed.source;
    let base = parsed.identity.from_byte;
    let in_code = parsed.nodes.iter().any(|node| {
        node.kind == "code" && node.source.start <= range.start && range.start < node.source.end
    });
    let in_math = parsed.events.iter().any(|(event, span)| {
        matches!(event, pulldown_cmark::Event::DisplayMath(_))
            && span.start <= range.start && range.start < span.end
    });
    if in_code && language.is_some() && toggle_existing {
        let node = parsed.nodes.iter().find(|node| node.kind == "code" && node.source.start <= range.start && range.start < node.source.end).unwrap();
        if node.parent.is_some() || !parsed.context.ancestors.is_empty() || range.end > node.source.end {
            return Err(ControlError::NeedsContext("嵌套或跨窗口代码块需要完整结构；原文已保留".into()));
        }
        return unwrap_fenced_block(parsed, selection, node.source.clone(), true);
    }
    if in_math && language.is_none() && toggle_existing {
        let span = parsed.events.iter().find_map(|(event, span)| {
            (matches!(event, pulldown_cmark::Event::DisplayMath(_)) && span.start <= range.start && range.start < span.end).then(|| span.clone())
        }).unwrap();
        if !parsed.context.ancestors.is_empty() || range.end > span.end {
            return Err(ControlError::NeedsContext("嵌套或跨窗口公式块需要完整结构；原文已保留".into()));
        }
        return unwrap_fenced_block(parsed, selection, span, false);
    }
    if in_code || in_math {
        return Err(ControlError::Unsupported("不能在代码块或公式块中插入另一种块；原文已保留".into()));
    }
    let eol = line_ending(source, range.start);
    let before_source = &source[..range.start];
    let after_source = &source[range.end..];
    let before_breaks = before_source[before_source.trim_end_matches(['\r', '\n']).len()..]
        .bytes().filter(|byte| *byte == b'\n').count();
    let after_breaks = after_source[..after_source.len() - after_source.trim_start_matches(['\r', '\n']).len()]
        .bytes().filter(|byte| *byte == b'\n').count();
    let before = eol.repeat(if before_source.is_empty() { 0 } else { 2usize.saturating_sub(before_breaks) });
    let after = eol.repeat(if after_source.is_empty() { 0 } else { 2usize.saturating_sub(after_breaks) });
    let selected = &source[range.clone()];
    let body = if selected.is_empty() { default_body } else { selected };
    if language.is_none() && body.contains("$$") { return Err(ControlError::Unsupported("选区包含公式块围栏；原文已保留".into())); }
    let (opening, closing) = if let Some(language) = language {
        let ticks = body.split(|ch| ch != '`').map(str::len).max().unwrap_or(0).saturating_add(1).max(3);
        let fence = "`".repeat(ticks);
        let language = if language.len() <= 32 && language.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_+#.-".contains(&byte)) { language } else { "" };
        (format!("{fence}{language}{eol}"), fence)
    } else {
        (format!("$${eol}"), "$$".to_string())
    };
    let ending = if body.ends_with('\n') { "" } else { eol };
    if body.len().saturating_add(opening.len()).saturating_add(ending.len()).saturating_add(closing.len()) > crate::text_document::MAX_SEMANTIC_BYTES {
        return Err(ControlError::NeedsContext("转换后的块超过语义窗口预算；选区原文已保留".into()));
    }
    let inserted = format!("{before}{opening}{body}{ending}{closing}{after}");
    let content_start = base + (range.start + before.len() + opening.len()) as u64;
    let content_end = content_start + body.len() as u64;
    let selected_after = if range.is_empty() {
        DocumentSelection::caret(content_start)
    } else if selection.anchor_byte <= selection.head_byte {
        DocumentSelection { anchor_byte: content_start, head_byte: content_end }
    } else {
        DocumentSelection { anchor_byte: content_end, head_byte: content_start }
    };
    Ok(EditResult {
        edits: vec![SourceEdit { from_byte: base + range.start as u64, to_byte: base + range.end as u64, inserted_text: inserted }],
        selection: selected_after,
        template_update: None,
    })
}

/// 仅在当前窗口包含完整围栏时拆除；不读取或复制窗口外的正文。
fn unwrap_fenced_block(parsed: &ParsedWindow, selection: DocumentSelection, span: Range<usize>, code: bool) -> Result<EditResult, ControlError> {
    let raw = &parsed.source[span.clone()];
    let trimmed = raw.trim_end_matches(['\r', '\n']);
    let suffix = &raw[trimmed.len()..];
    let (mut content_start, mut content_end) = if code {
        let first_end = trimmed.find('\n').ok_or_else(|| ControlError::NeedsContext("代码围栏不完整；原文已保留".into()))? + 1;
        let close_start = trimmed.rfind('\n').map(|at| at + 1).ok_or_else(|| ControlError::NeedsContext("代码围栏不完整；原文已保留".into()))?;
        let opening = trimmed[..first_end].trim();
        let closing = trimmed[close_start..].trim();
        let character = opening.chars().next().unwrap_or(' ');
        let length = opening.chars().take_while(|ch| *ch == character).count();
        if !matches!(character, '`' | '~') || length < 3 || closing.len() < length || !closing.chars().all(|ch| ch == character) || close_start < first_end {
            return Err(ControlError::NeedsContext("代码围栏不完整；原文已保留".into()));
        }
        (first_end, close_start)
    } else {
        if !trimmed.starts_with("$$") || !trimmed.ends_with("$$") || trimmed.len() < 4 {
            return Err(ControlError::NeedsContext("公式围栏不完整；原文已保留".into()));
        }
        (2, trimmed.len() - 2)
    };
    if !code && raw[content_start..].starts_with("\r\n") { content_start += 2; }
    else if !code && raw[content_start..].starts_with('\n') { content_start += 1; }
    if raw[..content_end].ends_with("\r\n") { content_end -= 2; }
    else if raw[..content_end].ends_with('\n') { content_end -= 1; }
    if content_end < content_start { content_end = content_start; }
    let content = &raw[content_start..content_end];
    let inserted = format!("{content}{suffix}");
    let base = parsed.identity.from_byte;
    let body_start = base + (span.start + content_start) as u64;
    let mapped = |position: u64| base + span.start as u64 + position.saturating_sub(body_start).min(content.len() as u64);
    Ok(EditResult {
        edits: vec![SourceEdit { from_byte: base + span.start as u64, to_byte: base + span.end as u64, inserted_text: inserted }],
        selection: DocumentSelection { anchor_byte: mapped(selection.anchor_byte), head_byte: mapped(selection.head_byte) },
        template_update: None,
    })
}

pub fn line_ending(source: &str, at: usize) -> &str {
    if let Some(end) = source[at..].find('\n') {
        if at + end > 0 && source.as_bytes()[at + end - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else if let Some(end) = source[..at].rfind('\n') {
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else {
        "\n"
    }
}
fn selected_lines(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut start = line_start(source, range.start);
    let mut lines = Vec::new();
    loop {
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |p| start + p);
        let content_end = if end > start && source.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        lines.push(start..content_end);
        if end == source.len() || end + 1 >= range.end {
            break;
        }
        start = end + 1;
    }
    lines
}
fn continuation(before: &str) -> String {
    let indent = &before[..before.len() - before.trim_start_matches([' ', '\t']).len()];
    let rest = &before[indent.len()..];
    if let Some(quoted) = rest.strip_prefix("> ") {
        return format!("{indent}> {}", continuation(quoted));
    }
    for marker in [
        "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] ", "+ [ ] ", "+ [x] ", "+ [X] ",
    ] {
        if rest.starts_with(marker) {
            return format!("{indent}{}[ ] ", &marker[..2]);
        }
    }
    for marker in ["- ", "* ", "+ ", "> "] {
        if rest.starts_with(marker) {
            return format!("{indent}{marker}");
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
        && rest.as_bytes().get(digits + 1) == Some(&b' ')
    {
        if let Ok(number) = rest[..digits].parse::<u64>() {
            return format!(
                "{indent}{}{} ",
                number.saturating_add(1),
                &rest[digits..digits + 1]
            );
        }
    }
    indent.into()
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Ordered,
    Task,
}
fn list_marker(line: &str) -> Option<(usize, ListKind)> {
    if [
        "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] ", "+ [ ] ", "+ [x] ", "+ [X] ",
    ]
    .iter()
    .any(|p| line.starts_with(p))
    {
        return Some((6, ListKind::Task));
    }
    if ["- ", "* ", "+ "].iter().any(|p| line.starts_with(p)) {
        return Some((2, ListKind::Bullet));
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && matches!(line.as_bytes().get(digits), Some(b'.' | b')'))
        && line.as_bytes().get(digits + 1) == Some(&b' ')
    {
        Some((digits + 2, ListKind::Ordered))
    } else {
        None
    }
}
fn outer_prefix(line: &str) -> usize {
    let mut offset = 0;
    loop {
        offset += line[offset..].len() - line[offset..].trim_start_matches([' ', '\t']).len();
        if line[offset..].starts_with("> ") {
            offset += 2;
        } else {
            return offset;
        }
    }
}
pub(crate) fn structural_prefix_len(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    for prefix in [
        "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] ", "+ [ ] ", "+ [x] ", "+ [X] ",
        "- ", "* ", "+ ", "> ",
    ] {
        if rest.starts_with(prefix) {
            return Some(indent + prefix.len());
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
        && rest.as_bytes().get(digits + 1) == Some(&b' ')
    {
        return Some(indent + digits + 2);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nomo_mark::parse::fixture;
    fn edited(parsed: &ParsedWindow, edits: &[SourceEdit]) -> String {
        let mut source = parsed.source.to_string();
        for edit in edits.iter().rev() {
            source.replace_range(
                (edit.from_byte - 100) as usize..(edit.to_byte - 100) as usize,
                &edit.inserted_text,
            );
        }
        source
    }
    #[test]
    fn toggles_preserve_nested_markup_and_unmodified_line_endings() {
        let parsed = fixture("a\r\n**粗*斜***\nb");
        let at = 100 + parsed.source.find('斜').unwrap() as u64;
        let result = apply(
            &parsed,
            DocumentSelection::caret(at),
            EditIntent::ToggleInline(InlineFormat::Strong),
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "a\r\n粗*斜*\nb");
        assert_eq!(result.selection.head_byte, at - 2);
    }
    #[test]
    fn empty_template_and_grapheme_delete_do_not_split_unicode() {
        let parsed = fixture("a👨‍👩‍👧‍👦");
        let result = apply(
            &parsed,
            DocumentSelection::caret(100 + parsed.source.len() as u64),
            EditIntent::Backspace,
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "a");
        let result = apply(
            &parsed,
            DocumentSelection::caret(101),
            EditIntent::ToggleInline(InlineFormat::Strong),
        )
        .unwrap();
        assert_eq!(result.selection.head_byte, 103);
        assert_eq!(edited(&parsed, &result.edits), "a****👨‍👩‍👧‍👦");
    }
    #[test]
    fn newline_uses_local_ending_and_resets_task_checkbox() {
        let parsed = fixture("- [x] 完成\r\nnext\n");
        let at = parsed.source.find('\r').unwrap();
        let result = apply(
            &parsed,
            DocumentSelection::caret(100 + at as u64),
            EditIntent::Newline,
        )
        .unwrap();
        assert_eq!(
            edited(&parsed, &result.edits),
            "- [x] 完成\r\n- [ ] \r\nnext\n"
        );
    }
    #[test]
    fn empty_checked_and_ordered_items_exit_without_affecting_code() {
        for source in ["- [x] ", "12. ", "* [X] "] {
            let parsed = fixture(source);
            let result = apply(
                &parsed,
                DocumentSelection::caret(100 + source.len() as u64),
                EditIntent::Newline,
            )
            .unwrap();
            assert_eq!(edited(&parsed, &result.edits), "");
        }
        let parsed = fixture("```text\n- [x] \n```\n");
        let at = parsed.source.find("\n```").unwrap();
        let result = apply(
            &parsed,
            DocumentSelection::caret(100 + at as u64),
            EditIntent::Newline,
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "```text\n- [x] \n\n```\n");
    }
    #[test]
    fn semantic_paragraph_and_html_text_edits_keep_valid_source() {
        let parsed = fixture("中文\r\n");
        let result = apply_semantic(
            &parsed,
            DocumentSelection::caret(106),
            EditIntent::ParagraphBreak,
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "中文\r\n\r\n\r\n");
        let html = "<table data-nomo-table=\"semantic\"><tbody><tr><td><p><strong>abc</strong> xyz</p></td></tr></tbody></table>";
        let parsed = fixture(html);
        let from = html.find("bc").unwrap();
        let to = html.find("yz").unwrap();
        let result = apply_semantic(
            &parsed,
            DocumentSelection {
                anchor_byte: 100 + from as u64,
                head_byte: 100 + to as u64,
            },
            EditIntent::Replace("中<&".into()),
        )
        .unwrap();
        let output = edited(&parsed, &result.edits);
        assert!(output.contains("<strong>a中&lt;&amp;</strong>yz"));
        assert_eq!(fixture(&output).html_tables.len(), 1);
        let at = parsed.html_tables[0].cells[0].runs[0].source.start;
        assert!(
            apply_semantic(
                &parsed,
                DocumentSelection::caret(100 + at as u64),
                EditIntent::Backspace
            )
            .unwrap()
            .edits
            .is_empty()
        );
    }
    #[test]
    fn paragraph_insertion_keeps_existing_blocks_and_eol() {
        for before in [true, false] {
            let parsed = fixture("# 标题\r\n\r\n正文\r\n");
            let result = apply(
                &parsed,
                DocumentSelection::caret(102),
                EditIntent::InsertParagraph { before },
            )
            .unwrap();
            let output = edited(&parsed, &result.edits);
            assert_eq!(result.edits.len(), 1);
            assert_eq!(result.edits[0].from_byte, result.edits[0].to_byte);
            assert_eq!(result.edits[0].inserted_text, "\r\n\r\n");
            assert!(
                fixture(&output)
                    .blank_paragraphs
                    .contains(&((result.selection.head_byte - 100) as usize))
            );
            assert!(output.contains("# 标题\r\n"));
            assert!(output.ends_with("正文\r\n"));
        }
    }
    #[test]
    fn list_type_switches_replace_markers_and_keep_quotes_and_line_endings() {
        let text = "> - [x] one\r\n> - [ ] two\n";
        let parsed = fixture(text);
        let result = apply(
            &parsed,
            DocumentSelection {
                anchor_byte: 100,
                head_byte: 100 + text.len() as u64,
            },
            EditIntent::LinePrefix("1. ".into()),
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "> 1. one\r\n> 2. two\n");
        let parsed = fixture("> 1. one\r\n> 2. two\n");
        let result = apply(
            &parsed,
            DocumentSelection {
                anchor_byte: 100,
                head_byte: 100 + parsed.source.len() as u64,
            },
            EditIntent::LinePrefix("1. ".into()),
        )
        .unwrap();
        assert_eq!(edited(&parsed, &result.edits), "> one\r\n> two\n");
        assert_eq!(continuation("> + [X] done"), "> + [ ] ");
    }
}
