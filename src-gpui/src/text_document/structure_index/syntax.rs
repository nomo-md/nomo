//! Nomo 扩展的原文边界。只返回原文范围，不通过全文替换制造虚假的字节坐标。
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct HtmlTag {
    pub name: String,
    pub closing: bool,
    pub self_closing: bool,
    pub end: usize,
    pub attrs: BTreeMap<String, String>,
}

pub fn html_tag(source: &str, start: usize) -> Option<HtmlTag> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut at = start + 1;
    let closing = bytes.get(at) == Some(&b'/');
    at += usize::from(closing);
    let name_start = at;
    while bytes
        .get(at)
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == b'-')
    {
        at += 1;
    }
    if at == name_start || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let name = source[name_start..at].to_ascii_lowercase();
    let mut attrs = BTreeMap::new();
    loop {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) == Some(&b'>') {
            return Some(HtmlTag {
                name,
                closing,
                self_closing: false,
                end: at + 1,
                attrs,
            });
        }
        if bytes.get(at..at + 2) == Some(b"/>") {
            return Some(HtmlTag {
                name,
                closing,
                self_closing: true,
                end: at + 2,
                attrs,
            });
        }
        let key_start = at;
        while bytes
            .get(at)
            .is_some_and(|ch| !ch.is_ascii_whitespace() && !b"\"'<>/=".contains(ch))
        {
            at += 1;
        }
        if at == key_start {
            return None;
        }
        let key = source[key_start..at].to_ascii_lowercase();
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let value = if bytes.get(at) == Some(&b'=') {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let quote = *bytes.get(at)?;
            if quote == b'\'' || quote == b'"' {
                at += 1;
                let from = at;
                while *bytes.get(at)? != quote {
                    at += 1;
                }
                let value = &source[from..at];
                at += 1;
                value
            } else {
                let from = at;
                while bytes
                    .get(at)
                    .is_some_and(|ch| !ch.is_ascii_whitespace() && *ch != b'>')
                {
                    at += 1;
                }
                if at == from {
                    return None;
                }
                &source[from..at]
            }
        } else {
            ""
        };
        attrs.insert(key, html_escape::decode_html_entities(value).into_owned());
    }
}

pub fn line_end(source: &str, from: usize) -> usize {
    let bytes = source.as_bytes();
    let mut at = from;
    while at < bytes.len() && !b"\r\n".contains(&bytes[at]) {
        at += 1;
    }
    if bytes.get(at) == Some(&b'\r') {
        at += 1;
    }
    if bytes.get(at) == Some(&b'\n') {
        at += 1;
    }
    at
}

pub fn front_matter_end(source: &str) -> Option<usize> {
    let newline = if source.starts_with("---\r\n") {
        "\r\n"
    } else if source.starts_with("---\n") {
        "\n"
    } else {
        return None;
    };
    let delimiter = format!("{newline}---{newline}");
    source[3..]
        .find(&delimiter)
        .map(|start| start + 3 + delimiter.len())
}

