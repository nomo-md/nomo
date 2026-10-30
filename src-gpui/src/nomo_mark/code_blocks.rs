//! 代码块原位编辑：只处理当前窗口的围栏和局部补丁，历史仍由文档会话持有。
use super::{commands::{self, EditIntent, EditResult}, mapping::local_range, parse::ParsedWindow, protocol::*};
use std::ops::Range;

pub(crate) const LANGUAGES: &[(&str, &str, &[&str])] = &[
    ("Plain Text", "text", &["plaintext", "txt"]), ("JavaScript", "javascript", &["js"]),
    ("TypeScript", "typescript", &["ts"]), ("HTML", "html", &[]), ("CSS", "css", &[]),
    ("JSON", "json", &[]), ("Markdown", "markdown", &["md"]), ("Python", "python", &["py"]),
    ("Java", "java", &[]), ("Shell", "shell", &["sh", "bash"]), ("SQL", "sql", &[]),
    ("YAML", "yaml", &["yml"]), ("Diff", "diff", &[]), ("Mermaid", "mermaid", &[]),
    ("XML", "xml", &[]), ("C", "c", &[]), ("C++", "cpp", &["c++"]), ("C#", "csharp", &["cs", "c#"]),
    ("Go", "go", &["golang"]), ("Rust", "rust", &["rs"]), ("Ruby", "ruby", &["rb"]),
    ("PHP", "php", &[]), ("Swift", "swift", &[]), ("Kotlin", "kotlin", &["kt"]),
    ("Dart", "dart", &[]), ("Lua", "lua", &[]), ("R", "r", &[]), ("Scala", "scala", &[]),
    ("Bash", "bash", &[]), ("PowerShell", "powershell", &["ps"]), ("TOML", "toml", &[]),
    ("INI", "ini", &[]), ("GraphQL", "graphql", &["gql"]), ("Docker", "docker", &["dockerfile"]),
];

pub(crate) fn language_label(value: &str) -> &str {
    if value.is_empty() { return "code"; }
    LANGUAGES.iter().find(|(_, name, aliases)| name.eq_ignore_ascii_case(value)
        || aliases.iter().any(|alias| alias.eq_ignore_ascii_case(value))).map_or(value, |entry| entry.0)
}

pub(super) fn language_matches(entry: &(&str, &str, &[&str]), query: &str) -> bool {
    entry.0.to_ascii_lowercase().contains(query) || entry.1.contains(query)
        || entry.2.iter().any(|alias| alias.contains(query))
}

pub(super) struct FencedCode {
    pub source: usize,
    pub opening: Range<usize>,
    pub language: Range<usize>,
    pub body: Range<usize>,
    closing: Option<Range<usize>>,
    container: Range<usize>,
}

impl FencedCode {
    pub fn at(parsed: &ParsedWindow, at: usize) -> Option<Self> {
        let node = parsed.nodes.iter().find(|node| node.kind == "code" && node.source.start <= at && at <= node.source.end)?;
        let source = &*parsed.source;
        let raw = &source[node.source.clone()];
        let first_end = raw.find('\n')?;
        let first = raw[..first_end].trim_end_matches('\r');
        let indent = first.len() - first.trim_start().len();
        let marker = *first.as_bytes().get(indent)?;
        if !matches!(marker, b'`' | b'~') { return None; }
        let count = first.as_bytes()[indent..].iter().take_while(|b| **b == marker).count();
        if count < 3 { return None; }
        let opening = node.source.start + indent..node.source.start + indent + count;
        let info = &first[indent + count..];
        let language_start = opening.end + info.len() - info.trim_start().len();
        let language_end = language_start + info.trim_start().split_whitespace().next().unwrap_or("").len();
        let mut body_start = node.source.start + first_end + 1;
        let trimmed_end = node.source.start + raw.trim_end_matches(['\r', '\n']).len();
        let last_start = commands::line_start(source, trimmed_end);
        let last = &source[last_start..trimmed_end];
        // 引用／列表中的围栏前缀属于容器，不随语言和正文修改。
        let prefix = last.len() - last.trim_start_matches([' ', '\t', '>']).len();
        let tail = last[prefix..].trim_end();
        let closing = (last_start >= body_start && tail.len() >= count
            && tail.bytes().all(|b| b == marker)).then_some(last_start + prefix..last_start + prefix + tail.len());
        if at == node.source.end && closing.is_some() { return None; }
        let body_end = if closing.is_some() && last_start > body_start {
            let end = last_start - 1;
            if source[..end].ends_with('\r') { end - 1 } else { end }
        } else if let Some(close) = &closing { body_start = close.start; body_start } else { node.source.end };
        let container = closing.as_ref().map(|close| commands::line_start(source, close.start)..close.start)
            .unwrap_or_else(|| {
                let first_text = parsed.events.iter().find_map(|(event, span)| {
                    (matches!(event, pulldown_cmark::Event::Text(_)) && body_start <= span.start && span.start <= body_end).then_some(span.start)
                }).unwrap_or(body_start);
                commands::line_start(source, first_text)..first_text
            });
        if body_start < body_end && source[body_start..body_end].starts_with(&source[container.clone()]) {
            body_start += container.len();
        }
        Some(Self { source: node.source.start, opening, language: language_start..language_end, body: body_start..body_end, closing, container })
    }

