//! 列表命令按解析后的 item/list 祖先操作，避免把子项当作独立文本行缩进。
//! 仅生成局部补丁；组外源码、文档状态和历史仍由主会话保存。
use super::{
    commands::{self, EditIntent, EditResult},
    parse::ParsedWindow,
    protocol::*,
};
use std::ops::Range;

#[derive(Clone)]
struct Item {
    node: usize,
    list: usize,
    start: usize,
    end: usize,
    indent_at: usize,
    marker: usize,
    marker_end: usize,
    body: usize,
    task: bool,
}

fn item(parsed: &ParsedWindow, node: usize) -> Option<Item> {
    let n = &parsed.nodes[node];
    if n.kind != "item" {
        return None;
    }
    let list = n.parent.filter(|i| parsed.nodes[*i].kind == "list")?;
    let source = &parsed.source;
    let start = commands::line_start(source, n.source.start);
    let line_end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |i| start + i);
    let line = &source[start..line_end];
    // 引用前缀属于外层容器；列表缩进只改最后一个 > 后面的空白。
    let mut pos = 0;
    let mut indent_at = start;
    loop {
        pos += line[pos..].len() - line[pos..].trim_start_matches([' ', '\t']).len();
        if line.as_bytes().get(pos) == Some(&b'>') {
            pos += 1;
            if line.as_bytes().get(pos) == Some(&b' ') {
                pos += 1;
            }
            indent_at = start + pos;
        } else {
            break;
        }
    }
    // 同一物理行也可能嵌套列表；节点起点才是本层标记的下界。
    pos = pos.max(n.source.start - start);
    pos += line[pos..].len() - line[pos..].trim_start_matches([' ', '\t']).len();
    let marker = start + pos;
    let raw = &line[pos..];
    let digits = raw.bytes().take_while(u8::is_ascii_digit).count();
    let len = if raw.starts_with(['-', '+', '*']) {
        1
    } else if digits > 0 && matches!(raw.as_bytes().get(digits), Some(b'.' | b')')) {
        digits + 1
    } else {
        return None;
    };
    if raw.len() > len && !matches!(raw.as_bytes()[len], b' ' | b'\t') {
        return None;
    }
    let mut body = marker + len;
    while body < line_end && matches!(source.as_bytes()[body], b' ' | b'\t') {
        body += 1;
    }
    let marker_end = body;
    let task = source
        .get(body..body.saturating_add(3))
        .is_some_and(|s| matches!(s, "[ ]" | "[x]" | "[X]"))
        && (body + 3 == line_end || matches!(source.as_bytes().get(body + 3), Some(b' ' | b'\t')));
    if task {
        body += 3;
        while body < line_end && matches!(source.as_bytes()[body], b' ' | b'\t') {
            body += 1;
        }
    }
    let mut end = n.source.end;
    if end > 0 && source.as_bytes()[end - 1] != b'\n' {
        end += source[end..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
    }
    Some(Item {
        node,
        list,
        start,
        end: end.max(body),
        indent_at,
        marker,
        marker_end,
        body,
        task,
    })
}

fn containing(parsed: &ParsedWindow, at: usize) -> Option<Item> {
    parsed
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, n)| n.kind == "item" && n.source.start <= at
            && (at <= n.source.end || (n.source.end > 0 && parsed.source.as_bytes()[n.source.end - 1] != b'\n'
                && parsed.source[n.source.end..at].bytes().all(|byte| matches!(byte, b' ' | b'\t')))))
        .and_then(|(i, _)| item(parsed, i))
        .filter(|item| {
            let start = commands::line_start(&parsed.source, at);
            start == item.start || !parsed.source[start..line_end(&parsed.source, at)].is_empty()
        })
}
fn siblings(parsed: &ParsedWindow, list: usize) -> Vec<Item> {
    parsed
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == "item" && n.parent == Some(list))
        .filter_map(|(i, _)| item(parsed, i))
        .collect()
}

/// 键盘入口与列表命令共用包含末尾落点的判断，不能退回替换选区的普通 Tab。
pub(super) fn tab(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    backwards: bool,
) -> Option<Result<EditResult, ControlError>> {
    if !backwards {
        if let Some(result) = complete_prefix(parsed, selection) {
            return Some(result);
        }
    }
    let range = match super::mapping::local_range(&parsed.source, parsed.identity.from_byte, selection.range()) {
        Ok(range) => range,
        Err(_) => return Some(Err(ControlError::NeedsContext("跨窗口列表缩进需要完整结构；选区已保留".into()))),
    };
    let touches = containing(parsed, range.start).is_some() || containing(parsed, range.end).is_some()
        || parsed.nodes.iter().any(|node| node.kind == "item" && range.start < node.source.end && node.source.start < range.end);
    if !touches { return None; }
    Some(apply(parsed, selection, &EditIntent::Indent { outdent: backwards })
        .unwrap_or_else(|| Ok(result(selection, Vec::new()))))
}

fn shortcut_prefix(text: &str) -> Option<String> {
    let text = text.trim_matches([' ', '\t']);
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let marker_end = if text.starts_with(['-', '+', '*']) { 1 }
        else if (1..=9).contains(&digits) && text.as_bytes().get(digits) == Some(&b'.') { digits + 1 }
        else { return None; };
    let tail = &text[marker_end..];
    let task = tail.trim_start_matches([' ', '\t']);
    if !task.is_empty() && (tail == task || !matches!(task, "[ ]" | "[x]" | "[X]")) { return None; }
    let marker = if digits > 0 { &text[..marker_end] } else { "-" };
    Some(if task.is_empty() { format!("{marker} ") } else { format!("{marker} {} ", task.to_ascii_lowercase()) })
}

