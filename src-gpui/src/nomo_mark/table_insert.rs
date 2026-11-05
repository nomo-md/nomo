//! 插表只生成当前窗口内的一笔补丁，容器前缀来自实际结构而非当前行的外观。
use super::{
    commands::{self, EditIntent, EditResult},
    mapping::local_range,
    parse::ParsedWindow,
    protocol::*,
};
use std::ops::Range;

pub(crate) fn insert(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    rows: usize,
    columns: usize,
    semantic: bool,
) -> Result<EditResult, ControlError> {
    if !(1..=5).contains(&rows) || !(1..=6).contains(&columns) {
        return Err(ControlError::InvalidRange);
    }
    let columns = columns.max(2);
    let line = format!("|{}", "  |".repeat(columns));
    let block = format!("{line}\n|{}{}", " --- |".repeat(columns), format!("\n{line}").repeat(rows));
    let intent = EditIntent::InsertBlockAt { block, caret: 2 };
    if !semantic { return commands::apply(parsed, selection, intent); }

    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range())?;
    if parsed.context.source_only {
        return Err(ControlError::NeedsContext("插表需要完整的当前块结构；原文已保留".into()));
    }
    let intersects = |span: &Range<usize>| if range.is_empty() { span.contains(&range.start) }
        else { span.start < range.end && range.start < span.end };
    if parsed.nodes.iter().any(|node| matches!(node.kind, "code" | "table") && intersects(&node.source))
        || parsed.html_tables.iter().any(|table| intersects(&table.source))
    {
        return Err(ControlError::Unsupported("代码块和表格内不能嵌套插表；原文已保留".into()));
    }
    if parsed.inline.iter().any(|span| span.intersects(&range)
        && !(range.start <= span.open.start && span.close.end <= range.end)
        && !(range.is_empty() && [span.open.start, span.close.end].contains(&range.start)))
    {
        return Err(ControlError::Unsupported("插表不能截断行内格式；原文已保留".into()));
    }
    if let Some(result) = super::html_table::insert_table(parsed, range.clone(), rows, columns) {
        return result;
    }
    // Details 正文是 Markdown；标题和未知 HTML 仍不能按普通文本拆开。
    if parsed.nodes.iter().any(|node| node.kind == "html" && intersects(&node.source)
        && !parsed.details.iter().any(|details| details.source == node.source
            && details.body.start <= range.start && range.end <= details.body.end))
    {
        return Err(ControlError::Unsupported("请在 HTML 容器的正文内插表；原文已保留".into()));
    }
    let prefix = continuation_prefix(parsed, range.start)?;
    let containers = |at: usize| parsed.nodes.iter().enumerate()
        .filter(|(_, node)| matches!(node.kind, "item" | "quote") && contains_caret(parsed, &node.source, at))
        .map(|(index, _)| index).collect::<Vec<_>>();
    if containers(range.start) != containers(range.end) {
        return Err(ControlError::Unsupported("插表选区不能跨越列表项或引用边界；原文已保留".into()));
    }
    let mut result = commands::apply(parsed, selection, intent)?;
    let edit = result.edits.first_mut().ok_or(ControlError::InvalidRange)?;
    let before = &parsed.source[commands::line_start(&parsed.source, range.start)..range.start];
    let mut consumed = 0;
    while let Some(length) = commands::structural_prefix_len(&before[consumed..]) {
        if length == 0 { break; }
        consumed += length;
    }
    // 空列表项直接成为表头；在任务框后保留原段落，续行只缩进列表标记宽度。
    let first_line = parsed.nodes.iter().rev().find(|node| matches!(node.kind, "item" | "quote")
        && contains_caret(parsed, &node.source, range.start))
        .map_or(range.start == before.len(), |node| commands::line_start(&parsed.source, node.source.start)
            == commands::line_start(&parsed.source, range.start));
    let prefix_only = first_line && before[consumed..].bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        && !before.contains("[ ]") && !before.contains("[x]") && !before.contains("[X]");
    if prefix_only {
        let excess = before.len().saturating_sub(prefix.len());
        if excess > 0 && before[before.len() - excess..].bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            // 空标记后的过量空白会把表头变成代码；只收拢这部分尚无正文的缩进。
            edit.from_byte -= excess as u64;
            result.selection.anchor_byte -= excess as u64;
            result.selection.head_byte -= excess as u64;
        }
        let removed = edit.inserted_text.len() - edit.inserted_text.trim_start_matches(['\r', '\n']).len();
        edit.inserted_text.drain(..removed);
        result.selection.anchor_byte -= removed as u64;
        result.selection.head_byte -= removed as u64;
        if before.trim().is_empty() && !prefix.is_empty() && before.len() < prefix.len() {
            let missing = &prefix[before.len()..];
            edit.inserted_text.insert_str(0, missing);
            result.selection.anchor_byte += missing.len() as u64;
            result.selection.head_byte += missing.len() as u64;
        }
    }
    if !prefix.is_empty() {
        // 懒续行的行首尚无引用前缀；补齐分隔空行，避免把一个引用拆成两个。
        if before.is_empty() && edit.inserted_text.starts_with(['\r', '\n']) {
            edit.inserted_text.insert_str(0, &prefix);
            result.selection = DocumentSelection::caret(result.selection.head_byte + prefix.len() as u64);
        }
        let caret = (result.selection.head_byte - edit.from_byte) as usize;
        let added = edit.inserted_text[..caret].bytes().filter(|byte| *byte == b'\n').count() * prefix.len();
        edit.inserted_text = edit.inserted_text.replace('\n', &format!("\n{prefix}"));
        result.selection = DocumentSelection::caret(result.selection.head_byte + added as u64);
    }
    Ok(result)
}

