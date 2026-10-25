//! 轻量高亮：Markdown 源码行着色与代码块词法着色。
//!
//! 都是逐行、无跨行状态的近似实现（跨行注释、多行字符串不识别），换来零依赖和可预测的成本；
//! 跨行上下文（是否在围栏或公式内）由块索引提供。

use crate::layout::{ColorRole, InlineStyle};
use std::ops::Range;

/// 源码行所在的块上下文。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineContext {
    Normal,
    /// 围栏代码的首尾标记行。
    FenceMarker,
    /// 围栏代码内部，附带语言供词法着色。
    Code,
    Math,
    FrontMatter,
}

type Spans = Vec<(Range<usize>, InlineStyle)>;

fn style(color: ColorRole) -> InlineStyle {
    InlineStyle { color, ..Default::default() }
}

/// Markdown 源码行的样式区间（按起点递增、互不重叠）。
pub fn markdown_line(line: &str, context: LineContext, lang: &str) -> Spans {
    match context {
        LineContext::FenceMarker => return vec![(0..line.len(), InlineStyle { mono: true, ..style(ColorRole::Marker) })],
        LineContext::Code => {
            return code_line(line, lang)
                .into_iter()
                .map(|(range, mut style)| {
                    style.mono = true;
                    (range, style)
                })
                .collect();
        }
        LineContext::Math => return vec![(0..line.len(), InlineStyle { mono: true, ..style(ColorRole::Accent) })],
        LineContext::FrontMatter => return vec![(0..line.len(), InlineStyle { mono: true, ..style(ColorRole::Muted) })],
        LineContext::Normal => {}
    }
    let mut spans = Spans::new();
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    // 标题：井号为标记色，其余加粗。
    if let Some(level) = super::blocks::atx_level(line) {
        let hashes = indent + level as usize;
        spans.push((0..hashes, style(ColorRole::Marker)));
        let mut body = inline_spans(&line[hashes..], hashes);
        for (_, style) in &mut body {
            style.bold = true;
            if style.color == ColorRole::Fg {
                style.color = ColorRole::Heading;
            }
        }
        if body.is_empty() && hashes < line.len() {
            body.push((hashes..line.len(), InlineStyle { bold: true, ..style(ColorRole::Heading) }));
        }
        fill_gaps(&mut body, hashes..line.len(), InlineStyle { bold: true, ..style(ColorRole::Heading) });
        spans.extend(body);
        return spans;
    }
    let mut at = indent;
    // 引用前缀可以多层：> > 文本。
    let mut cursor = rest;
    while let Some(after) = cursor.strip_prefix('>') {
        spans.push((at..at + 1, style(ColorRole::Marker)));
        at += 1;
        let spaces = after.len() - after.trim_start().len();
        at += spaces;
        cursor = &after[spaces..];
    }
    // 列表与任务标记。
    let marker_len = list_marker_len(cursor);
    if marker_len > 0 {
        spans.push((at..at + marker_len, style(ColorRole::Accent)));
        at += marker_len;
        cursor = &cursor[marker_len..];
        for task in ["[ ] ", "[x] ", "[X] "] {
            if cursor.starts_with(task) {
                spans.push((at..at + 3, style(ColorRole::Accent)));
                at += task.len();
                cursor = &cursor[task.len()..];
                break;
            }
        }
    }
    if super::blocks::is_table_delimiter(cursor) {
        spans.push((at..line.len(), style(ColorRole::Marker)));
        return spans;
    }
    spans.extend(inline_spans(cursor, at));
    spans
}

fn list_marker_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'-' | b'*' | b'+') && bytes[1] == b' ' {
        return 2;
    }
    let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if (1..=9).contains(&digits) && bytes.len() > digits + 1 && matches!(bytes[digits], b'.' | b')') && bytes[digits + 1] == b' ' {
        return digits + 2;
    }
    0
}