/// 空格与 Tab 共用前缀转换；一次事务移除临时外壳，避免产生同一行的空父项。
pub(super) fn complete_prefix(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
) -> Option<Result<EditResult, ControlError>> {
    let range = super::mapping::local_range(&parsed.source, parsed.identity.from_byte, selection.range()).ok()?;
    if !range.is_empty() { return None; }
    let at = range.start;
    let source = &parsed.source;
    if !source[..at].ends_with(['-', '+', '*', '.', ']']) { return None; }
    let start = commands::line_start(source, at);
    // 普通词间空格直接返回，不为每次输入扫描窗口内的语义节点。
    if at != line_end(source, at) || source[..at].ends_with([' ', '\t'])
        || source[start..at].bytes().any(|byte| !byte.is_ascii_digit() && !b" \t>-.+*[]xX".contains(&byte))
    { return None; }
    if parsed.nodes.iter().any(|node| matches!(node.kind, "code" | "table" | "html") && node.source.start <= at && at <= node.source.end)
        || parsed.inline.iter().any(|span| span.format == super::parse::InlineFormat::Code && span.open.start <= at && at <= span.close.end)
    { return None; }
    let outer = parsed.nodes.iter().enumerate()
        .filter(|(_, node)| node.kind == "item" && node.source.start <= at && at <= node.source.end)
        .filter_map(|(node, _)| item(parsed, node))
        .filter(|entry| entry.start == start)
        .min_by_key(|entry| entry.marker);
    let from = outer.as_ref().map_or(start, |entry| entry.indent_at);
    if let Some(prefix) = shortcut_prefix(&source[from..at]) {
        let prefix = format!("{}{prefix}", &source[from..at][..source[from..at].len() - source[from..at].trim_start_matches([' ', '\t']).len()]);
        return Some(Ok(result(selection, vec![edit(parsed, from..at, prefix)])));
    }
    let outer = outer?;
    if outer.task || !source[at..outer.end].trim().is_empty() { return None; }
    let prefix = shortcut_prefix(source.get(outer.body..at)?)?;
    let peers = siblings(parsed, outer.list);
    let index = peers.iter().position(|entry| entry.node == outer.node)?;
    let indent = if index > 0 {
        let previous = &peers[index - 1];
        outer.marker - outer.indent_at + previous.marker_end - previous.marker
    } else { outer.marker - outer.indent_at };
    let replacement = format!("{}{prefix}", " ".repeat(indent));
    Some(Ok(result(selection, vec![edit(parsed, outer.indent_at..at, replacement)])))
}
fn edit(parsed: &ParsedWindow, range: Range<usize>, text: String) -> SourceEdit {
    SourceEdit {
        from_byte: parsed.identity.from_byte + range.start as u64,
        to_byte: parsed.identity.from_byte + range.end as u64,
        inserted_text: text,
    }
}
fn result(selection: DocumentSelection, mut edits: Vec<SourceEdit>) -> EditResult {
    edits.retain(|e| e.from_byte != e.to_byte || !e.inserted_text.is_empty());
    edits.sort_by_key(|e| (e.from_byte, e.to_byte));
    let mut merged: Vec<SourceEdit> = Vec::with_capacity(edits.len());
    for edit in edits {
        if let Some(previous) = merged.last_mut().filter(|previous| previous.from_byte == edit.from_byte && previous.to_byte == edit.from_byte) {
            previous.to_byte = edit.to_byte;
            previous.inserted_text.push_str(&edit.inserted_text);
        } else { merged.push(edit); }
    }
    EditResult {
        selection: commands::map_selection(selection, &merged),
        edits: merged,
        template_update: None,
    }
}
fn line_end(source: &str, from: usize) -> usize {
    source[from..]
        .find(['\r', '\n'])
        .map_or(source.len(), |i| from + i)
}

fn join_end(parsed: &ParsedWindow, previous: &Item) -> Option<usize> {
    let source = &parsed.source;
    let last_text = previous.body + source[previous.body..previous.end].trim_end().len();
    // 前项的普通续行可以合并；列表、围栏等子块的尾部不属于父项的可编辑正文。
    let ends_in = |span: &Range<usize>| previous.body <= span.start && span.start < last_text && last_text <= span.end;
    if parsed.nodes.iter().any(|node| ends_in(&node.source)
        && matches!(node.kind, "list" | "quote" | "code" | "html" | "table" | "heading" | "footnote"))
        || parsed.events.iter().any(|(event, span)| matches!(event, pulldown_cmark::Event::Rule | pulldown_cmark::Event::DisplayMath(_)) && ends_in(span)) {
        return None;
    }
    Some(line_end(source, last_text).min(previous.end))
}
fn shifted_lines(
    parsed: &ParsedWindow,
    it: &Item,
    amount: isize,
    edits: &mut Vec<SourceEdit>,
) -> Result<(), ControlError> {
    let source = &parsed.source;
    let quote = &source[it.start..it.indent_at];
    let mut start = it.start;
    while start < it.end {
        let end = line_end(source, start);
        let line = &source[start..end];
        if !line.trim().is_empty() {
            // 懒续行需要完整容器上下文，不能把正文的第一个字符当缩进删掉。
            if !line.starts_with(quote) {
                return Err(ControlError::NeedsContext(
                    "列表包含不同引用前缀，需完整结构操作".into(),
                ));
            }
            let at = start + quote.len();
            if amount >= 0 {
                edits.push(edit(parsed, at..at, " ".repeat(amount as usize)));
            } else {
                let remove = amount.unsigned_abs();
                if !source[at..end].starts_with(&" ".repeat(remove)) {
                    return Err(ControlError::NeedsContext(
                        "列表续行缩进不足，原文已保留".into(),
                    ));
                }
                edits.push(edit(parsed, at..at + remove, String::new()));
            }
        }
        start = source[end..]
            .find('\n')
            .map_or(source.len(), |i| end + i + 1);
    }
    Ok(())
}

/// 标记宽度变化时同步移动后续段落和子树；未缩进的 Markdown 懒续行保持原样。
fn reindent_tail(
    parsed: &ParsedWindow,
    it: &Item,
    mut from: usize,
    delta: isize,
    edits: &mut Vec<SourceEdit>,
) {
    if delta == 0 { return; }
    let source = &parsed.source;
    let quote = &source[it.start..it.indent_at];
    let content_indent = it.marker_end - it.indent_at;
    while from < it.end {
        let end = line_end(source, from);
        let line = &source[from..end];
        if let Some(body) = line.strip_prefix(quote) {
            let spaces = body.bytes().take_while(|byte| *byte == b' ').count();
            if spaces >= content_indent && !body.trim().is_empty() {
                let at = from + quote.len();
                edits.push(if delta > 0 { edit(parsed, at..at, " ".repeat(delta as usize)) }
                    else { edit(parsed, at..at + delta.unsigned_abs(), String::new()) });
            }
        }
        from = source[end..].find('\n').map_or(source.len(), |offset| end + offset + 1);
    }
}