    fn contains(&self, range: &Range<usize>) -> bool {
        self.body.start <= range.start && range.end <= self.body.end
    }
}

pub(super) fn input_prefix<'a>(parsed: &'a ParsedWindow, range: &Range<usize>) -> &'a str {
    FencedCode::at(parsed, range.start).filter(|code| code.contains(range))
        .map_or("", |code| &parsed.source[code.container])
}

/// 输入和键盘命令共用围栏保护；仅扫描本次变化的行，不复制整块正文。
/// 返回开围栏增长的字节数，供 IME 同步标记区间。
pub(super) fn protect_input(parsed: &ParsedWindow, result: &mut EditResult) -> usize {
    let base = parsed.identity.from_byte;
    let Some(first) = result.edits.first() else { return 0; };
    let Some(code) = FencedCode::at(parsed, first.from_byte.saturating_sub(base) as usize) else { return 0; };
    if !result.edits.iter().all(|edit| code.contains(&(edit.from_byte.saturating_sub(base) as usize..edit.to_byte.saturating_sub(base) as usize))) { return 0; }
    let source = &*parsed.source;
    let marker = source.as_bytes()[code.opening.start];
    let mut longest = 0;
    for edit in &mut result.edits {
        let from = (edit.from_byte - base) as usize;
        let to = (edit.to_byte - base) as usize;
        let start = commands::line_start(source, from).max(code.body.start);
        let end = source[to..code.body.end].find('\n').map_or(code.body.end, |n| to + n);
        let mut leading = true;
        let mut run = 0;
        for byte in source[start..from].bytes().chain(edit.inserted_text.bytes()).chain(source[to..end].bytes()) {
            if byte == b'\n' { leading = true; run = 0; }
            else if leading && matches!(byte, b' ' | b'\t' | b'\r' | b'>') && run == 0 { continue; }
            else if leading && byte == marker { run += 1; longest = longest.max(run); }
            else { leading = false; }
        }
        // 空围栏之间没有正文行时，为首次输入补上必要换行，光标仍停在输入之后。
        if code.closing.as_ref().is_some_and(|close| close.start == code.body.end)
            && to == code.body.end && !edit.inserted_text.is_empty() {
            edit.inserted_text.push_str(commands::line_ending(source, from));
            if let Some(close) = &code.closing {
                edit.inserted_text.push_str(&source[commands::line_start(source, close.start)..close.start]);
            }
        }
    }
    if longest < code.opening.len() { return 0; }
    let length = longest + 1;
    let fence = (marker as char).to_string().repeat(length);
    let delta = length - code.opening.len();
    result.edits.push(SourceEdit { from_byte: base + code.opening.start as u64, to_byte: base + code.opening.end as u64, inserted_text: fence.clone() });
    if let Some(closing) = code.closing {
        if closing.len() < length {
            result.edits.push(SourceEdit { from_byte: base + closing.start as u64, to_byte: base + closing.end as u64, inserted_text: fence });
        }
    }
    result.edits.sort_by_key(|edit| edit.from_byte);
    result.selection.anchor_byte += delta as u64;
    result.selection.head_byte += delta as u64;
    delta
}