fn fill_gaps(spans: &mut Spans, range: Range<usize>, base: InlineStyle) {
    let mut filled = Spans::new();
    let mut at = range.start;
    for (span, style) in spans.drain(..) {
        if span.start > at {
            filled.push((at..span.start, base));
        }
        at = span.end;
        filled.push((span, style));
    }
    if at < range.end {
        filled.push((at..range.end, base));
    }
    *spans = filled;
}

/// 行内标记着色：强调与删除线的标记变淡、内容带样式；行内代码等宽；链接文本与地址分色；表格竖线变淡。
pub fn inline_spans(text: &str, offset: usize) -> Spans {
    let bytes = text.as_bytes();
    let mut spans = Spans::new();
    let mut index = 0;
    let marker = style(ColorRole::Marker);
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && index + 1 < bytes.len() {
            spans.push((offset + index..offset + index + 1, marker));
            index += 2;
            continue;
        }
        if byte == b'`' {
            let run = bytes[index..].iter().take_while(|byte| **byte == b'`').count();
            let fence = &text[index..index + run];
            if let Some(close) = text[index + run..].find(fence) {
                let end = index + run + close;
                spans.push((offset + index..offset + index + run, marker));
                spans.push((offset + index + run..offset + end, InlineStyle { mono: true, code_bg: true, ..style(ColorRole::InlineCode) }));
                spans.push((offset + end..offset + end + run, marker));
                index = end + run;
                continue;
            }
        }
        if (byte == b'*' || byte == b'_' || byte == b'~') && index + 1 < bytes.len() {
            let run = bytes[index..].iter().take_while(|next| **next == byte).count().min(3);
            let delimiter = &text[index..index + run];
            // CommonMark：`_` 不能在单词内部开启强调（如 NOMO_START）。
            let intraword = byte == b'_' && index > 0 && bytes[index - 1].is_ascii_alphanumeric();
            let valid = (byte != b'~' || run == 2) && !intraword;
            if valid && !text[index + run..].starts_with(' ') {
                if let Some(close) = text[index + run..].find(delimiter) {
                    let end = index + run + close;
                    // 强调不能跨越行内代码；`_` 的闭合也不能紧贴后续单词字符。
                    let crosses_code = text[index + run..end].contains('`');
                    let closes_intraword = byte == b'_' && bytes.get(end + run).is_some_and(|next| next.is_ascii_alphanumeric());
                    if close > 0 && !crosses_code && !closes_intraword {
                        let mut content = InlineStyle::default();
                        match (byte, run) {
                            (b'~', _) => content.strike = true,
                            (_, 1) => content.italic = true,
                            (_, 2) => content.bold = true,
                            _ => {
                                content.bold = true;
                                content.italic = true;
                            }
                        }
                        spans.push((offset + index..offset + index + run, marker));
                        spans.push((offset + index + run..offset + end, content));
                        spans.push((offset + end..offset + end + run, marker));
                        index = end + run;
                        continue;
                    }
                }
            }
        }
        if byte == b'[' || (byte == b'!' && bytes.get(index + 1) == Some(&b'[')) {
            let open = if byte == b'!' { index + 1 } else { index };
            if let Some(close) = text[open..].find("](") {
                let label_end = open + close;
                if let Some(url_close) = text[label_end + 2..].find(')') {
                    let end = label_end + 2 + url_close + 1;
                    spans.push((offset + index..offset + open + 1, marker));
                    spans.push((offset + open + 1..offset + label_end, InlineStyle { underline: true, ..style(ColorRole::Link) }));
                    spans.push((offset + label_end..offset + end, style(ColorRole::Muted)));
                    index = end;
                    continue;
                }
            }
        }
        if byte == b'|' {
            spans.push((offset + index..offset + index + 1, marker));
        }
        index += 1;
    }
    spans
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "def", "default", "defer", "do",
    "elif", "else", "enum", "export", "extends", "false", "fn", "for", "from", "func", "function", "go", "if", "impl",
    "import", "in", "interface", "let", "loop", "match", "mod", "module", "mut", "new", "nil", "None", "null", "package",
    "pub", "return", "self", "Self", "static", "struct", "super", "switch", "this", "throw", "trait", "true", "True",
    "False", "try", "type", "typeof", "use", "var", "void", "where", "while", "with", "yield", "lambda", "public",
    "private", "protected", "int", "string", "bool", "select", "SELECT", "FROM", "WHERE", "and", "or", "not", "echo",
    "then", "fi", "done", "local", "unsafe", "dyn", "ref", "move", "crate", "extern", "goto", "sizeof", "char", "float",
    "double", "long", "unsigned", "include", "define", "assert", "del", "global", "pass", "raise", "except", "finally",
    "is", "instanceof", "of", "readonly", "declare", "namespace", "abstract", "final", "override", "virtual",
];

