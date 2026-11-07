//! markdownlint 0.41.1 的固定默认参数；不提供修复、不读取外部配置、不访问链接。
use super::{
    scanner::{Syntax, checkpoint},
    *,
};
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Tag, TagEnd};
use regex::Regex;
use std::{collections::HashSet, sync::OnceLock, time::Instant};

pub(super) fn check(
    s: &Syntax<'_>,
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    let mut ends = vec![0; s.events.len()];
    let mut stack = Vec::new();
    for (i, (event, _)) in s.events.iter().enumerate() {
        match event {
            Event::Start(_) => stack.push(i),
            Event::End(_) => {
                if let Some(begin) = stack.pop() {
                    ends[begin] = i;
                }
            }
            _ => {}
        }
    }
    super::advanced::check(s, &ends, out, request, started)?;
    let mut code_content = s.code.clone();
    let mut ignored_tab_lines = vec![false; s.lines.len()];
    let ignored_languages = s
        .config
        .strings(10, "ignore_code_languages")
        .unwrap_or_default();
    for (event, range) in &s.events {
        if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))) = event {
            code_content[s.line_at(range.start)] = false;
            let last = s.last_line(range);
            if fence_start(s.line(last)).is_some() {
                code_content[last] = false;
            }
            if ignored_languages.contains(&language.split_whitespace().next().unwrap_or("")) {
                ignored_tab_lines[s.line_at(range.start)..=last].fill(true);
            }
        }
    }
    let br_spaces = s.config.number(9, "br_spaces", 2);
    let br_spaces = if br_spaces < 2 { 0 } else { br_spaces };
    let maximum_blanks = s.config.number(12, "maximum", 1);
    let mut blanks = 0;
    for line in s.body_line..s.lines.len() {
        checkpoint(request, started)?;
        let text = s.line(line);
        let range = s.lines[line].clone();
        if !code_content[line] || s.config.boolean(9, "code_blocks", false) {
            let trimmed = text.trim_end();
            let count = text[trimmed.len()..].encode_utf16().count();
            if count > 0 && count != br_spaces && !s.config.in_comment(range.start + trimmed.len())
            {
                out.add(
                    9,
                    range.start + trimmed.len()..range.end,
                    format!("Expected: 0 or {br_spaces}; Actual: {count}"),
                );
            }
        }
        if !ignored_tab_lines[line] && (!s.code[line] || s.config.boolean(10, "code_blocks", true))
        {
            let mut tabs = text
                .match_indices('\t')
                .map(|(index, _)| index)
                .filter(|index| !s.config.in_comment(range.start + index));
            if let Some(first) = tabs.next() {
                let end = tabs.next_back().unwrap_or(first) + 1;
                out.add(10, range.start + first..range.start + end, "");
            }
        }
        blanks = if s.code[line] || !text.trim().is_empty() {
            0
        } else {
            blanks + 1
        };
        if blanks > maximum_blanks {
            out.add(
                12,
                range.clone(),
                format!("Expected: {maximum_blanks}; Actual: {blanks}"),
            );
        }
        if !s.code[line] && !s.html[line] {
            malformed_heading(text, range.start, out);
        }
    }
    let heading_setting = s.config.string(3, "style", "consistent");
    let mut heading_style = match heading_setting {
        "atx" => Some(0),
        "atx_closed" => Some(1),
        "setext" => Some(2),
        _ => None,
    };
    let mut headings = HashSet::new();
    let mut previous_level = if s.title { 1 } else { 6 };
    let mut title_count = usize::from(s.title);
    let mut first_content = None;
    let mut bullet_style = match s.config.string(4, "style", "consistent") {
        "asterisk" => Some('*'),
        "dash" => Some('-'),
        "plus" => Some('+'),
        _ => None,
    };
    let mut nesting_styles: Vec<Option<char>> = Vec::new();
    let mut code_style = match s.config.string(46, "style", "consistent") {
        "fenced" => Some(true),
        "indented" => Some(false),
        _ => None,
    };
    let mut fence_style = match s.config.string(48, "style", "consistent") {
        "backtick" => Some('`'),
        "tilde" => Some('~'),
        _ => None,
    };
    let mut emphasis_style = None;
    let mut strong_style = None;
    let configured_hr = s.config.string(35, "style", "consistent");
    let mut rule_style = (configured_hr != "consistent").then_some(configured_hr);
    let mut pipe_style = match s.config.string(55, "style", "consistent") {
        "leading_and_trailing" => Some((true, true)),
        "leading_only" => Some((true, false)),
        "trailing_only" => Some((false, true)),
        "no_leading_or_trailing" => Some((false, false)),
        _ => None,
    };
    let mut lists = Vec::new();
    let mut quote_depth = 0;
    let mut link_depth = 0;
    let mut code_depth = 0;
    let prohibited_texts = s
        .config
        .strings(59, "prohibited_texts")
        .unwrap_or_else(|| vec!["click here", "here", "link", "more"])
        .into_iter()
        .map(normalized_link_text)
        .collect::<HashSet<_>>();
    let allowed_languages = s
        .config
        .strings(40, "allowed_languages")
        .unwrap_or_default();
    for (i, (event, range)) in s.events.iter().enumerate() {
        checkpoint(request, started)?;
        let line = s.line_at(range.start);
        match event {
            Event::Start(tag) => {
                if first_content.is_none() {
                    let first = match tag {
                        Tag::Heading { level, .. } => Some(*level as u8 == 1),
                        Tag::Paragraph
                        | Tag::List(_)
                        | Tag::BlockQuote(_)
                        | Tag::CodeBlock(_)
                        | Tag::Table(_) => Some(false),
                        Tag::HtmlBlock
                            if !s.source[range.clone()].trim_start().starts_with("<!--") =>
                        {
                            Some(
                                s.source[range.clone()]
                                    .trim_start()
                                    .to_ascii_lowercase()
                                    .starts_with("<h1"),
                            )
                        }
                        _ => None,
                    };
                    if let Some(valid) = first {
                        first_content = Some((range.clone(), valid));
                    }
                }
                match tag {
                    Tag::Heading { level, .. } => {
                        let level = *level as u8;
                        let (text, text_range, style) = heading_text(s, range);
                        if level > previous_level + 1 {
                            out.add(
                                1,
                                range.clone(),
                                format!("Expected: h{}; Actual: h{level}", previous_level + 1),
                            );
                        }
                        previous_level = level;
                        if heading_setting.starts_with("setext_with_atx") {
                            let wanted = if level <= 2 {
                                2
                            } else {
                                u8::from(heading_setting.ends_with("closed"))
                            };
                            if style != wanted {
                                out.add(3, range.clone(), "");
                            }
                        } else {
                            consistent(3, style, &mut heading_style, range.clone(), out);
                        }
                        blank_around(22, s, range, out);
                        if lists.is_empty() {
                            let prefix = &s.source[s.lines[line].start..range.start];
                            let after_quote = prefix.rsplit('>').next().unwrap_or(prefix);
                            let allowed = if prefix.contains('>') { 1 } else { 0 };
                            if after_quote.len() > allowed && after_quote.trim().is_empty() {
                                out.add(
                                    23,
                                    range.start - after_quote.len() + allowed..range.start,
                                    "",
                                );
                            }
                        }
                        if !headings.insert(text.to_owned()) {
                            out.add(24, text_range.clone(), "");
                        }
                        if level == 1 {
                            if title_count > 0 {
                                out.add(25, text_range.clone(), "");
                            }
                            if s.title
                                || first_content.as_ref().is_some_and(|(first, valid)| {
                                    *valid && first.start == range.start
                                })
                            {
                                title_count += 1;
                            }
                        }
                        let punctuation = s.config.string(26, "punctuation", ".,;:!。，；：！");
                        let trimmed = text.trim_end_matches(|c| punctuation.contains(c));
                        if trimmed.len() != text.len() && !entity_or_emoji_suffix(text) {
                            out.add(26, text_range.start + trimmed.len()..text_range.end, "");
                        }
                        heading_spaces(s, range, out);
                    }
                    Tag::List(ordered) => lists.push(ordered.is_some()),
                    Tag::Item if lists.last() == Some(&false) => {
                        if let Some(marker) = s.source[range.clone()]
                            .chars()
                            .next()
                            .filter(|c| "-*+".contains(*c))
                        {
                            let expected = if s.config.string(4, "style", "consistent") == "sublist"
                            {
                                let depth = lists.len() - 1;
                                nesting_styles.resize(nesting_styles.len().max(depth + 1), None);
                                let parent = depth
                                    .checked_sub(1)
                                    .and_then(|parent| nesting_styles[parent]);
                                nesting_styles[depth].get_or_insert(if parent == Some(marker) {
                                    match marker {
                                        '-' => '+',
                                        '+' => '*',
                                        _ => '-',
                                    }
                                } else {
                                    marker
                                });
                                &mut nesting_styles[depth]
                            } else {
                                &mut bullet_style
                            };
                            consistent(4, marker, expected, range.start..range.start + 1, out);
                        }
                    }
                    Tag::BlockQuote(_) => quote_depth += 1,
                    Tag::CodeBlock(kind) => {
                        code_depth += 1;
                        let fenced = matches!(kind, CodeBlockKind::Fenced(_));
                        consistent(46, fenced, &mut code_style, range.clone(), out);
                        if let CodeBlockKind::Fenced(language) = kind {
                            if lists.is_empty() || s.config.boolean(31, "list_items", true) {
                                blank_around(31, s, range, out);
                            }
                            let language_name = language.split_whitespace().next().unwrap_or("");
                            if language_name.is_empty()
                                || (!allowed_languages.is_empty()
                                    && !allowed_languages.contains(&language_name))
                            {
                                out.add(40, s.lines[line].clone(), "");
                            }
                            if s.config.boolean(40, "language_only", false)
                                && language.trim() != language_name
                            {
                                out.add(
                                    40,
                                    s.lines[line].clone(),
                                    "Info string contains more than language",
                                );
                            }
                            if let Some((_, marker)) = fence_start(s.line(line)) {
                                consistent(
                                    48,
                                    marker,
                                    &mut fence_style,
                                    s.lines[line].clone(),
                                    out,
                                );
                            }
                        }
                        let content: Vec<&str> = s.events[i + 1..ends[i]]
                            .iter()
                            .filter_map(|(event, _)| {
                                if let Event::Text(text) = event {
                                    Some(text.as_ref())
                                } else {
                                    None
                                }
                            })
                            .flat_map(str::lines)
                            .filter(|line| !line.trim().is_empty())
                            .collect();
                        if !content.is_empty()
                            && content
                                .iter()
                                .all(|line| line.trim_start().starts_with("$ "))
                        {
                            for (event, body) in &s.events[i + 1..ends[i]] {
                                if matches!(event, Event::Text(_)) {
                                    for row in s.line_at(body.start)..=s.last_line(body) {
                                        if let Some(at) = s.line(row).find("$ ") {
                                            out.add(
                                                14,
                                                s.lines[row].start + at
                                                    ..s.lines[row].start + at + 2,
                                                "",
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Tag::Emphasis => {
                        emphasis_style_check(49, 1, s, range, &mut emphasis_style, out);
                    }
                    Tag::Strong => {
                        emphasis_style_check(50, 2, s, range, &mut strong_style, out);
                    }
                    Tag::Paragraph if lists.is_empty() && quote_depth == 0 => {
                        emphasis_heading(s, i, ends[i], range, out);
                    }
                    Tag::Link {
                        link_type,
                        dest_url,
                        ..
                    }
                    | Tag::Image {
                        link_type,
                        dest_url,
                        ..
                    } => {
                        link_depth += 1;
                        let image = matches!(tag, Tag::Image { .. });
                        let style = match link_type {
                            LinkType::Autolink | LinkType::Email => "autolink",
                            LinkType::Inline => "inline",
                            LinkType::Reference | LinkType::ReferenceUnknown => "full",
                            LinkType::Collapsed | LinkType::CollapsedUnknown => "collapsed",
                            _ => "shortcut",
                        };
                        if !dest_url.is_empty() && !s.config.boolean(54, style, true) {
                            out.add(54, range.clone(), format!("Style: {style}"));
                        }
                        if !image && (dest_url.is_empty() || dest_url.as_ref() == "#") {
                            out.add(42, range.clone(), "");
                        }
                        if !matches!(link_type, LinkType::Autolink | LinkType::Email) {
                            if let Some(label) = link_label(s.source, range) {
                                let raw_label = &s.source[label.clone()];
                                if !image
                                    && *link_type == LinkType::Inline
                                    && s.config.boolean(54, "autolink", true)
                                    && s.config.boolean(54, "inline", true)
                                    && !s.config.boolean(54, "url_inline", true)
                                    && raw_label == dest_url.as_ref()
                                    && !dest_url.contains([' ', '<', '>'])
                                    && url::Url::parse(dest_url).is_ok()
                                    && matches!(tag, Tag::Link { title, .. } if title.is_empty())
                                {
                                    out.add(54, range.clone(), "Style: url_inline");
                                }
                                if image && raw_label.is_empty() {
                                    out.add(45, range.clone(), "");
                                }
                                if !image && raw_label.trim() != raw_label {
                                    out.add(39, label.clone(), "");
                                }
                                let has_code_or_html =
                                    s.events[i + 1..ends[i]].iter().any(|(event, _)| {
                                        matches!(event, Event::Code(_) | Event::InlineHtml(_))
                                    });
                                if !image && !has_code_or_html {
                                    if prohibited_texts.contains(&normalized_link_text(raw_label)) {
                                        out.add(59, label, "");
                                    }
                                }
                            }
                        }
                    }
                    Tag::Table(_) => {
                        blank_around(58, s, range, out);
                        table(s, range, &mut pipe_style, out);
                    }
                    _ => {}
                }
            }
            Event::End(tag) => match tag {
                TagEnd::List(_) => {
                    lists.pop();
                }
                TagEnd::BlockQuote(_) => quote_depth -= 1,
                TagEnd::CodeBlock => code_depth -= 1,
                TagEnd::Link | TagEnd::Image => link_depth -= 1,
                _ => {}
            },
            Event::Code(_) => code_span(s, range, out),
            Event::Html(_) | Event::InlineHtml(_) => html(s, range, out),
            Event::Rule => {
                if first_content.is_none() {
                    first_content = Some((range.clone(), false));
                }
                let text = s.source[range.clone()].trim_end();
                consistent(35, text, &mut rule_style, range.clone(), out);
            }
            Event::Text(_) if code_depth == 0 && link_depth == 0 && !s.html[line] => {
                plain_text(s, range, out);
            }
            _ => {}
        }
    }
    if !s.title {
        if let Some((range, false)) = first_content {
            out.add(41, range, "");
        }
    }
    for range in &s.missing {
        out.add(52, range.clone(), "");
    }
    if let Some(last) = s.lines.last() {
        if !s.source[last.clone()]
            .trim_matches(|c: char| c.is_whitespace() || c == '>')
            .is_empty()
        {
            let begin = s.source[..last.end]
                .char_indices()
                .next_back()
                .map_or(last.end, |(index, _)| index);
            out.add(47, begin..last.end, "");
        }
    }
    // MD043/044 默认没有 headings/names 参数；非默认配置在指令层明确拒绝。
    Ok(())
}

fn normalized_link_text(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}
fn consistent<T: Copy + PartialEq>(
    rule: u16,
    value: T,
    expected: &mut Option<T>,
    range: Range<usize>,
    out: &mut Sink<'_>,
) {
    if expected.is_some_and(|expected| expected != value) {
        out.add(rule, range, "");
    } else {
        *expected = Some(value);
    }
}
pub(super) fn blank_around(rule: u16, s: &Syntax<'_>, range: &Range<usize>, out: &mut Sink<'_>) {
    let first = s.line_at(range.start);
    let mut last = s.last_line(range);
    while last > first && s.blank(last) {
        last -= 1;
    }
    if first > s.body_line && !s.blank(first - 1) {
        out.add(rule, s.lines[first].clone(), "Above");
    }
    if last + 1 < s.lines.len() && !s.blank(last + 1) {
        out.add(
            rule,
            s.lines[if rule == 22 { first } else { last }].clone(),
            "Below",
        );
    }
}
fn fence_start(line: &str) -> Option<(usize, char)> {
    let at = line.find(['`', '~'])?;
    let marker = line.as_bytes()[at];
    (line.as_bytes()[at..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count()
        >= 3)
        .then_some((at, marker as char))
}
fn heading_text<'a>(s: &'a Syntax<'_>, range: &Range<usize>) -> (&'a str, Range<usize>, u8) {
    let first = s.line_at(range.start);
    let last = s.last_line(range);
    let raw = &s.source[range.start..s.lines[first].end];
    let hashes = raw.bytes().take_while(|byte| *byte == b'#').count();
    let (mut from, mut to, mut style) = if hashes > 0 {
        (range.start + hashes, s.lines[first].end, 0)
    } else {
        (
            range.start,
            s.lines[last.saturating_sub(1).max(first)].end,
            2,
        )
    };
    if hashes > 0 {
        let trimmed = s.source[from..to].trim_end();
        let closing = trimmed
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'#')
            .count();
        if closing > 0
            && (trimmed.len() == closing
                || trimmed.as_bytes()[trimmed.len() - closing - 1].is_ascii_whitespace())
        {
            to = from + trimmed.len() - closing;
            style = 1;
        }
    }
    from += s.source[from..to].len() - s.source[from..to].trim_start().len();
    to = from + s.source[from..to].trim_end().len();
    (&s.source[from..to], from..to, style)
}
fn entity_or_emoji_suffix(text: &str) -> bool {
    static ENTITY: OnceLock<Regex> = OnceLock::new();
    ENTITY
        .get_or_init(|| {
            Regex::new(
                r"(?:&(?:#[0-9]+|#[xX][0-9a-fA-F]+|[a-zA-Z][a-zA-Z0-9]*);|:[a-zA-Z0-9_+-]+:)$",
            )
            .unwrap()
        })
        .is_match(text)
}
fn malformed_heading(text: &str, base: usize, out: &mut Sink<'_>) {
    if !text.starts_with('#') || text.starts_with("#️⃣") {
        return;
    }
    let count = text.bytes().take_while(|byte| *byte == b'#').count();
    if count == text.len() {
        return;
    }
    let trimmed = text.trim_end();
    let closed = trimmed.ends_with('#') && !trimmed.ends_with("\\#");
    let missing_left = !text.as_bytes()[count].is_ascii_whitespace();
    if !closed && missing_left {
        let length = text[count..].chars().next().map_or(0, char::len_utf8);
        out.add(18, base..base + count + length, "");
    } else if closed {
        let right = trimmed
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'#')
            .count();
        let body_end = trimmed.len() - right;
        if body_end <= count {
            return;
        }
        let body = &text[count..body_end];
        if body.trim().is_empty() {
            return;
        }
        let missing_right = !text.as_bytes()[body_end - 1].is_ascii_whitespace();
        if missing_left || missing_right {
            out.add(20, base..base + trimmed.len(), "");
        }
    }
}
fn heading_spaces(s: &Syntax<'_>, range: &Range<usize>, out: &mut Sink<'_>) {
    let line = s.line_at(range.start);
    let text = &s.source[range.start..s.lines[line].end];
    let count = text.bytes().take_while(|byte| *byte == b'#').count();
    if count == 0 || count == text.len() {
        return;
    }
    let rest = &text[count..];
    let left = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let trimmed = text.trim_end();
    let right_hashes = trimmed
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'#')
        .count();
    let closed = right_hashes > 0
        && right_hashes < trimmed.len() - count
        && trimmed.as_bytes()[trimmed.len() - right_hashes - 1].is_ascii_whitespace();
    if closed {
        let content = &trimmed[count..trimmed.len() - right_hashes];
        let right = content.len() - content.trim_end_matches([' ', '\t']).len();
        if left > 1 {
            out.add(21, range.start + count + 1..range.start + count + left, "");
        }
        if right > 1 {
            let end = range.start + trimmed.len() - right_hashes;
            out.add(21, end - right + 1..end, "");
        }
    } else if left > 1 {
        out.add(19, range.start + count + 1..range.start + count + left, "");
    }
}
fn emphasis_heading(
    s: &Syntax<'_>,
    start: usize,
    end: usize,
    range: &Range<usize>,
    out: &mut Sink<'_>,
) {
    if end <= start + 2 || s.line_at(range.start) != s.last_line(range) {
        return;
    }
    let children = &s.events[start + 1..end];
    if children.len() == 3 && matches!(children[0].0, Event::Start(Tag::Emphasis | Tag::Strong)) {
        if let Event::Text(text) = &children[1].0 {
            if !text.ends_with(|c| {
                s.config
                    .string(36, "punctuation", ".,;:!?。，；：！？")
                    .contains(c)
            }) {
                out.add(36, range.clone(), "");
            }
        }
    }
}
fn link_label(source: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let text = &source[range.clone()];
    let open = if text.starts_with("![") {
        1
    } else if text.starts_with('[') {
        0
    } else {
        return None;
    };
    let mut depth = 1;
    let mut escaped = false;
    for (i, c) in text[open + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(range.start + open + 1..range.start + open + 1 + i);
            }
        }
    }
    None
}
fn code_span(s: &Syntax<'_>, range: &Range<usize>, out: &mut Sink<'_>) {
    let text = &s.source[range.clone()];
    let ticks = text.bytes().take_while(|byte| *byte == b'`').count();
    if ticks == 0 || text.len() <= ticks * 2 {
        return;
    }
    let body = &text[ticks..text.len() - ticks];
    if body.trim().is_empty() {
        return;
    }
    let leading = body.len() - body.trim_start().len();
    let trailing = body.len() - body.trim_end().len();
    let padding = usize::from(leading > 0 && trailing > 0);
    let first_backtick = body.trim_start().starts_with('`');
    let last_backtick = body.trim_end().ends_with('`');
    let start_allowed = padding.max(usize::from(first_backtick));
    let end_allowed = padding.max(usize::from(last_backtick));
    if leading > start_allowed {
        out.add(38, range.start + ticks..range.start + ticks + leading, "");
    }
    if trailing > end_allowed {
        out.add(38, range.end - ticks - trailing..range.end - ticks, "");
    }
}
fn plain_text(s: &Syntax<'_>, range: &Range<usize>, out: &mut Sink<'_>) {
    static REVERSED: OnceLock<Regex> = OnceLock::new();
    let text = &s.source[range.clone()];
    for found in REVERSED
        .get_or_init(|| Regex::new(r"\([^\r\n)]+\)\[[^\r\n\]^][^\r\n\]]*\]").unwrap())
        .find_iter(text)
    {
        if found.start() == 0 || !text[..found.start()].ends_with('\\') {
            out.add(
                11,
                range.start + found.start()..range.start + found.end(),
                "",
            );
        }
    }
}
fn emphasis_style_check(
    rule: u16,
    width: usize,
    s: &Syntax<'_>,
    range: &Range<usize>,
    expected: &mut Option<char>,
    out: &mut Sink<'_>,
) {
    let Some(marker) = s.source[range.clone()].chars().next() else {
        return;
    };
    let wanted = match s.config.string(rule, "style", "consistent") {
        "asterisk" => '*',
        "underscore" => '_',
        _ => *expected.get_or_insert(marker),
    };
    if wanted == marker {
        return;
    }
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if wanted == '_'
        && (s.source[..range.start]
            .chars()
            .next_back()
            .is_some_and(word)
            || s.source[range.end..].chars().next().is_some_and(word))
    {
        return;
    }
    out.add(rule, range.start..range.start + width, "");
    out.add(rule, range.end - width..range.end, "");
}
fn html(s: &Syntax<'_>, range: &Range<usize>, out: &mut Sink<'_>) {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static ALT: OnceLock<Regex> = OnceLock::new();
    static HIDDEN: OnceLock<Regex> = OnceLock::new();
    let text = &s.source[range.clone()];
    if text.trim_start().starts_with("<!--") {
        return;
    }
    let tag = TAG.get_or_init(|| {
        Regex::new(r#"(?s)<([A-Za-z][A-Za-z0-9-]*)(?:\s+(?:[^>"']|"[^"]*"|'[^']*')*)?\s*/?>"#)
            .unwrap()
    });
    let alt = ALT.get_or_init(|| Regex::new(r#"(?i)\s+alt\s*="#).unwrap());
    let hidden = HIDDEN.get_or_init(|| {
        Regex::new(r#"(?i)\s+aria-hidden\s*=\s*(?:"true"|'true'|true(?:\s|/?>))"#).unwrap()
    });
    for captures in tag.captures_iter(text) {
        let found = captures.get(0).unwrap();
        let span = range.start + found.start()..range.start + found.end();
        if s.config.in_comment(span.start) {
            continue;
        }
        let allowed = s.config.strings(33, "allowed_elements").unwrap_or_default();
        let in_table = s.tables[s.line_at(span.start)];
        let table_allowed = s
            .config
            .strings(33, "table_allowed_elements")
            .unwrap_or_default();
        if !allowed
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&captures[1]))
            && !(in_table
                && table_allowed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&captures[1])))
        {
            out.add(33, span.clone(), format!("Element: {}", &captures[1]));
        }
        if captures[1].eq_ignore_ascii_case("img")
            && !alt.is_match(found.as_str())
            && !hidden.is_match(found.as_str())
        {
            out.add(45, span, "");
        }
    }
}
fn table(
    s: &Syntax<'_>,
    range: &Range<usize>,
    expected_style: &mut Option<(bool, bool)>,
    out: &mut Sink<'_>,
) {
    let first = s.line_at(range.start);
    let last = s.last_line(range);
    let mut expected_columns = 0;
    for line in first..=last {
        let raw = s.line(line);
        let text = if line == first {
            &s.source[range.start..s.lines[line].end]
        } else {
            let mut text = raw.trim_start();
            while let Some(rest) = text.strip_prefix('>') {
                text = rest.trim_start();
            }
            text
        }
        .trim();
        let pipes: Vec<_> = text
            .bytes()
            .enumerate()
            .filter(|(i, byte)| {
                *byte == b'|'
                    && text.as_bytes()[..*i]
                        .iter()
                        .rev()
                        .take_while(|byte| **byte == b'\\')
                        .count()
                        % 2
                        == 0
            })
            .map(|(i, _)| i)
            .collect();
        let leading = pipes.first() == Some(&0);
        let trailing = pipes.last().is_some_and(|i| *i + 1 == text.len());
        let columns = pipes.len() + 1 - usize::from(leading) - usize::from(trailing);
        let style = *expected_style.get_or_insert((leading, trailing));
        if leading != style.0 {
            out.add(55, s.lines[line].clone(), "Leading pipe");
        }
        if trailing != style.1 {
            out.add(55, s.lines[line].clone(), "Trailing pipe");
        }
        if line == first {
            expected_columns = columns;
        } else if columns != expected_columns {
            out.add(
                56,
                s.lines[line].clone(),
                format!("Expected: {expected_columns}; Actual: {columns}"),
            );
        }
    }
}
