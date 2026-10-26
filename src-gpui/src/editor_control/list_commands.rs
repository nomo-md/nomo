//! 列表命令按解析后的 item/list 祖先操作，避免把子项当作独立文本行缩进。
//! 仅生成局部补丁；类型、任务状态和未选中的源码仍由原版会话保存。
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
    Some(Item {
        node,
        list,
        start,
        end: n.source.end,
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
        .find(|(_, n)| n.kind == "item" && n.source.start <= at && at <= n.source.end)
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
fn edit(parsed: &ParsedWindow, range: Range<usize>, text: String) -> SourceEdit {
    SourceEdit {
        from_byte: parsed.identity.from_byte + range.start as u64,
        to_byte: parsed.identity.from_byte + range.end as u64,
        inserted_text: text,
    }
}
fn result(selection: DocumentSelection, mut edits: Vec<SourceEdit>) -> EditResult {
    edits.sort_by_key(|e| (e.from_byte, e.to_byte));
    edits.retain(|e| e.from_byte != e.to_byte || !e.inserted_text.is_empty());
    EditResult {
        selection: commands::map_selection(selection, &edits),
        edits,
        template_update: None,
    }
}
fn line_end(source: &str, from: usize) -> usize {
    source[from..]
        .find(['\r', '\n'])
        .map_or(source.len(), |i| from + i)
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
            if clear_empty_task && it.task {
                edits.push(edit(parsed, marker_end..it.body, String::new()));
            }
        }
    } else {
        for it in selected {
            let eol = commands::line_ending(&parsed.source, it.start);
            // 退出顶层列表时移除任务符号，保留子任务。空行阻止 Markdown 懒续行吞并。
            let before = if it.start > parsed.nodes[it.list].source.start {
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
            let from = peers.iter().position(|p| p.node == it.node).unwrap();
            let selected: Vec<_> = peers
                .iter()
                .skip(from)
                .take_while(|p| p.start < range.end.max(range.start + 1))
                .cloned()
                .collect();
            let ordered = source.as_bytes()[it.marker].is_ascii_digit();
            if prefix != "- [ ] " && ordered == (prefix == "1. ") {
                return Some(lift(parsed, &selected, selection, false));
            }
            let remove_tasks = prefix == "- [ ] " && selected.iter().all(|p| p.task);
            let mut edits = Vec::new();
            for (i, entry) in selected.iter().enumerate() {
                let marker_end = entry.marker_end;
                if prefix == "- [ ] " {
                    if remove_tasks {
                        edits.push(edit(parsed, marker_end..entry.body, String::new()));
                    } else if !entry.task {
                        edits.push(edit(parsed, entry.body..entry.body, "[ ] ".into()));
                    }
                } else {
                    edits.push(edit(
                        parsed,
                        entry.marker..marker_end,
                        if prefix == "1. " {
                            format!("{}. ", i + 1)
                        } else {
                            "- ".into()
                        },
                    ));
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
            for entry in &selected {
                if let Err(error) = shifted_lines(parsed, entry, width as isize, &mut edits) {
                    return Some(Err(error));
                }
            }
            Some(Ok(result(selection, edits)))
        }
        EditIntent::Newline | EditIntent::ParagraphBreak
            if range.start >= it.body && range.end <= line_end(source, it.body) =>
        {
            let end = line_end(source, it.body);
            let has_children = parsed
                .nodes
                .iter()
                .any(|n| n.parent == Some(it.node) && n.kind != "paragraph");
            if source[it.body..end].trim().is_empty() && !has_children {
                return Some(lift(parsed, &[it], selection, true));
            }
            let mut prefix = source[it.start..it.body].to_string();
            if it.task {
                let at = it.marker_end - it.start + 1;
                prefix.replace_range(at..at + 1, " ");
            }
            // 有序列表续项从当前项编号递增；未改动的同级标记保持原文。
            let offset = it.marker - it.start;
            let digits = prefix[offset..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if digits > 0 {
                if let Ok(number) = prefix[offset..offset + digits].parse::<u64>() {
                    prefix.replace_range(
                        offset..offset + digits,
                        &number.saturating_add(1).to_string(),
                    );
                }
            }
            Some(Ok(result(
                selection,
                vec![edit(
                    parsed,
                    range.clone(),
                    format!("{}{prefix}", commands::line_ending(source, range.start)),
                )],
            )))
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
                            line_end(source, peers[at - 1].body)
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
                let previous_end = line_end(source, previous.body);
                if source[previous_end..it.start].trim().is_empty() {
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
        let mut text = source.to_string();
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
    fn list_type_and_tasks_touch_only_selected_items() {
        assert_eq!(
            run(
                "1. first\n2. second\n   - child\n3. last\n",
                "second",
                EditIntent::LinePrefix("- ".into())
            ),
            "1. first\n- second\n   - child\n3. last\n"
        );
        assert_eq!(
            run(
                "- [x] first\n  - [x] child\n- last\n",
                "first",
                EditIntent::LinePrefix("- [ ] ".into())
            ),
            "- first\n  - [x] child\n- last\n"
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
}