/// 拆分列表项时先闭合左侧格式，再在新项恢复右侧格式；光标仍使用原文坐标。
fn split_item(
    parsed: &ParsedWindow,
    it: &Item,
    selection: DocumentSelection,
    mut range: Range<usize>,
) -> Result<EditResult, ControlError> {
    let source = &parsed.source;
    if !range.is_empty() && parsed.nodes.iter().any(|node| matches!(node.kind, "item" | "list" | "code" | "table" | "html")
        && range.start < node.source.start && node.source.start < range.end)
    {
        return Err(ControlError::NeedsContext("列表选区跨越子结构，需要完整结构操作；选区已保留".into()));
    }
    let base = parsed.identity.from_byte;
    let mut visible = it.body;
    while visible < it.end {
        let marker = parsed.context.inline_templates.iter().flat_map(|template| [
            template.from_byte..template.open_end_byte, template.close_start_byte..template.to_byte,
        ]).find(|marker| marker.start == base + visible as u64 && marker.end > marker.start);
        if let Some(marker) = marker { visible = (marker.end - base) as usize; }
        else if let Some(ch) = source[visible..it.end].chars().next().filter(|ch| ch.is_whitespace()) { visible += ch.len_utf8(); }
        else { break; }
    }
    if visible == it.end {
        let mut lifted = lift(parsed, std::slice::from_ref(it), selection, true)?;
        let end = it.body + source[it.body..it.end].trim_end_matches(['\r', '\n']).len();
        if end > it.body { lifted.edits.push(edit(parsed, it.body..end, String::new())); }
        return Ok(result(selection, lifted.edits));
    }
    let mut prefix = source[it.start..it.body].to_string();
    if it.task {
        let at = it.marker_end - it.start + 1;
        prefix.replace_range(at..at + 1, " ");
    }
    let offset = it.marker - it.start;
    let digits = prefix[offset..].bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let peers = siblings(parsed, it.list);
        let index = peers.iter().position(|entry| entry.node == it.node).ok_or(ControlError::InvalidRange)?;
        let first = &peers[0];
        let first_digits = source[first.marker..].bytes().take_while(u8::is_ascii_digit).count();
        let first_number = source[first.marker..first.marker + first_digits].parse::<u64>().map_err(|_| ControlError::InvalidRange)?;
        let number = first_number.saturating_add(index as u64).saturating_add(1);
        prefix.replace_range(offset..offset + digits, &number.to_string());
    }
    // 可见文字边缘的光标不能把隐藏的起止标记留在另一项中。
    if range.is_empty() {
        loop {
            let old = range.start;
            if let Some(span) = parsed.inline.iter().rev().find(|span| !span.content.is_empty() && span.content.end == old) {
                range = span.close.end..span.close.end;
            } else if let Some(span) = parsed.inline.iter().find(|span| !span.content.is_empty() && span.content.start == old) {
                range = span.open.start..span.open.start;
            }
            if old == range.start { break; }
        }
    } else {
        loop {
            let old = range.clone();
            for span in &parsed.inline {
                if range.start == span.content.start { range.start = span.open.start; }
                if range.end == span.content.end { range.end = span.close.end; }
            }
            if old == range { break; }
        }
    }
    for span in &parsed.inline {
        for marker in [&span.open, &span.close] {
            if [range.start, range.end].iter().any(|at| marker.start < *at && *at < marker.end) {
                return Err(ControlError::NeedsContext("列表回车不能截断行内格式标记；原文已保留".into()));
            }
        }
    }
    let mut inserted = String::new();
    for span in parsed.inline.iter().rev().filter(|span| span.content.start < range.start && range.start < span.content.end) {
        inserted.push_str(&source[span.close.clone()]);
    }
    inserted.push_str(commands::line_ending(source, range.start));
    inserted.push_str(&prefix);
    for span in parsed.inline.iter().filter(|span| span.content.start < range.end && range.end < span.content.end) {
        inserted.push_str(&source[span.open.clone()]);
    }
    let mut right = range.end;
    while let Some(span) = parsed.inline.iter().find(|span| span.open.start == right && span.open.end > right) {
        right = span.open.end;
    }
    let caret = parsed.identity.from_byte + (range.start + inserted.len() + right - range.end) as u64;
    let next = source[range.end..].find('\n').map_or(it.end, |offset| range.end + offset + 1);
    let delta = prefix.len() as isize - (it.body - it.start) as isize;
    let mut edits = vec![edit(parsed, range, inserted)];
    reindent_tail(parsed, it, next, delta, &mut edits);
    let mut result = result(selection, edits);
    result.selection = DocumentSelection::caret(caret);
    Ok(result)
}

fn lift(
    parsed: &ParsedWindow,
    selected: &[Item],
    selection: DocumentSelection,
    clear_empty_task: bool,
) -> Result<EditResult, ControlError> {
    let first = &selected[0];
    let parent = parsed.nodes[first.list]
        .parent
        .and_then(|n| item(parsed, n));
    let mut edits = Vec::new();
    if let Some(parent) = parent {
        let delta = first
            .marker
            .saturating_sub(first.indent_at)
            .saturating_sub(parent.marker.saturating_sub(parent.indent_at));
        if delta == 0 {
            return Err(ControlError::NeedsContext(
                "列表祖先缩进没有完整映射".into(),
            ));
        }
        for it in selected {
            shifted_lines(parsed, it, -(delta as isize), &mut edits)?;
            // 提升后的项采用外层列表类型，保留已有任务状态。
            let parent_marker = &parsed.source[parent.marker..parent.marker_end];
            let marker_end = it.marker_end;
            edits.push(edit(parsed, it.marker..marker_end, parent_marker.into()));
            let next = parsed.source[it.start..].find('\n').map_or(it.end, |offset| it.start + offset + 1);
            reindent_tail(parsed, it, next, parent_marker.len() as isize - (marker_end - it.marker) as isize, &mut edits);
            if clear_empty_task && it.task {
                edits.push(edit(parsed, marker_end..it.body, String::new()));
            }
        }
    } else {
        for it in selected {
            let eol = commands::line_ending(&parsed.source, it.start);
            // 退出顶层列表时移除任务符号，保留子任务。空行阻止 Markdown 懒续行吞并。
            let previous_line = commands::line_start(&parsed.source, it.start.saturating_sub(1));
            let separated = parsed.source[previous_line..it.start].trim_matches([' ', '\t', '\r', '\n', '>']).is_empty();
            let before = if it.start > parsed.nodes[it.list].source.start && !separated {
                format!("{eol}{}", &parsed.source[it.start..it.indent_at])
            } else {
                String::new()
            };
            edits.push(edit(parsed, it.indent_at..it.body, before));
            let width = it.marker_end - it.marker;
            let mut continuation = it.clone();
            continuation.start = parsed.source[it.start..]
                .find('\n')
                .map_or(it.end, |i| it.start + i + 1);
            // continuation 的 quote 仍沿用原首行。
            let quote = &parsed.source[it.start..it.indent_at];
            let mut at = continuation.start;
            while at < it.end {
                let end = line_end(&parsed.source, at);
                if parsed.source[at..end].starts_with(quote) {
                    let indent = at + quote.len();
                    let available = parsed.source[indent..end]
                        .bytes()
                        .take_while(|b| *b == b' ')
                        .count();
                    let trim = width.min(available);
                    if trim > 0 {
                        edits.push(edit(parsed, indent..indent + trim, String::new()));
                    }
                }
                at = parsed.source[end..]
                    .find('\n')
                    .map_or(parsed.source.len(), |i| end + i + 1);
            }
        }
        let last = selected.last().unwrap();
        if last.end < parsed.nodes[last.list].source.end {
            let eol = commands::line_ending(&parsed.source, last.end);
            edits.push(edit(parsed, last.end..last.end, eol.into()));
        }
    }
    Ok(result(selection, edits))
}