fn closing_tag(source: &str, name: &str, from: usize) -> Option<(usize, usize)> {
    let mut at = from;
    while let Some(next) = source[at..].find('<') {
        at += next;
        if let Some(tag) = html_tag(source, at) {
            if tag.closing && tag.name == name {
                return Some((at, tag.end));
            }
            at = tag.end;
        } else {
            at += 1;
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct Details {
    pub body_from: usize,
    pub body_to: usize,
    pub end: usize,
}

/// 对齐编辑器的 Details 闭合规则，排除围栏、代码、注释、原始 HTML 字面区域。
pub fn details(source: &str, start: usize) -> Option<Details> {
    let opening = html_tag(source, start)?;
    if opening.name != "details"
        || opening.closing
        || opening.self_closing
        || opening.attrs.keys().any(|key| key != "open")
    {
        return None;
    }
    let mut cursor = opening.end;
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    let summary = html_tag(source, cursor)?;
    if summary.name != "summary"
        || summary.closing
        || summary.self_closing
        || !summary.attrs.is_empty()
    {
        return None;
    }
    let (summary_end, body_from) = closing_tag(source, "summary", summary.end)?;
    if !editable_summary(&source[summary.end..summary_end]) {
        return None;
    }
    cursor = body_from;
    let mut depth = 1usize;
    let mut fence: Option<(u8, usize, usize, usize)> = None;
    let mut container_indents = vec![0usize];
    let mut line_container_indent = 0;
    while cursor < source.len() {
        let bytes = source.as_bytes();
        if bytes[cursor] == b'\\' {
            cursor += 1;
            if let Some(ch) = source[cursor..].chars().next() {
                cursor += ch.len_utf8();
            }
            continue;
        }
        if cursor == 0 || b"\r\n".contains(&bytes[cursor - 1]) {
            let end = line_end(source, cursor);
            let mut line = source[cursor..end].trim_end_matches(['\r', '\n']);
            let mut quotes = 0;
            loop {
                let trimmed = line.trim_start_matches(' ');
                if line.len() - trimmed.len() <= 3 && trimmed.starts_with('>') {
                    line = &trimmed[1..];
                    if line.starts_with([' ', '\t']) {
                        line = &line[1..];
                    }
                    quotes += 1;
                } else {
                    break;
                }
            }
            let indent = *container_indents.last().unwrap_or(&0);
            if line.starts_with(&" ".repeat(indent)) {
                line = &line[indent..];
            }
            let spaces = line.bytes().take_while(|ch| *ch == b' ').count();
            let mut list_indent = 0;
            let plain = &line[spaces..];
            if plain
                .as_bytes()
                .first()
                .is_some_and(|ch| b"-+*".contains(ch))
                && plain.as_bytes().get(1).is_some_and(u8::is_ascii_whitespace)
            {
                list_indent = spaces
                    + 1
                    + plain[1..]
                        .bytes()
                        .take_while(u8::is_ascii_whitespace)
                        .count();
            } else {
                let digits = plain.bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0
                    && plain
                        .as_bytes()
                        .get(digits)
                        .is_some_and(|ch| b".)".contains(ch))
                    && plain
                        .as_bytes()
                        .get(digits + 1)
                        .is_some_and(u8::is_ascii_whitespace)
                {
                    list_indent = spaces
                        + digits
                        + 1
                        + plain[digits + 1..]
                            .bytes()
                            .take_while(u8::is_ascii_whitespace)
                            .count();
                }
            }
            line_container_indent = indent + list_indent;
            if let Some((marker, length, quote_depth, nested_indent)) = fence {
                if quotes < quote_depth || (!line.trim().is_empty() && spaces < nested_indent) {
                    fence = None;
                } else {
                    let trimmed = line.trim();
                    let run = trimmed.bytes().take_while(|ch| *ch == marker).count();
                    if run >= length && trimmed[run..].trim().is_empty() {
                        fence = None;
                    }
                    cursor = end;
                    continue;
                }
            }
            let fence_line = &line[list_indent..];
            let trimmed = fence_line.trim_start_matches(' ');
            let marker = trimmed.as_bytes().first().copied().unwrap_or(0);
            let run = trimmed.bytes().take_while(|ch| *ch == marker).count();
            if fence_line.len() - trimmed.len() <= 3
                && b"`~".contains(&marker)
                && run >= 3
                && (marker != b'`' || !trimmed[run..].contains('`'))
            {
                fence = Some((marker, run, quotes, list_indent));
                cursor = end;
                continue;
            }
            if line.starts_with("    ") || line.starts_with('\t') {
                cursor = end;
                continue;
            }
        }
        if source[cursor..].starts_with("<!--") {
            cursor += source[cursor + 4..].find("-->")? + 7;
            continue;
        }
        if bytes[cursor] == b'`' {
            let original = cursor;
            let length = source[cursor..]
                .bytes()
                .take_while(|ch| *ch == b'`')
                .count();
            let delimiter = "`".repeat(length);
            let mut search = cursor + length;
            while let Some(found) = source[search..].find(&delimiter) {
                let found = search + found;
                if source.as_bytes().get(found.wrapping_sub(1)) != Some(&b'`')
                    && source.as_bytes().get(found + length) != Some(&b'`')
                {
                    if !source[cursor..found].contains("\n\n") {
                        cursor = found + length;
                    }
                    break;
                }
                search = found + length;
            }
            if cursor != original {
                continue;
            }
            cursor += length;
            continue;
        }
        if bytes[cursor] != b'<' {
            cursor += source[cursor..].chars().next()?.len_utf8();
            continue;
        }
        let Some(tag) = html_tag(source, cursor) else {
            cursor += 1;
            continue;
        };
        if !tag.closing && ["pre", "script", "style", "textarea"].contains(&tag.name.as_str()) {
            cursor = closing_tag(source, &tag.name, tag.end)?.1;
            continue;
        }
        if tag.name == "details" {
            if tag.closing {
                depth = depth.checked_sub(1)?;
                container_indents.pop();
                if depth == 0 {
                    return Some(Details {
                        body_from,
                        body_to: cursor,
                        end: tag.end,
                    });
                }
            } else {
                depth += 1;
                container_indents.push(line_container_indent);
            }
        }
        cursor = tag.end;
    }
    None
}

fn editable_summary(source: &str) -> bool {
    let mut at = 0;
    let mut stack: Vec<String> = vec![];
    while let Some(next) = source[at..].find('<') {
        at += next;
        let Some(tag) = html_tag(source, at) else {
            return false;
        };
        if ![
            "b", "strong", "i", "em", "s", "del", "u", "mark", "sub", "sup", "code", "a",
        ]
        .contains(&tag.name.as_str())
        {
            return false;
        }
        if tag.closing {
            if stack.pop().as_deref() != Some(&tag.name) {
                return false;
            }
        } else {
            if tag.self_closing
                || tag
                    .attrs
                    .keys()
                    .any(|key| tag.name != "a" || !["href", "title"].contains(&key.as_str()))
            {
                return false;
            }
            stack.push(tag.name);
        }
        at = tag.end;
    }
    stack.is_empty()
}

pub fn html_code(source: &str, start: usize) -> Option<(usize, String)> {
    let tag = html_tag(source, start)?;
    if tag.name != "code" || tag.closing {
        return None;
    }
    let (to, end) = closing_tag(source, "code", tag.end)?;
    Some((
        end,
        html_escape::decode_html_entities(&source[tag.end..to]).into_owned(),
    ))
}
