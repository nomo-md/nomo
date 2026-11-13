//! Nomo 扩展的原文边界；临时解析投影保持字节长度，事件始终映射到原文。
use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, Tag};
use std::{borrow::Cow, collections::BTreeMap, ops::Range};

#[path = "math_delimiters.rs"]
mod math_delimiters;

/// Adapt math delimiters and Setext underlines only where the parser confirms math.
/// The projection is byte-for-byte aligned with source and lives for one parse.
pub struct MathProjection<'a> {
    source: Cow<'a, str>,
    marker: Option<char>,
    delimiters: Option<(char, char)>,
}

impl AsRef<str> for MathProjection<'_> {
    fn as_ref(&self) -> &str { &self.source }
}

impl MathProjection<'_> {
    pub fn restore<'a>(&self, event: Event<'a>) -> Event<'a> {
        let restore = |tex: pulldown_cmark::CowStr<'a>| {
            if !self.marker.is_some_and(|marker| tex.contains(marker))
                && !self.delimiters.is_some_and(|(padding, dollar)| tex.contains([padding, dollar]))
            { return tex; }
            tex.chars().filter_map(|ch| {
                if self.marker == Some(ch) { Some('=') }
                else if self.delimiters.is_some_and(|(padding, _)| ch == padding) { None }
                else if self.delimiters.is_some_and(|(_, dollar)| ch == dollar) { Some('$') }
                else { Some(ch) }
            }).collect::<String>().into()
        };
        match event {
            Event::InlineMath(tex) => Event::InlineMath(restore(tex)),
            Event::DisplayMath(tex) => Event::DisplayMath(restore(tex)),
            event => event,
        }
    }
}

pub fn project_math(source: &str, options: Options) -> MathProjection<'_> {
    project_math_with_links(source, options, |_, _| false)
}

pub fn project_math_with_links<'a>(
    source: &'a str,
    options: Options,
    resolves_link: impl Fn(&str, LinkType) -> bool,
) -> MathProjection<'a> {
    math_delimiters::project(source, options, resolves_link)
}

fn project_math_setext_with_links<'a>(
    source: &'a str,
    options: Options,
    resolves_link: impl Fn(&str, LinkType) -> bool,
) -> MathProjection<'a> {
    let unchanged = || MathProjection { source: Cow::Borrowed(source), marker: None, delimiters: None };
    if !options.contains(Options::ENABLE_MATH) || !source.contains("$$") {
        return unchanged();
    }
    // Common formulas have no standalone '=' line: avoid another parser pass.
    if !source.split(['\r', '\n']).any(|line| {
        let line = line.trim_end_matches([' ', '\t']);
        let count = line.bytes().rev().take_while(|ch| *ch == b'=').count();
        count > 0 && line[..line.len() - count].bytes()
            .all(|ch| matches!(ch, b' ' | b'\t' | b'>'))
    }) { return unchanged(); }
    // A one-byte sentinel preserves offsets, including in quotes and list items.
    // Never choose a byte already present in the user's document.
    let Some(marker) = (1u8..=8).map(char::from).find(|ch| !source.contains(*ch)) else {
        return unchanged();
    };
    let mut underlines = Vec::new();
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        if !matches!(event, Event::Start(Tag::Heading { level: pulldown_cmark::HeadingLevel::H1, .. })) {
            continue;
        }
        let raw = source[range.clone()].trim_end_matches(['\r', '\n']);
        let end = range.start + raw.trim_end_matches([' ', '\t']).len();
        let count = source[..end].bytes().rev().take_while(|ch| *ch == b'=').count();
        if count == 0 { continue; }
        let start = end - count;
        let line = source[..start].rfind(['\r', '\n']).map_or(0, |at| at + 1);
        // ATX headings ending with '=' are not Setext underlines.
        if source[line..start].bytes().all(|ch| matches!(ch, b' ' | b'\t' | b'>')) {
            underlines.push(start..end);
        }
    }
    if underlines.is_empty() { return unchanged(); }
    let mut projected = source.as_bytes().to_vec();
    for range in &underlines { projected[range.clone()].fill(marker as u8); }
    // Replacing ASCII with ASCII preserves UTF-8 and every original byte offset.
    let mut projected = String::from_utf8(projected).expect("ASCII-only math projection");
    let mut rejected = Vec::new();
    loop {
        {
            // 链接解析会消费引用标签中的定界符，公式校验必须沿用调用方的链接解析条件。
            let parser = Parser::new_with_broken_link_callback(
                &projected,
                options,
                Some(|link: BrokenLink<'_>| {
                    resolves_link(&link.reference, link.link_type).then_some(("".into(), "".into()))
                }),
            );
            let mut math = parser
                .into_offset_iter()
                .filter_map(|(event, range)| matches!(event, Event::DisplayMath(_)).then_some(range))
                .peekable();
            underlines.retain(|range| {
                while math.peek().is_some_and(|span| span.end <= range.start) {
                    math.next();
                }
                let inside_math = math
                    .peek()
                    .is_some_and(|span| span.start <= range.start && range.end <= span.end);
                if !inside_math {
                    rejected.push(range.clone());
                }
                inside_math
            });
        }
        if underlines.is_empty() {
            return unchanged();
        }
        if rejected.is_empty() {
            break;
        }
        // 恢复标题可能改变容器边界。每轮只撤回替换，直到最终投影中的标记都属于公式。
        for range in rejected.drain(..) {
            projected.replace_range(range.clone(), &source[range]);
        }
    }
    MathProjection { source: Cow::Owned(projected), marker: Some(marker), delimiters: None }
}