/// Some 表示在列表内已处理（包括不能缩进的首项）；None 才落入普通段落处理。
pub fn apply(
    parsed: &ParsedWindow,
    selection: DocumentSelection,
    intent: &EditIntent,
) -> Option<Result<EditResult, ControlError>> {
    let range =
        super::mapping::local_range(&parsed.source, parsed.identity.from_byte, selection.range())
            .ok()?;
    if parsed
        .nodes
        .iter()
        .any(|n| n.kind == "code" && n.source.contains(&range.start))
    {
        return None;
    }
    let source = &parsed.source;
    let it = match containing(parsed, range.start) {
        Some(it) => it,
        None => {
            if matches!(intent, EditIntent::Backspace)
                && range.is_empty()
                && source[commands::line_start(source, range.start)..line_end(source, range.start)]
                    .trim()
                    .is_empty()
            {
                let previous = parsed
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        n.kind == "list"
                            && n.source.end <= range.start
                            && n.parent
                                .is_none_or(|parent| parsed.nodes[parent].kind != "item")
                            && source[n.source.end..range.start].trim().is_empty()
                    })
                    .max_by_key(|(_, n)| n.source.end);
                if let Some((list, node)) = previous {
                    let peers = siblings(parsed, list);
                    let last = peers.last()?;
                    let mut prefix = source[last.start..last.marker_end].to_string();
                    if last.task {
                        prefix.push_str("[ ] ");
                    }
                    let offset = last.marker - last.start;
                    let digits = prefix[offset..]
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count();
                    if digits > 0 {
                        if let Ok(n) = prefix[offset..offset + digits].parse::<u64>() {
                            prefix.replace_range(offset..offset + digits, &(n + 1).to_string());
                        }
                    }
                    let from = source[..node.source.end]
                        .trim_end_matches(['\r', '\n'])
                        .len();
                    return Some(Ok(result(
                        selection,
                        vec![edit(
                            parsed,
                            from..range.start,
                            format!("{}{prefix}", commands::line_ending(source, range.start)),
                        )],
                    )));
                }
            }
            return None;
        }
    };
    match intent {
        EditIntent::LinePrefix(prefix) if matches!(prefix.as_str(), "- " | "1. " | "- [ ] ") => {
            let peers = siblings(parsed, it.list);
            let ordered = source.as_bytes()[it.marker].is_ascii_digit();
            if prefix != "- [ ] " && ordered == (prefix == "1. ") {
                // 同类型快捷键取消整个当前列表层级，不能只提升光标所在项。
                return Some(lift(parsed, &peers, selection, true));
            }
            // v0.6.2 跨类型切换修改父列表容器；子列表类型及任务状态独立保留。
            let remove_tasks = prefix == "- [ ] " && peers.iter().all(|p| p.task);
            let mut edits = Vec::new();
            for (i, entry) in peers.iter().enumerate() {
                let marker_end = entry.marker_end;
                if prefix == "- [ ] " {
                    if remove_tasks {
                        edits.push(edit(parsed, marker_end..entry.body, String::new()));
                    } else if !entry.task {
                        edits.push(edit(parsed, entry.body..entry.body, "[ ] ".into()));
                    }
                } else {
                    let marker = if prefix == "1. " { format!("{}. ", i + 1) } else { "- ".into() };
                    let delta = marker.len() as isize - (marker_end - entry.marker) as isize;
                    edits.push(edit(parsed, entry.marker..marker_end, marker));
                    let next = source[entry.start..].find('\n').map_or(entry.end, |offset| entry.start + offset + 1);
                    reindent_tail(parsed, entry, next, delta, &mut edits);
                }
            }
            Some(Ok(result(selection, edits)))
        }
        EditIntent::Indent { outdent } => {
            let mut first = it;
            // 跨子列表选区提升到共同列表层级，避免父子项重复应用缩进。
            while range.end > first.end {
                let Some(parent) = parsed.nodes[first.list]
                    .parent
                    .and_then(|n| item(parsed, n))
                else {
                    break;
                };
                if range.end <= parsed.nodes[first.list].source.end {
                    break;
                }
                first = parent;
            }
            let peers = siblings(parsed, first.list);
            if range.end > peers.last().map_or(parsed.nodes[first.list].source.end, |entry| entry.end.max(parsed.nodes[first.list].source.end)) {
                return Some(Ok(result(selection, Vec::new())));
            }
            let from = peers.iter().position(|p| p.node == first.node).unwrap();
            let selected: Vec<_> = peers
                .iter()
                .skip(from)
                .take_while(|p| p.start < range.end.max(range.start + 1))
                .cloned()
                .collect();
            if *outdent {
                return Some(lift(parsed, &selected, selection, false));
            }
            let previous = if from > 0 {
                Some(peers[from - 1].clone())
            } else {
                // 类型转换会拆开相邻列表；保持原版“先转换再 Tab”与“先 Tab 再转换”等价。
                parsed
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        n.kind == "list"
                            && n.parent == parsed.nodes[first.list].parent
                            && n.source.end <= first.start
                            && source[n.source.end..first.start].trim().is_empty()
                    })
                    .max_by_key(|(_, n)| n.source.end)
                    .and_then(|(list, _)| siblings(parsed, list).pop())
            };
            let Some(previous) = previous else {
                return Some(Ok(result(selection, vec![])));
            };
            let width = previous.marker_end - previous.marker;
            let mut edits = Vec::new();
            let ordered = from > 0 && source.as_bytes()[first.marker].is_ascii_digit();
            let mut number = 1u64;
            if ordered {
                let tail = parsed.nodes.iter().enumerate().filter(|(_, node)| node.parent == Some(previous.node)).max_by_key(|(_, node)| node.source.end);
                if let Some((list, _)) = tail.filter(|(_, node)| node.kind == "list") {
                    let children = siblings(parsed, list);
                    if let Some(child) = children.first().filter(|child| source.as_bytes()[child.marker].is_ascii_digit()) {
                        let digits = source[child.marker..].bytes().take_while(u8::is_ascii_digit).count();
                        let Ok(start) = source[child.marker..child.marker + digits].parse::<u64>() else {
                            return Some(Err(ControlError::InvalidRange));
                        };
                        number = start.saturating_add(children.len() as u64);
                    }
                }
            }
            for entry in &selected {
                if let Err(error) = shifted_lines(parsed, entry, width as isize, &mut edits) {
                    return Some(Err(error));
                }
                if ordered {
                    let delimiter = if source[entry.marker..entry.marker_end].contains(')') { ')' } else { '.' };
                    let marker = format!("{number}{delimiter} ");
                    let delta = marker.len() as isize - (entry.marker_end - entry.marker) as isize;
                    edits.push(edit(parsed, entry.marker..entry.marker_end, marker));
                    let next = source[entry.start..].find('\n').map_or(entry.end, |offset| entry.start + offset + 1);
                    reindent_tail(parsed, entry, next, delta, &mut edits);
                    number = number.saturating_add(1);
                }
            }
            Some(Ok(result(selection, edits)))
        }
        EditIntent::Newline | EditIntent::ParagraphBreak
            if range.start >= it.body && range.end <= it.end
                && !parsed.nodes.iter().any(|node| matches!(node.kind, "quote" | "html" | "table")
                    && node.source.start >= it.body && node.source.start <= range.start && range.start < node.source.end) =>
        {
            Some(split_item(parsed, &it, selection, range))
        }
        EditIntent::Backspace if range.is_empty() && range.start == it.body => {
            if it.task {
                // 对齐原版 deleteTaskListItemBeforeCursor：在隐藏任务标记后删除当前项。
                let peers = siblings(parsed, it.list);
                let at = peers.iter().position(|p| p.node == it.node).unwrap();
                let mut r = result(
                    selection,
                    vec![edit(parsed, it.start..it.end, String::new())],
                );
                if peers.len() == 1 && it.indent_at > it.start {
                    r = result(
                        selection,
                        vec![edit(
                            parsed,
                            it.indent_at..it.end,
                            commands::line_ending(source, it.start).into(),
                        )],
                    );
                }
                r.selection = DocumentSelection::caret(
                    parsed.identity.from_byte
                        + if at > 0 {
                            join_end(parsed, &peers[at - 1]).unwrap_or_else(|| line_end(source, peers[at - 1].body))
                        } else {
                            it.start
                        } as u64,
                );
                Some(Ok(r))
            } else {
                let peers = siblings(parsed, it.list);
                let at = peers.iter().position(|p| p.node == it.node).unwrap();
                if at == 0 {
                    return Some(lift(parsed, &[it], selection, false));
                }
                let previous = &peers[at - 1];
                if let Some(previous_end) = join_end(parsed, previous).filter(|&end| source[end..it.start].trim().is_empty()) {
                    Some(Ok(result(
                        selection,
                        vec![edit(parsed, previous_end..it.body, String::new())],
                    )))
                } else {
                    // 前项含子结构时提升当前项，不跨过子项删除源码。
                    Some(lift(parsed, &[it], selection, false))
                }
            }
        }
        EditIntent::Delete if range.is_empty() && range.start == line_end(source, it.body) => {
            let peers = siblings(parsed, it.list);
            let at = peers.iter().position(|p| p.node == it.node).unwrap();
            let next = peers.get(at + 1)?;
            if source[range.start..next.start].trim().is_empty() {
                Some(Ok(result(
                    selection,
                    vec![edit(parsed, range.start..next.body, String::new())],
                )))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(source: &str, needle: &str, intent: EditIntent) -> String {
        run_at(source, source.find(needle).unwrap(), intent)
    }
    fn run_at(source: &str, at: usize, intent: EditIntent) -> String {
        let parsed = super::super::parse::fixture(source);
        let selection = DocumentSelection::caret(parsed.identity.from_byte + at as u64);
        let result = apply(&parsed, selection, &intent).unwrap_or_else(|| panic!("not handled: {source:?} at {at}: {intent:?}")).unwrap();
        edited(&parsed, &result)
    }
    fn edited(parsed: &ParsedWindow, result: &EditResult) -> String {
        let mut text = parsed.source.to_string();
        for edit in result.edits.iter().rev() {
            text.replace_range(
                (edit.from_byte - parsed.identity.from_byte) as usize
                    ..(edit.to_byte - parsed.identity.from_byte) as usize,
                &edit.inserted_text,
            );
        }
        text
    }
    #[test]
    fn indent_moves_subtree_and_first_item_is_consumed() {
        let text = "- first\r\n- second\r\n  - child\r\n- last\r\n";
        assert_eq!(
            run(text, "second", EditIntent::Indent { outdent: false }),
            "- first\r\n  - second\r\n    - child\r\n- last\r\n"
        );
        assert_eq!(
            run(text, "first", EditIntent::Indent { outdent: false }),
            text
        );
    }
    #[test]
    fn quoted_nested_task_enter_lifts_one_level_and_resets_check() {
        let text = "> - parent\n>   - [x] \n";
        assert_eq!(
            run_at(text, text.len() - 1, EditIntent::Newline),
            "> - parent\n> - \n"
        );
        let text = "- parent\n  - [x] \n";
        assert_eq!(
            run_at(text, text.len() - 1, EditIntent::Newline),
            "- parent\n- \n"
        );
        assert_eq!(
            run("> - [x] hello\n", "hello", EditIntent::Newline),
            "> - [x] \n> - [ ] hello\n"
        );
    }
    #[test]
    fn lifting_nested_item_keeps_following_items_as_children() {
        assert_eq!(
            run(
                "- parent\n  - one\n  - two\n  - three\n",
                "two",
                EditIntent::Indent { outdent: true }
            ),
            "- parent\n  - one\n- two\n  - three\n"
        );
    }
    #[test]
    fn backspace_joins_siblings_without_losing_format_or_child() {
        assert_eq!(
            run(
                "- **one**\n- two\n  - child\n",
                "two",
                EditIntent::Backspace
            ),
            "- **one**two\n  - child\n"
        );
        assert_eq!(
            run(
                "- one\n- [x] two\n  - child\n- last\n",
                "two",
                EditIntent::Backspace
            ),
            "- one\n- last\n"
        );
    }
    #[test]
    fn list_type_and_tasks_switch_the_parent_group() {
        assert_eq!(
            run(
                "1. first\n2. second\n   - child\n3. last\n",
                "second",
                EditIntent::LinePrefix("- ".into())
            ),
            "- first\n- second\n  - child\n- last\n"
        );
        assert_eq!(
            run(
                "- [x] first\n  - [x] child\n- last\n",
                "first",
                EditIntent::LinePrefix("- [ ] ".into())
            ),
            "- [x] first\n  - [x] child\n- [ ] last\n"
        );
        assert_eq!(
            run(
                "1. first\n",
                "first",
                EditIntent::LinePrefix("- [ ] ".into())
            ),
            "1. [ ] first\n"
        );
        assert_eq!(
            run("> - [x] first\n", "first", EditIntent::Backspace),
            "> \n"
        );
    }
    #[test]
    fn adjacent_list_indent_and_empty_task_recovery_follow_original_commands() {
        assert_eq!(
            run(
                "1. Parent\n- Child\n",
                "Child",
                EditIntent::Indent { outdent: false }
            ),
            "1. Parent\n   - Child\n"
        );
        assert_eq!(
            run(
                "- Parent\n\n5. Child\n6. Tail",
                "Child",
                EditIntent::Indent { outdent: false }
            ),
            "- Parent\n\n  5. Child\n6. Tail"
        );
        assert_eq!(run_at("- [x] ", 6, EditIntent::Newline), "");
        assert_eq!(run_at("- [x]", 5, EditIntent::Newline), "");
        assert_eq!(
            run_at("- [x] A\n\n", 9, EditIntent::Backspace),
            "- [x] A\n- [ ] "
        );
        assert_eq!(
            run(
                "- [ ] A\n  - [x] B",
                "B",
                EditIntent::Indent { outdent: true }
            ),
            "- [ ] A\n- [x] B"
        );
    }

    #[test]
    fn tab_at_eof_moves_the_item_and_preserves_first_item_and_selection() {
        for (source, expected) in [
            ("- A\n- B", "- A\n  - B"),
            ("5. A\r\n6. B", "5. A\r\n   1. B"),
            ("- [x] A\n- [ ] B", "- [x] A\n  - [ ] B"),
            ("- A", "- A"),
        ] {
            let parsed = super::super::parse::fixture(source);
            let selection = DocumentSelection::caret(parsed.identity.from_byte + source.len() as u64);
            let result = tab(&parsed, selection, false).unwrap().unwrap();
            assert_eq!(edited(&parsed, &result), expected, "{source:?}");
            assert_eq!(result.selection.head_byte, parsed.identity.from_byte + expected.len() as u64);
        }
        let parsed = super::super::parse::fixture("- A\n- B");
        for reversed in [false, true] {
            let mut selection = DocumentSelection { anchor_byte: parsed.identity.from_byte + 2, head_byte: parsed.identity.to_byte };
            if reversed { std::mem::swap(&mut selection.anchor_byte, &mut selection.head_byte); }
            let result = tab(&parsed, selection, false).unwrap().unwrap();
            assert!(result.edits.is_empty());
            assert_eq!(result.selection, selection);
        }
        let source = "- A\n  - [x] B";
        let parsed = super::super::parse::fixture(source);
        let result = tab(&parsed, DocumentSelection::caret(parsed.identity.to_byte), true).unwrap().unwrap();
        assert_eq!(edited(&parsed, &result), "- A\n- [x] B");
        let outside = DocumentSelection { anchor_byte: parsed.identity.from_byte, head_byte: parsed.identity.to_byte + 100 };
        assert!(matches!(tab(&parsed, outside, false), Some(Err(ControlError::NeedsContext(_)))));
    }

    #[test]
    fn space_and_tab_complete_prefixes_with_one_mixed_list_transition() {
        for (source, expected) in [
            ("-", "- "), ("*", "- "), ("+", "- "), ("12.", "12. "),
            ("- [X]", "- [x] "), ("> -", "> - "),
            ("1. Parent\n2. -", "1. Parent\n   - "),
            ("1. Parent\n2. - [x]", "1. Parent\n   - [x] "),
            ("1. Parent\n   1. -", "1. Parent\n   - "),
        ] {
            let parsed = super::super::parse::fixture(source);
            let selection = DocumentSelection::caret(parsed.identity.to_byte);
            let space = complete_prefix(&parsed, selection).unwrap_or_else(|| panic!("{source:?}")).unwrap();
            let tab = tab(&parsed, selection, false).unwrap().unwrap();
            assert_eq!(space, tab, "{source:?}");
            assert_eq!(edited(&parsed, &space), expected, "{source:?}");
            assert_eq!(space.selection.head_byte, parsed.identity.from_byte + expected.len() as u64);
        }
        for source in ["text", "--", "1. already", "1. ", "code\n\n\u{0060}\u{0060}\u{0060}\n-", "\u{0060}-\u{0060}"] {
            let parsed = super::super::parse::fixture(source);
            assert!(complete_prefix(&parsed, DocumentSelection::caret(parsed.identity.to_byte)).is_none(), "{source:?}");
        }
    }

    #[test]
    fn group_conversion_moves_descendants_without_changing_their_type_or_check_state() {
        let source = "5. [x] A\r\n   7. Child\r\n6. B\r\n";
        let parsed = super::super::parse::fixture(source);
        let selection = DocumentSelection::caret(parsed.identity.from_byte + source.find('B').unwrap() as u64);
        let result = apply(&parsed, selection, &EditIntent::LinePrefix("- ".into())).unwrap().unwrap();
        let expected = "- [x] A\r\n  7. Child\r\n- B\r\n";
        assert_eq!(edited(&parsed, &result), expected);
        assert_eq!(result.selection.head_byte, parsed.identity.from_byte + expected.find('B').unwrap() as u64);
        assert_eq!(run("- A\n  7. Child\n- B", "B", EditIntent::LinePrefix("1. ".into())), "1. A\n   7. Child\n2. B");
        let source = "- [x] A\n- B\n  - [x] Child";
        let marked = run(source, "B", EditIntent::LinePrefix("- [ ] ".into()));
        assert_eq!(marked, "- [x] A\n- [ ] B\n  - [x] Child");
        assert_eq!(run(&marked, "B", EditIntent::LinePrefix("- [ ] ".into())), "- A\n- B\n  - [x] Child");
    }

    #[test]
    fn same_type_shortcut_cancels_all_peers_from_any_item_or_selection() {
        for (source, prefix, expected) in [
            ("1. A\n2. B\n3. C", "1. ", "A\n\nB\n\nC"),
            ("1. A\n\n2. B\n\n3. C", "1. ", "A\n\nB\n\nC"),
            ("- A\r\n- B\r\n- C", "- ", "A\r\n\r\nB\r\n\r\nC"),
            ("> 1. A\n> 2. B\n> 3. C", "1. ", "> A\n> \n> B\n> \n> C"),
        ] {
            let parsed = super::super::parse::fixture(source);
            for needle in ['A', 'B', 'C'] {
                let at = source.find(needle).unwrap();
                for selection in [
                    DocumentSelection::caret(parsed.identity.from_byte + at as u64),
                    DocumentSelection { anchor_byte: parsed.identity.from_byte + at as u64, head_byte: parsed.identity.to_byte },
                    DocumentSelection { anchor_byte: parsed.identity.to_byte, head_byte: parsed.identity.from_byte + at as u64 },
                ] {
                    let change = apply(&parsed, selection, &EditIntent::LinePrefix(prefix.into())).unwrap().unwrap();
                    assert_eq!(edited(&parsed, &change), expected, "{source:?}: {selection:?}");
                    let next = super::super::parse::fixture(expected);
                    assert!(!next.nodes.iter().any(|node| node.kind == "list"));
                    assert!(change.selection.range().end <= parsed.identity.from_byte + expected.len() as u64);
                }
            }
        }
        let source = "1. Parent\n   1. A\n   2. B\n2. Tail";
        assert_eq!(run(source, "B", EditIntent::LinePrefix("1. ".into())), "1. Parent\n1. A\n1. B\n2. Tail");
        let source = "1. A\n2. B\n\n---\n\n1. Other";
        assert_eq!(run(source, "B", EditIntent::LinePrefix("1. ".into())), "A\n\nB\n\n---\n\n1. Other");
    }

    #[test]
    fn enter_splits_inline_formats_at_visible_edges_and_inside_text() {
        for (source, at, expected) in [
            ("- **ABCD**", 6, "- **AB**\n- **CD**"),
            ("- **AB**", 4, "- \n- **AB**"),
            ("- **AB**", 6, "- **AB**\n- "),
            ("- [ABCD](https://example.test)", 5, "- [AB](https://example.test)\n- [CD](https://example.test)"),
            ("- A\n\n  BC", 8, "- A\n\n  B\n- C"),
        ] {
            let parsed = super::super::parse::fixture(source);
            let selection = DocumentSelection::caret(parsed.identity.from_byte + at);
            let result = apply(&parsed, selection, &EditIntent::ParagraphBreak).unwrap().unwrap();
            let text = edited(&parsed, &result);
            assert_eq!(text, expected, "{source:?} at {at}");
            assert!(result.selection.head_byte <= parsed.identity.from_byte + text.len() as u64);
            let next = super::super::parse::fixture(&text);
            if source.contains("**") && at == 6 && source.contains("CD") {
                assert_eq!(next.inline.iter().filter(|span| span.format == super::super::parse::InlineFormat::Strong).count(), 2);
            }
        }
    }

    #[test]
    fn enter_continues_displayed_numbers_and_exits_only_one_empty_level() {
        for (source, expected) in [
            ("5. A\n1. B", "5. A\n1. B\n7. "),
            ("- [x] Done", "- [x] Done\n- [ ] "),
            ("- A\n- ", "- A\n\n"),
            ("- A\n  - ", "- A\n- "),
            ("> - [x] Done", "> - [x] Done\n> - [ ] "),
        ] {
            assert_eq!(run_at(source, source.len(), EditIntent::ParagraphBreak), expected, "{source:?}");
        }
        assert_eq!(run_at("- Parent\n  - Child", 8, EditIntent::ParagraphBreak), "- Parent\n- \n  - Child");
    }

    #[test]
    fn ordered_indent_starts_or_joins_a_child_list_and_keeps_descendant_depth() {
        for (source, needle, expected) in [
            ("9. A\n10. B\n    - Child", "B", "9. A\n   1. B\n      - Child"),
            ("1. A\n   1. Child\n2. B", "B", "1. A\n   1. Child\n   2. B"),
            ("1. A\n\n   7. Child\n2. B", "B", "1. A\n\n   7. Child\n   8. B"),
        ] {
            let parsed = super::super::parse::fixture(source);
            let selection = DocumentSelection::caret(parsed.identity.from_byte + source.find(needle).unwrap() as u64);
            let result = tab(&parsed, selection, false).unwrap().unwrap();
            assert_eq!(edited(&parsed, &result), expected);
        }
        assert_eq!(run("1. Parent\n   - Child\n     - Grandchild", "Child", EditIntent::Indent { outdent: true }),
            "1. Parent\n1. Child\n   - Grandchild");
    }

    #[test]
    fn empty_child_stays_a_list_across_tab_enter_and_next_character() {
        let source = "- Parent\n- ";
        let parsed = super::super::parse::fixture(source);
        let result = tab(&parsed, DocumentSelection::caret(parsed.identity.to_byte), false).unwrap().unwrap();
        let indented = edited(&parsed, &result);
        assert_eq!(indented, "- Parent\n  - ");
        let child = super::super::parse::fixture(&indented);
        assert_eq!(child.nodes.iter().filter(|node| node.kind == "item").count(), 2);
        assert_eq!(run_at(&indented, indented.len(), EditIntent::ParagraphBreak), "- Parent\n- ");
        let typed = format!("{indented}正文");
        assert_eq!(super::super::parse::fixture(&typed).nodes.iter().filter(|node| node.kind == "item").count(), 2);
    }

    #[test]
    fn list_transitions_save_and_undo_redo_as_one_session_transaction() {
        use super::super::native_port::NativeWindowPort;
        use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
            .join(".artifacts/list-parity-v062-20261102/session-tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!("history-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        for (index, source) in ["1. A\n2. B", "9. A\n10. B\n    - Child", "1. Parent\n2. -", "- **中文AB**", "1. A\n2. B\n3. C"].into_iter().enumerate() {
            let port = NativeWindowPort::open(root.join(index.to_string()), "list-test".into(), None, Some(source.into())).unwrap();
            let status = port.status().unwrap();
            let parsed = ParsedWindow::parse(port.read_window(status.revision, 0..status.byte_length).unwrap(),
                "list-test", 0, WindowContext::Complete).unwrap();
            let at = if index == 3 { source.find("AB").unwrap() + 1 }
                else if matches!(index, 1 | 4) { source.find('B').unwrap() } else { source.len() };
            let before = DocumentSelection::caret(at as u64);
            let change = match index {
                0 => commands::apply_semantic(&parsed, before, EditIntent::LinePrefix("- ".into())).unwrap(),
                1 | 2 => tab(&parsed, before, false).unwrap().unwrap(),
                4 => commands::apply_semantic(&parsed, before, EditIntent::LinePrefix("1. ".into())).unwrap(),
                _ => commands::apply_semantic(&parsed, before, EditIntent::ParagraphBreak).unwrap(),
            };
            let expected = edited(&parsed, &change);
            let after = change.selection;
            let revision = port.submit(DocumentTransaction {
                base_revision: status.revision, operation_id: "list-change".into(), history_group_id: "list-change".into(),
                edits: change.edits, selection_before: before, selection_after: after, template_update: change.template_update,
            }).unwrap();
            assert_eq!(port.read_window(revision, 0..expected.len() as u64).unwrap().text, expected);
            let (undone, selection) = port.history(revision, false).unwrap();
            assert_eq!(selection, Some(before));
            assert_eq!(port.read_window(undone, 0..source.len() as u64).unwrap().text, source);
            let (redone, selection) = port.history(undone, true).unwrap();
            assert_eq!(selection, Some(after));
            assert_eq!(port.read_window(redone, 0..expected.len() as u64).unwrap().text, expected);
            let path = root.join(format!("saved-{index}.md"));
            let snapshot = port.capture_for_save(redone).unwrap();
            port.save_captured(snapshot, Some(path.clone()), false).unwrap();
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
            port.close(true).unwrap();
        }
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(base.canonicalize().unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }

    #[test]
    fn enter_exits_an_empty_formatted_item_and_clears_its_template() {
        use super::super::host::WindowHostAdapter;
        let mut host = WindowHostAdapter::new(DocumentWindow { document_id: "list-format".into(), revision: 1,
            from_byte: 0, to_byte: 2, text: "- ".into(), inline_templates: Vec::new() },
            "list-format", 0, WindowContext::Complete).unwrap();
        host.mark_layout_ready(LayoutStamp { identity: host.parsed.identity.clone(), width: 600.0,
            font_generation: 0, resource_generation: 0, zoom: 1.0 }, 24.0).unwrap();
        let styled = commands::apply_semantic(&host.parsed, DocumentSelection::caret(2),
            EditIntent::ToggleInline(super::super::parse::InlineFormat::Strong)).unwrap();
        host.queue_template_edit(styled.edits, styled.selection, "style", styled.template_update).unwrap();
        assert_eq!(&*host.parsed.source, "- ****");
        let exited = commands::apply_semantic(&host.parsed, host.selection, EditIntent::ParagraphBreak).unwrap();
        host.queue_template_edit(exited.edits, exited.selection, "enter", exited.template_update).unwrap();
        assert_eq!(&*host.parsed.source, "");
        assert!(host.parsed.context.inline_templates.is_empty());
        assert_eq!(host.selection, DocumentSelection::caret(0));
    }

    #[test]
    fn list_history_retains_the_tail_window_origin_after_other_groups() {
        use super::super::{native_port::NativeWindowPort, session_window::WindowRequest};
        use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
        let base=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
            .join(".artifacts/list-followup-20261102/history-tests");
        fs::create_dir_all(&base).unwrap();
        let root=base.join(format!("tail-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let mut source=(1..130).map(|index|format!("窗口保留段落 {index}\n\n")).collect::<String>();
        source.push_str("1. Alpha\n2. Bravo\n3. Charlie\n");
        let port=NativeWindowPort::open(root.clone(),"list-tail-test".into(),None,Some(source.clone())).unwrap();
        let revision=port.status().unwrap().revision;
        let request=WindowRequest::default();
        let loaded=port.load_group(revision,source.len() as u64,None,&request).unwrap();
        let from=loaded.input.from_byte;
        let parsed=ParsedWindow::parse(loaded.input,"tail",0,loaded.context).unwrap();
        let before=DocumentSelection::caret(source.find("Bravo").unwrap() as u64);
        let change=commands::apply_semantic(&parsed,before,EditIntent::LinePrefix("1. ".into())).unwrap();
        let revision=port.submit(DocumentTransaction {base_revision:revision,operation_id:"tail-cancel".into(),history_group_id:"tail-cancel".into(),
            edits:change.edits,selection_before:before,selection_after:change.selection,template_update:change.template_update}).unwrap();
        let mut revision=revision;
        for redo in [false,true,false,true] {
            let (next,selection)=port.history(revision,redo).unwrap();
            revision=next;
            let loaded=port.load_group(revision,from,None,&request).unwrap();
            let groups=port.window_groups(revision,&[(loaded.input.from_byte,loaded.input.to_byte)],&request).unwrap();
            assert_eq!(loaded.input.from_byte,from);
            assert!(selection.unwrap().head_byte>=loaded.input.from_byte && selection.unwrap().head_byte<=loaded.input.to_byte);
            assert_eq!(groups.last().unwrap().to,loaded.total_bytes);
        }
        port.close(true).unwrap();
        drop(port);
        let resolved=root.canonicalize().unwrap();
        assert!(resolved.starts_with(base.canonicalize().unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }
}