fn contains_caret(parsed: &ParsedWindow, span: &Range<usize>, at: usize) -> bool {
    span.contains(&at) || at == span.end && (at == parsed.source.len() || !parsed.source[..at].ends_with('\n'))
}

/// 列表任务框不是缩进的一部分；使用容器首行也能覆盖没有显式前缀的懒续行。
pub(super) fn continuation_prefix(parsed: &ParsedWindow, at: usize) -> Result<String, ControlError> {
    let source = &parsed.source;
    let node = parsed.nodes.iter().rev().find(|node| matches!(node.kind, "item" | "quote")
        && contains_caret(parsed, &node.source, at));
    let prefix = if let Some(node) = node {
        let start = node.source.start;
        let raw = &source[start..];
        let marker = if node.kind == "quote" { 1 }
            else if raw.starts_with(['-', '+', '*']) { 1 }
            else { raw.bytes().take_while(u8::is_ascii_digit).count() + 1 };
        let mut end = start + marker;
        if end > source.len() { return Err(ControlError::InvalidRange); }
        if node.kind == "quote" {
            if matches!(source.as_bytes().get(end), Some(b' ' | b'\t')) { end += 1; }
        } else {
            while matches!(source.as_bytes().get(end), Some(b' ' | b'\t')) { end += 1; }
        }
        let raw = &source[commands::line_start(source, start)..end];
        let mut prefix = String::new();
        for ch in raw.chars() {
            if ch == '\t' { prefix.push_str(&" ".repeat(4 - prefix.len() % 4)); }
            else { prefix.push(if ch == '>' { '>' } else { ' ' }); }
        }
        if node.kind == "item" {
            let marker_column = source[commands::line_start(source, start)..start + marker].bytes()
                .fold(0, |column, byte| column + if byte == b'\t' { 4 - column % 4 } else { 1 });
            if prefix.len() > marker_column + 4 { prefix.truncate(marker_column + 1); }
        }
        // 空的裸标记仍需要一个正文缩进空格。
        if end == start + marker { prefix.push(' '); }
        prefix
    } else {
        let line = &source[commands::line_start(source, at)..at];
        let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        if line == indent && parsed.context.ancestors.is_empty() { String::new() } else { indent.to_owned() }
    };
    let mut outer = String::new();
    for ancestor in &parsed.context.ancestors {
        match ancestor {
            AncestorContainer::Quote => outer.push_str("> "),
            AncestorContainer::ListItem { number } => outer.push_str(&" ".repeat(number.map_or(2, |number| number.to_string().len() + 2))),
            AncestorContainer::Details { .. } => {}
        }
    }
    if node.is_none() && !prefix.starts_with(&outer) {
        // ancestors 还参与外层布局，不能凭标签补出一层已被投影移除的容器。
        return Err(ControlError::NeedsContext("当前片段缺少外层列表或引用的实际前缀，请加载完整容器后插表；原文已保留".into()));
    }
    Ok(prefix)
}