#[cfg(test)]
mod math_setext_tests {
    use super::*;
    use pulldown_cmark::TagEnd;

    fn parse(source: &str) -> Vec<(Event<'static>, Range<usize>)> {
        let projected = project_math(source, Options::ENABLE_MATH);
        assert_eq!(projected.as_ref().len(), source.len());
        Parser::new_ext(projected.as_ref(), Options::ENABLE_MATH).into_offset_iter()
            .map(|(event, range)| (projected.restore(event).into_static(), range)).collect()
    }

    #[test]
    fn restores_formula_and_source_offsets() {
        for newline in ["\n", "\r\n", "\r"] {
            let source = ["$$", "p_{\\theta,v}(z_v\\mid K,H,z_{<v})", "=", "\\frac{a}{b}", "$$", ""]
                .join(newline);
            let events = parse(&source);
            let math: Vec<_> = events
                .iter()
                .filter_map(|(event, range)| match event {
                    Event::DisplayMath(tex) => Some((tex.as_ref(), range.clone())),
                    _ => None,
                })
                .collect();
            let end = source.len() - newline.len();
            assert_eq!(math, vec![(&source[2..end - 2], 0..end)], "{source:?}");
            assert!(!events.iter().any(|(event, _)| matches!(event, Event::Start(Tag::Heading { .. }))));
        }
    }