/// 代码行的词法着色：注释、字符串、数字、关键字与标点。
pub fn code_line(line: &str, lang: &str) -> Spans {
    let bytes = line.as_bytes();
    let hash_comment = matches!(lang, "python" | "py" | "sh" | "bash" | "zsh" | "shell" | "ruby" | "rb" | "yaml" | "yml" | "toml" | "perl" | "r" | "makefile" | "dockerfile" | "powershell" | "ps1");
    let dash_comment = matches!(lang, "sql" | "lua" | "haskell" | "hs");
    let mut spans = Spans::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let rest = &line[index..];
        if rest.starts_with("//") || (hash_comment && byte == b'#') || (dash_comment && rest.starts_with("--")) || rest.starts_with("/*") {
            spans.push((index..line.len(), InlineStyle { italic: true, ..style(ColorRole::Comment) }));
            break;
        }
        if byte == b'"' || byte == b'\'' || byte == b'`' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end] != byte {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let end = (end + 1).min(bytes.len());
            spans.push((index..end, style(ColorRole::Str)));
            index = end;
            continue;
        }
        if byte.is_ascii_digit() && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric() && bytes[index - 1] != b'_') {
            let end = index + bytes[index..].iter().take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'.' || **byte == b'_').count();
            spans.push((index..end, style(ColorRole::Number)));
            index = end;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let end = index + bytes[index..].iter().take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_').count();
            let word = &line[index..end];
            if KEYWORDS.contains(&word) {
                spans.push((index..end, style(ColorRole::Keyword)));
            } else if bytes.get(end) == Some(&b'(') || bytes.get(end) == Some(&b'!') {
                spans.push((index..end, style(ColorRole::Number)));
            }
            index = end;
            continue;
        }
        if byte.is_ascii_punctuation() {
            spans.push((index..index + 1, style(ColorRole::Punct)));
        }
        index += line[index..].chars().next().map_or(1, char::len_utf8);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(spans: &Spans) -> bool {
        spans.windows(2).all(|pair| pair[0].0.end <= pair[1].0.start)
    }

    #[test]
    fn markdown_spans_are_ordered() {
        for line in ["# 标题 **粗**", "> - [ ] 任务 `code` [链接](http://x)", "| a | b |", "普通 *斜* ~~删~~ \\* 转义"] {
            let spans = markdown_line(line, LineContext::Normal, "");
            assert!(covered(&spans), "{line}: {spans:?}");
            assert!(spans.iter().all(|(range, _)| line.is_char_boundary(range.start) && line.is_char_boundary(range.end)));
        }
    }

    #[test]
    fn code_spans_mark_keywords_and_strings() {
        let spans = code_line("fn main() { let s = \"hi\"; // c", "rust");
        assert!(covered(&spans));
        assert!(spans.iter().any(|(range, style)| range == &(0..2) && style.color == ColorRole::Keyword));
        assert!(spans.iter().any(|(_, style)| style.color == ColorRole::Comment));
    }
}