pub(super) fn apply(parsed: &ParsedWindow, selection: DocumentSelection, intent: &EditIntent) -> Option<Result<EditResult, ControlError>> {
    if !matches!(intent, EditIntent::Replace(_) | EditIntent::Newline | EditIntent::ParagraphBreak
        | EditIntent::SoftBreak | EditIntent::Backspace | EditIntent::Delete | EditIntent::IndentWith { .. }) { return None; }
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range()).ok()?;
    let code = FencedCode::at(parsed, range.start)?;
    if !code.contains(&range) { return None; }
    let source = &*parsed.source;
    let base = parsed.identity.from_byte;
    let prefix = &source[code.container.clone()];
    if range.is_empty() && ((matches!(intent, EditIntent::Backspace) && range.start == code.body.start)
        || (matches!(intent, EditIntent::Delete) && range.end == code.body.end)) {
        if let Some(closing) = code.closing.as_ref().filter(|_| source[code.body.clone()].lines()
            .all(|line| line.strip_prefix(prefix).unwrap_or(line).trim().is_empty())) {
            // 只移除完整的空围栏；外层引用／列表前缀和块间换行仍归原容器所有。
            return Some(Ok(EditResult {
                edits: vec![SourceEdit { from_byte: base + code.source as u64, to_byte: base + closing.end as u64, inserted_text: String::new() }],
                selection: DocumentSelection::caret(base + code.source as u64), template_update: None,
            }));
        }
        return Some(Ok(EditResult { edits: Vec::new(), selection, template_update: None }));
    }
    let eol = commands::line_ending(source, range.start);
    let intent = match intent {
        EditIntent::Replace(text) if !prefix.is_empty() && text.contains('\n') =>
            EditIntent::Replace(text.replace('\n', &format!("\n{prefix}"))),
        EditIntent::Newline | EditIntent::ParagraphBreak | EditIntent::SoftBreak => {
            let start = commands::line_start(source, range.start);
            let before = source[start..range.start].strip_prefix(prefix).unwrap_or(&source[start..range.start]);
            let indent = if matches!(intent, EditIntent::SoftBreak) { "" }
                else { &before[..before.len() - before.trim_start_matches([' ', '\t']).len()] };
            EditIntent::Replace(format!("{eol}{prefix}{indent}"))
        }
        _ => intent.clone(),
    };
    // 容器前缀不属于代码正文；合并两行时一并移除它，缩进则从正文起点开始。
    let custom = match &intent {
        EditIntent::IndentWith { outdent, unit } if !prefix.is_empty() => {
            if !matches!(unit.as_str(), "  " | "    " | "\t") { return Some(Err(ControlError::InvalidRange)); }
            let edits: Vec<_> = commands::selected_lines(source, range.clone()).into_iter().map(|line| {
                let start = (line.start + prefix.len()).max(code.body.start).min(code.body.end);
                let text = &source[start..line.end.min(code.body.end).max(start)];
                let trim = if !outdent { 0 } else if text.starts_with('\t') { 1 }
                    else { text.bytes().take_while(|b| *b == b' ').count().min(unit.len()) };
                SourceEdit { from_byte: base + start as u64, to_byte: base + (start + trim) as u64,
                    inserted_text: if *outdent { String::new() } else { unit.clone() } }
            }).collect();
            Some(EditResult { selection: commands::map_selection(selection, &edits), edits, template_update: None })
        }
        EditIntent::Backspace if range.is_empty() && !prefix.is_empty()
            && range.start == commands::line_start(source, range.start) + prefix.len() => {
            let end = commands::line_start(source, range.start) - 1;
            let start = if source[..end].ends_with('\r') { end - 1 } else { end };
            Some(EditResult { edits: vec![SourceEdit { from_byte: base + start as u64, to_byte: selection.head_byte, inserted_text: String::new() }],
                selection: DocumentSelection::caret(base + start as u64), template_update: None })
        }
        EditIntent::Delete if range.is_empty() && !prefix.is_empty() && source[range.start..].starts_with(eol)
            && range.start + eol.len() + prefix.len() <= code.body.end => {
            Some(EditResult { edits: vec![SourceEdit { from_byte: selection.head_byte,
                to_byte: selection.head_byte + (eol.len() + prefix.len()) as u64, inserted_text: String::new() }], selection, template_update: None })
        }
        _ => None,
    };
    Some(custom.map_or_else(|| commands::apply(parsed, selection, intent), Ok).map(|mut result| {
        protect_input(parsed, &mut result);
        result
    }))
}