    #[test]
    fn restoring_headings_does_not_move_math_across_quote_boundaries() {
        let formula = "$$\nx\n=\ny\n$$";
        let quotes = "> A\n> =\n$$\n> B\n> =\n> $$\n";
        let source = format!("{formula}\n\n{quotes}");
        let quotes_start = formula.len() + 2;
        let original: Vec<_> = Parser::new_ext(&source, Options::ENABLE_MATH)
            .into_offset_iter()
            .filter(|(_, range)| range.start >= quotes_start)
            .map(|(event, range)| (event.into_static(), range))
            .collect();
        let events = parse(&source);
        let quote_events = events.partition_point(|(_, range)| range.start < quotes_start);
        assert_eq!(events[quote_events..], original);

        let math: Vec<_> = events
            .iter()
            .filter_map(|(event, range)| match event {
                Event::DisplayMath(tex) => Some((tex.as_ref(), range.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(math, vec![("\nx\n=\ny\n", 0..formula.len())]);
    }

    #[test]
    fn preserves_headings_code_and_unclosed_math() {
        for source in [
            "标题\n===\n\n$$x$$\n", "# 标题 =\n\n$$x$$\n",
            "\u{60}\u{60}\u{60}\n$$\nx\n=\ny\n$$\n\u{60}\u{60}\u{60}\n", "    $$\n    x\n    =\n    y\n    $$\n",
            "$$\nx\n=\ny\n", "\u{60}$$ x\u{60} 标题\n=\n",
            "<pre>\n$$\nx\n=\ny\n$$\n</pre>\n",
            "\\$\\$\nx\n=\ny\n$$\n",
        ] {
            let original: Vec<_> = Parser::new_ext(source, Options::ENABLE_MATH).into_offset_iter()
                .map(|(event, range)| (event.into_static(), range)).collect();
            assert_eq!(parse(source), original, "{source:?}");
        }
    }

    #[test]
    fn supports_containers_multiple_equals_and_crlf() {
        for source in [
            "$$\nx\n=\ny\n=\nz\n$$\n",
            "$$\r\n中文 x\r\n=== \t\r\ny\r\n$$\r\n",
            "> $$\n> x\n> =\n> y\n> $$\n",
            "- $$\n  x\n  =\n  y\n  $$\n",
            "$$\nx\n=\ny\n$$\n\n标题\n===\n",
        ] {
            let events = parse(source);
            let math: Vec<_> = events.iter().filter_map(|(event, _)| match event {
                Event::DisplayMath(tex) => Some(tex.as_ref()), _ => None,
            }).collect();
            assert_eq!(math.len(), 1, "{source:?}");
            assert!(math[0].contains('='), "{source:?}");
            assert!(!math[0].chars().any(|ch| (1..=8).contains(&(ch as u32))));
            let starts = events.iter().filter(|(e, _)| matches!(e, Event::Start(_))).count();
            let ends = events.iter().filter(|(e, _)| matches!(e, Event::End(_))).count();
            assert_eq!(starts, ends);
        }
    }

    #[test]
    fn preserves_user_control_characters_and_disabled_math() {
        let source = "\u{1}\n\n$$\nx\n=\ny\n$$\n";
        let events = parse(source);
        assert!(events.iter().any(|(e, _)| matches!(e, Event::Text(t) if t.contains('\u{1}'))));
        assert!(events.iter().any(|(e, _)| matches!(e, Event::DisplayMath(t) if t.contains("\n=\n"))));
        let projected = project_math(source, Options::empty());
        assert!(matches!(projected.source, Cow::Borrowed(_)));
        assert_eq!(projected.as_ref(), source);
        let occupied = format!("{}\n{source}", (1u8..=8).map(char::from).collect::<String>());
        assert_eq!(project_math(&occupied, Options::ENABLE_MATH).as_ref(), occupied);
    }

    #[test]
    fn ordinary_input_borrows_source() {
        for source in ["# Heading\ntext\n", "$$\nx = y\n$$\n", "title\n=\n\n$$x$$"] {
            assert!(matches!(project_math(source, Options::ENABLE_MATH).source, Cow::Borrowed(_)));
        }
        assert!(parse("title\n=\n").iter().any(|(e, _)| matches!(e, Event::End(TagEnd::Heading(_)))));
    }
}


/// 单行注释后还有正文时按段落解析，独立注释、目录标记和代码保持原有语义。
pub fn project_comment_paragraphs(source: &str, options: Options) -> Cow<'_, str> {
    let mut projected = Cow::Borrowed(source);
    if !source.contains("<!--") {
        return projected;
    }
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        if !matches!(event, Event::Start(Tag::HtmlBlock)) {
            continue;
        }
        let raw = source[range.clone()].trim_end_matches(['\r', '\n']);
        if !raw.starts_with("<!--") || raw.contains(['\r', '\n']) {
            continue;
        }
        let Some(close) = raw[4..].find("-->").map(|at| at + 4) else { continue; };
        let end = close + 3;
        let comment = raw[4..close].trim();
        if raw[end..].trim().is_empty()
            || comment.eq_ignore_ascii_case("toc") || comment.eq_ignore_ascii_case("/toc")
        {
            continue;
        }
        // 等长的行内标签让解析器保留正文格式和引用；随后恢复注释事件，不改写文档。
        let marker = format!("<span{}>", " ".repeat(end - 6));
        projected.to_mut().replace_range(range.start..range.start + end, &marker);
    }
    projected
}

pub fn restore_inline_comment<'a>(source: &'a str, event: Event<'a>, range: &Range<usize>) -> Event<'a> {
    if matches!(event, Event::InlineHtml(_)) && source[range.clone()].starts_with("<!--") {
        Event::InlineHtml(source[range.clone()].into())
    } else {
        event
    }
}

#[derive(Debug)]
pub struct HtmlTag {
    pub name: String,
    pub closing: bool,
    pub self_closing: bool,
    pub end: usize,
    pub attrs: BTreeMap<String, String>,
    /// 链接 href 值的原文字节范围，不含引号。
    pub href_range: Option<Range<usize>>,
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
    let mut href_range = None;
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
                href_range,
            });
        }
        if bytes.get(at..at + 2) == Some(b"/>") {
            return Some(HtmlTag {
                name,
                closing,
                self_closing: true,
                end: at + 2,
                attrs,
                href_range,
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
        let value_range = if bytes.get(at) == Some(&b'=') {
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
                let value = from..at;
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
                from..at
            }
        } else {
            at..at
        };
        if name == "a" && key == "href" { href_range = Some(value_range.clone()); }
        attrs.insert(key, html_escape::decode_html_entities(&source[value_range]).into_owned());
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
