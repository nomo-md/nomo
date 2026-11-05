//! 容器及跨段规则。全部使用当前扫描的固定语法上下文，状态随任务释放。
use super::width::display_width;
use super::{
    scanner::{Syntax, checkpoint},
    *,
};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Instant,
};

struct Item {
    marker: Range<usize>,
    number: u64,
    column: usize,
    width: usize,
}
struct List {
    ordered: bool,
    quote_depth: usize,
    items: Vec<Item>,
}

pub(super) fn check(
    s: &Syntax<'_>,
    ends: &[usize],
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    let mut lists: Vec<List> = Vec::new();
    let mut quote_depth = 0;
    let mut footnotes = 0;
    let mut parent_stack = Vec::new();
    let mut previous_quote: HashMap<Option<usize>, Range<usize>> = HashMap::new();
    let mut link_lines = vec![false; s.lines.len()];
    let mut data_lines = vec![false; s.lines.len()];
    let mut heading_lines = vec![false; s.lines.len()];
    let mut in_link = 0;
    let mut in_code = 0;
    let mut paragraph = 0;
    let mut used_references = HashSet::new();
    let mut fragments: HashMap<String, usize> = HashMap::from([("#top".into(), 0)]);
    let mut fragment_links = Vec::new();
    for (i, (event, range)) in s.events.iter().enumerate() {
        checkpoint(request, started)?;
        let first = s.line_at(range.start);
        let last = s.last_line(range);
        match event {
            Event::Start(tag) => {
                match tag {
                    Tag::List(ordered) => {
                        if lists.is_empty() {
                            super::rules::blank_around(32, s, range, out);
                        }
                        lists.push(List {
                            ordered: ordered.is_some(),
                            quote_depth,
                            items: Vec::new(),
                        });
                    }
                    Tag::Item => {
                        if let Some(item) = list_item(s, range) {
                            if let Some(current) = lists.last() {
                                let prefix = &s.source[s.lines[first].start..item.marker.start];
                                if !current.ordered {
                                    let parents = lists
                                        .iter()
                                        .rev()
                                        .take_while(|list| list.quote_depth == quote_depth)
                                        .collect::<Vec<_>>();
                                    if parents.iter().all(|list| !list.ordered) {
                                        let indent = s.config.number(7, "indent", 2);
                                        let initial =
                                            if s.config.boolean(7, "start_indented", false) {
                                                s.config.number(7, "start_indent", indent)
                                            } else {
                                                0
                                            };
                                        let expected = parents
                                            .len()
                                            .saturating_sub(1)
                                            .saturating_mul(indent)
                                            .saturating_add(initial)
                                            .saturating_add(if footnotes > 0 { 4 } else { 0 });
                                        let quote_adjustment = quote_prefix_end(prefix);
                                        let actual =
                                            prefix[quote_adjustment..].encode_utf16().count();
                                        if actual != expected {
                                            out.add(
                                                7,
                                                item.marker.clone(),
                                                format!("Expected: {expected}; Actual: {actual}"),
                                            );
                                        }
                                    }
                                }
                                let suffix = &s.source[item.marker.end..s.lines[first].end];
                                let whitespace =
                                    suffix.len() - suffix.trim_start_matches([' ', '\t']).len();
                                // 五个及以上空格只有首个属于 listItemPrefix，其余是缩进代码。
                                if whitespace > 1 && whitespace <= 4 && !suffix.trim().is_empty() {
                                    out.add(
                                        30,
                                        item.marker.end..item.marker.end + whitespace,
                                        format!("Expected: 1; Actual: {whitespace}"),
                                    );
                                }
                            }
                            if let Some(list) = lists.last_mut() {
                                list.items.push(item);
                            }
                        }
                    }
                    Tag::BlockQuote(_) => {
                        quote_depth += 1;
                        let parent = parent_stack.last().copied();
                        if let Some(previous) = previous_quote.insert(parent, range.clone()) {
                            let previous_last = s.last_line(&previous);
                            if previous_last + 1 < first
                                && (previous_last + 1..first)
                                    .all(|line| s.line(line).trim().is_empty())
                            {
                                for line in previous_last + 1..first {
                                    out.add(28, s.lines[line].clone(), "");
                                }
                            }
                        }
                    }
                    Tag::FootnoteDefinition(_) => footnotes += 1,
                    Tag::Paragraph => paragraph += 1,
                    Tag::CodeBlock(_) => in_code += 1,
                    Tag::Link {
                        id,
                        link_type,
                        dest_url,
                        ..
                    }
                    | Tag::Image {
                        id,
                        link_type,
                        dest_url,
                        ..
                    } => {
                        in_link += 1;
                        link_lines[first..=last].fill(true);
                        if matches!(
                            link_type,
                            LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
                        ) {
                            used_references.insert(normalize(id));
                        }
                        if matches!(tag, Tag::Link { .. })
                            && *link_type == LinkType::Inline
                            && dest_url.starts_with('#')
                            && dest_url.len() > 1
                        {
                            fragment_links.push((dest_url.to_string(), range.clone()));
                        }
                    }
                    Tag::Heading { .. } => {
                        heading_lines[first..=last].fill(true);
                        collect_heading(s, i, ends[i], &mut fragments);
                    }
                    Tag::Table(_) => table_style(s, range, out, request, started)?,
                    _ => {}
                }
                parent_stack.push(i);
            }
            Event::End(tag) => {
                parent_stack.pop();
                match tag {
                    TagEnd::List(_) => {
                        if let Some(list) = lists.pop() {
                            check_list(list, s.config.string(29, "style", "one_or_ordered"), out);
                        }
                    }
                    TagEnd::BlockQuote(_) => quote_depth -= 1,
                    TagEnd::FootnoteDefinition => footnotes -= 1,
                    TagEnd::Paragraph => paragraph -= 1,
                    TagEnd::CodeBlock => in_code -= 1,
                    TagEnd::Link | TagEnd::Image => in_link -= 1,
                    _ => {}
                }
            }
            Event::Text(text)
                if in_link == 0 && in_code == 0 && paragraph > 0 && !text.trim().is_empty() =>
            {
                data_lines[first..=last].fill(true)
            }
            Event::Html(_) | Event::InlineHtml(_) => collect_html(s, range, &mut fragments),
            _ => {}
        }
    }
    let mut definition_lines = vec![false; s.lines.len()];
    let mut labels = HashMap::new();
    let ignored_definitions = s
        .config
        .strings(53, "ignored_definitions")
        .unwrap_or_else(|| vec!["//"])
        .into_iter()
        .map(normalize)
        .collect::<HashSet<_>>();
    for (label, destination, range) in &s.definitions {
        checkpoint(request, started)?;
        let name = normalize(label);
        labels.insert(name.clone(), range.start);
        definition_lines[s.line_at(range.start)..=s.last_line(range)].fill(true);
        if !ignored_definitions.contains(&name) && !used_references.contains(&name) {
            out.add(53, range.clone(), format!("Unused reference: {label}"));
        }
        if destination.starts_with('#') && destination.len() > 1 {
            fragment_links.push((destination.clone(), range.clone()));
        }
    }
    // pulldown-cmark 只保留首次定义；重复定义从未生成正文事件的行中单独确认。
    let mut occupied = vec![false; s.lines.len()];
    for (event, range) in &s.events {
        if matches!(
            event,
            Event::Text(_) | Event::Code(_) | Event::Html(_) | Event::InlineHtml(_)
        ) {
            occupied[s.line_at(range.start)..=s.last_line(range)].fill(true);
        }
    }
    for line in s.body_line..s.lines.len() {
        checkpoint(request, started)?;
        let text = s.line(line);
        if !occupied[line] && !s.code[line] && !s.html[line] && text.trim_start().starts_with('[') {
            let candidate =
                &s.source[s.lines[line].start..s.lines[(line + 3).min(s.lines.len() - 1)].end];
            let parser = Parser::new_ext(candidate, Options::empty());
            for (label, def) in parser.reference_definitions().iter() {
                if def.span.start >= text.len() {
                    continue;
                }
                let name = normalize(label);
                let absolute = s.lines[line].start + def.span.start;
                if !ignored_definitions.contains(&name)
                    && labels.get(&name).is_some_and(|start| *start != absolute)
                {
                    definition_lines[line] = true;
                    out.add(
                        53,
                        s.lines[line].clone(),
                        format!("Duplicate reference: {label}"),
                    );
                }
            }
        }
        let strict = s.config.boolean(13, "strict", false);
        let stern = s.config.boolean(13, "stern", false);
        let base_length = s.config.number(13, "line_length", 80);
        let base_length = if base_length == 0 { 80 } else { base_length };
        let maximum = if s.code[line] {
            s.config.number(13, "code_block_line_length", base_length)
        } else if heading_lines[line] {
            s.config.number(13, "heading_line_length", base_length)
        } else {
            base_length
        };
        let maximum = if maximum == 0 { base_length } else { maximum };
        let checked_length = if strict || stern {
            text.encode_utf16().count()
        } else {
            text.trim_end_matches(|c: char| !c.is_whitespace())
                .encode_utf16()
                .count()
                + 1
        };
        let not_wrappable = text
            .trim_start_matches(['#', '>', ' ', '\t'])
            .split_whitespace()
            .count()
            <= 1;
        if !definition_lines[line]
            && (!s.code[line] || s.config.boolean(13, "code_blocks", true))
            && (!s.tables[line] || s.config.boolean(13, "tables", true))
            && (!heading_lines[line] || s.config.boolean(13, "headings", true))
            && (strict || (!(stern && not_wrappable) && (!link_lines[line] || data_lines[line])))
            && checked_length > maximum
        {
            let mut units = 0;
            let byte = text
                .char_indices()
                .find_map(|(byte, c)| {
                    let previous = units;
                    units += c.len_utf16();
                    (previous >= maximum).then_some(byte)
                })
                .unwrap_or(text.len());
            out.add(
                13,
                s.lines[line].start + byte..s.lines[line].end,
                format!(
                    "Expected: {maximum}; Actual: {}",
                    text.encode_utf16().count()
                ),
            );
        }
    }
    blockquote_spacing(s, out, request, started)?;
    emphasis_spaces(s, ends, out, request, started)?;
    bare_links(s, ends, out, request, started)?;
    static LINE_FRAGMENT: OnceLock<Regex> = OnceLock::new();
    let line_fragment = LINE_FRAGMENT
        .get_or_init(|| Regex::new(r"^#(?:L\d+(?:C\d+)?-L\d+(?:C\d+)?|L\d+)$").unwrap());
    let folded_fragments = if s.config.boolean(51, "ignore_case", false) {
        Some(
            fragments
                .keys()
                .map(|key| key.to_lowercase())
                .collect::<HashSet<_>>(),
        )
    } else {
        None
    };
    for (fragment, range) in fragment_links {
        checkpoint(request, started)?;
        let exists = folded_fragments.as_ref().map_or_else(
            || fragments.contains_key(&fragment),
            |keys| keys.contains(&fragment.to_lowercase()),
        );
        if !exists && !line_fragment.is_match(&fragment) {
            out.add(51, range, format!("Fragment: {fragment}"));
        }
    }
    Ok(())
}

fn normalize(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .to_uppercase()
}
fn list_item(s: &Syntax<'_>, range: &Range<usize>) -> Option<Item> {
    let first = s.line_at(range.start);
    let raw = &s.source[range.start..s.lines[first].end];
    let text = raw.trim_start_matches([' ', '\t']);
    let start = range.start + raw.len() - text.len();
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let (number, width) = if digits > 0 && matches!(text.as_bytes().get(digits), Some(b'.' | b')'))
    {
        (text[..digits].parse().ok()?, digits + 1)
    } else if matches!(text.as_bytes().first(), Some(b'-' | b'*' | b'+')) {
        (0, 1)
    } else {
        return None;
    };
    Some(Item {
        marker: start..start + width,
        number,
        width,
        column: s.source[s.lines[first].start..start].encode_utf16().count(),
    })
}
fn quote_prefix_end(prefix: &str) -> usize {
    prefix.rfind('>').map_or(0, |index| {
        index
            + 1
            + usize::from(
                prefix
                    .as_bytes()
                    .get(index + 1)
                    .is_some_and(u8::is_ascii_whitespace),
            )
    })
}
fn check_list(list: List, style: &str, out: &mut Sink<'_>) {
    let Some(first) = list.items.first() else {
        return;
    };
    let indent = first.column;
    let expected_end = indent + first.width;
    let mut end_matching = false;
    let inferred_incrementing = list
        .items
        .get(1)
        .is_some_and(|second| second.number != 1 || first.number == 0);
    let incrementing = style == "ordered" || (style == "one_or_ordered" && inferred_incrementing);
    let mut expected_number = if style == "zero" || (incrementing && first.number == 0) {
        0
    } else {
        1
    };
    for item in &list.items {
        if item.column != indent || end_matching {
            if list.ordered && item.column + item.width == expected_end {
                end_matching = true;
            } else {
                out.add(
                    5,
                    item.marker.clone(),
                    format!("Expected: {indent}; Actual: {}", item.column),
                );
            }
        }
        if list.ordered {
            if item.number != expected_number {
                out.add(
                    29,
                    item.marker.clone(),
                    format!("Expected: {expected_number}; Actual: {}", item.number),
                );
            }
            if incrementing {
                expected_number += 1;
            }
        }
    }
}
fn blockquote_spacing(
    s: &Syntax<'_>,
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    let mut quote_lines = vec![false; s.lines.len()];
    let mut required_column = vec![0; s.lines.len()];
    for (event, range) in &s.events {
        checkpoint(request, started)?;
        if matches!(event, Event::Start(Tag::BlockQuote(_))) {
            quote_lines[s.line_at(range.start)..=s.last_line(range)].fill(true);
        }
        if matches!(event, Event::Start(Tag::Item))
            && let Some(item) = list_item(s, range)
        {
            let first = s.line_at(range.start);
            let last = s.last_line(range);
            let suffix = &s.source[item.marker.end..s.lines[first].end];
            let spaces = suffix.len() - suffix.trim_start_matches([' ', '\t']).len();
            let content_column =
                item.column + item.width + if (1..=4).contains(&spaces) { spaces } else { 1 };
            for line in first + 1..=last {
                required_column[line] = required_column[line].max(content_column);
            }
        }
    }
    for line in s.body_line..s.lines.len() {
        checkpoint(request, started)?;
        if !quote_lines[line] || s.code[line] {
            continue;
        }
        let text = s.line(line);
        let mut offset = 0;
        loop {
            let prefix = &text[offset..];
            let whitespace = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
            offset += whitespace;
            if text.as_bytes().get(offset) != Some(&b'>') {
                break;
            }
            offset += 1;
            let suffix = &text[offset..];
            let spaces = suffix.len() - suffix.trim_start_matches([' ', '\t']).len();
            let required = if suffix.trim_start().starts_with('>') {
                1
            } else {
                required_column[line].saturating_sub(offset).max(1)
            };
            if spaces > required && !suffix.trim().is_empty() {
                // 列表内容的必要缩进由 MD007/MD005 检查，额外引用前缀空格在此报告。
                out.add(
                    27,
                    s.lines[line].start + offset + required..s.lines[line].start + offset + spaces,
                    "",
                );
            }
            offset += spaces;
            if offset >= text.len() {
                break;
            }
        }
    }
    Ok(())
}
fn emphasis_spaces(
    s: &Syntax<'_>,
    ends: &[usize],
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    for (i, (event, _)) in s.events.iter().enumerate() {
        checkpoint(request, started)?;
        if !matches!(
            event,
            Event::Start(
                Tag::Paragraph
                    | Tag::Heading { .. }
                    | Tag::TableCell
                    | Tag::Emphasis
                    | Tag::Strong
                    | Tag::Link { .. }
            )
        ) {
            continue;
        }
        let mut markers: [Vec<Range<usize>>; 6] = std::array::from_fn(|_| Vec::new());
        let mut child = i + 1;
        while child < ends[i] {
            checkpoint(request, started)?;
            let (event, range) = &s.events[child];
            if matches!(event, Event::Start(_)) {
                child = ends[child] + 1;
                continue;
            }
            if let Event::Text(_) = event {
                let raw = &s.source[range.clone()];
                let mut at = 0;
                while at < raw.len() {
                    let marker = raw.as_bytes()[at];
                    if b"*_".contains(&marker) && (at == 0 || raw.as_bytes()[at - 1] != b'\\') {
                        let count = raw.as_bytes()[at..]
                            .iter()
                            .take_while(|byte| **byte == marker)
                            .count();
                        let before = s.source[..range.start + at].chars().next_back();
                        let after = raw[at + count..].chars().next();
                        let intraword = marker == b'_'
                            && before.is_some_and(char::is_alphanumeric)
                            && after.is_some_and(char::is_alphanumeric);
                        if count <= 3 && !intraword {
                            markers[(if marker == b'_' { 0 } else { 3 }) + count - 1]
                                .push(range.start + at..range.start + at + count);
                        }
                        at += count;
                    } else {
                        at += raw[at..].chars().next().unwrap().len_utf8();
                    }
                }
            }
            child += 1;
        }
        for group in markers {
            for pair in group.chunks_exact(2) {
                let start_line = s.line_at(pair[0].end);
                let suffix = &s.source[pair[0].end..s.lines[start_line].end];
                let leading = suffix.len() - suffix.trim_start().len();
                if leading > 0 && !suffix.trim().is_empty() {
                    out.add(37, pair[0].end..pair[0].end + leading, "");
                }
                let end_line = s.line_at(pair[1].start);
                let prefix = &s.source[s.lines[end_line].start..pair[1].start];
                let trailing = prefix.len() - prefix.trim_end().len();
                if trailing > 0 && !prefix.trim().is_empty() {
                    out.add(37, pair[1].start - trailing..pair[1].start, "");
                }
            }
        }
    }
    Ok(())
}

fn collect_heading(
    s: &Syntax<'_>,
    start: usize,
    end: usize,
    fragments: &mut HashMap<String, usize>,
) {
    static SLUG: OnceLock<Regex> = OnceLock::new();
    static EXPLICIT: OnceLock<Regex> = OnceLock::new();
    let mut text = String::new();
    let mut image = 0;
    for (event, _) in &s.events[start + 1..end] {
        match event {
            Event::Start(Tag::Image { .. }) => image += 1,
            Event::End(TagEnd::Image) => image -= 1,
            Event::Text(value) | Event::Code(value) | Event::InlineMath(value) if image == 0 => {
                text.push_str(value)
            }
            Event::SoftBreak | Event::HardBreak if image == 0 => text.push(' '),
            _ => {}
        }
    }
    let lower = text.to_lowercase();
    let slug = SLUG
        .get_or_init(|| Regex::new(r"[^\p{L}\p{M}\p{N}\p{Pc}\- ]").unwrap())
        .replace_all(&lower, "")
        .replace(' ', "-");
    if slug.is_empty() {
        return;
    }
    let fragment = format!("#{slug}");
    let count = fragments.get(&fragment).copied().unwrap_or(0);
    if count > 0 {
        fragments.insert(format!("{fragment}-{count}"), 0);
    }
    fragments.insert(fragment, count + 1);
    for capture in EXPLICIT
        .get_or_init(|| Regex::new(r"\{(#[a-z\d]+(?:[-_][a-z\d]+)*)\}").unwrap())
        .captures_iter(&s.source[s.events[start].1.clone()])
    {
        fragments.entry(capture[1].to_owned()).or_insert(1);
    }
}
fn bare_links(
    s: &Syntax<'_>,
    ends: &[usize],
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    static URL: OnceLock<Regex> = OnceLock::new();
    static ENTITY: OnceLock<Regex> = OnceLock::new();
    let urls = URL.get_or_init(|| Regex::new(r"(?i)(?:https?://|www\.)[^\s<>]+|[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9_-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9_-]*[a-z0-9])?)+").unwrap());
    let entity = ENTITY
        .get_or_init(|| Regex::new(r"&(?:#[xX]?[0-9a-fA-F]+|[a-zA-Z][a-zA-Z0-9]*);$").unwrap());
    let mut excluded = Vec::<Range<usize>>::new();
    let mut html_tags: Vec<(String, usize)> = Vec::new();
    for (event, range) in &s.events {
        checkpoint(request, started)?;
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. })
            | Event::Code(_)
            | Event::InlineMath(_) => excluded.push(range.clone()),
            Event::InlineHtml(html) => {
                excluded.push(range.clone());
                let raw = html.trim_start().trim_start_matches('<');
                let closing = raw.starts_with('/');
                let name = raw
                    .trim_start_matches('/')
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name.is_empty() {
                    continue;
                }
                if closing {
                    if let Some(at) = html_tags.iter().rposition(|(tag, _)| *tag == name) {
                        excluded.push(html_tags[at].1..range.end);
                        html_tags.truncate(at);
                    }
                } else if !html.trim_end().ends_with("/>")
                    && !matches!(
                        name.as_str(),
                        "area"
                            | "base"
                            | "br"
                            | "col"
                            | "embed"
                            | "hr"
                            | "img"
                            | "input"
                            | "link"
                            | "meta"
                            | "param"
                            | "source"
                            | "track"
                            | "wbr"
                    )
                {
                    html_tags.push((name, range.start));
                }
            }
            _ => {}
        }
    }
    excluded.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in excluded {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    for (i, (event, range)) in s.events.iter().enumerate() {
        checkpoint(request, started)?;
        if !matches!(
            event,
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell)
        ) || ends[i] == 0
        {
            continue;
        }
        let raw = &s.source[range.clone()];
        for found in urls.find_iter(raw) {
            checkpoint(request, started)?;
            let start = range.start + found.start();
            let previous = s.source[..start].chars().next_back();
            if previous.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '<') {
                continue;
            }
            let at = merged.partition_point(|range| range.start <= start);
            if at > 0 && start < merged[at - 1].end {
                continue;
            }
            let mut text = found
                .as_str()
                .trim_end_matches(['?', '!', '.', ',', ':', '*', '_', '~']);
            let mut extra = text
                .matches(')')
                .count()
                .saturating_sub(text.matches('(').count());
            while extra > 0 && text.ends_with(')') {
                text = &text[..text.len() - 1];
                extra -= 1;
            }
            if let Some(found) = entity.find(text) {
                text = &text[..found.start()];
            }
            if text.is_empty() {
                continue;
            }
            if text.to_ascii_lowercase().starts_with("www.") {
                let domain = text.split('/').next().unwrap_or(text);
                if domain.matches('.').count() < 2 || domain.ends_with('.') {
                    continue;
                }
            }
            out.add(34, start..start + text.len(), "");
        }
    }
    Ok(())
}
fn collect_html(s: &Syntax<'_>, range: &Range<usize>, fragments: &mut HashMap<String, usize>) {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    let source = &s.source[range.clone()];
    if source.trim_start().starts_with("<!--") {
        return;
    }
    let tags = TAG.get_or_init(|| {
        Regex::new(r#"(?s)<([a-zA-Z][a-zA-Z0-9-]*)(?:\s+(?:[^>"']|"[^"]*"|'[^']*')*)?\s*/?>"#)
            .unwrap()
    });
    let attr = ATTRIBUTE.get_or_init(|| {
        Regex::new(r#"(?i)\s+(id|name)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap()
    });
    for tag in tags.captures_iter(source) {
        if s.config
            .in_comment(range.start + tag.get(0).unwrap().start())
        {
            continue;
        }
        let mut named = None;
        for capture in attr.captures_iter(&tag[0]) {
            let value = capture
                .get(2)
                .or_else(|| capture.get(3))
                .or_else(|| capture.get(4))
                .unwrap()
                .as_str();
            if capture[1].eq_ignore_ascii_case("id") {
                fragments.insert(format!("#{value}"), 0);
                named = None;
                break;
            }
            if tag[1].eq_ignore_ascii_case("a") {
                named = Some(value.to_owned());
            }
        }
        if let Some(name) = named {
            fragments.insert(format!("#{name}"), 0);
        }
    }
}

fn table_style(
    s: &Syntax<'_>,
    range: &Range<usize>,
    out: &mut Sink<'_>,
    request: &Request,
    started: Instant,
) -> TextDocumentResult<()> {
    let first = s.line_at(range.start);
    let last = s.last_line(range);
    let mut header = HashSet::new();
    let mut aligned = Vec::new();
    let mut compact = Vec::new();
    let mut tight = Vec::new();
    for line in first..=last {
        checkpoint(request, started)?;
        let text = s.line(line);
        let pipes = text
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
            .collect::<Vec<_>>();
        let mut remaining = header.clone();
        let mut previous = 0;
        let mut effective = 0;
        for (position, pipe) in pipes.iter().copied().enumerate() {
            checkpoint(request, started)?;
            effective += display_width(&text[previous..pipe]);
            previous = pipe;
            let span = s.lines[line].start + pipe..s.lines[line].start + pipe + 1;
            if line == first {
                header.insert(effective);
            } else if !remaining.is_empty() {
                if !remaining.remove(&effective) {
                    aligned.push(span.clone());
                }
            }
            let left = &text[pipes.get(position.wrapping_sub(1)).map_or(0, |p| p + 1)..pipe];
            let right = &text[pipe + 1..pipes.get(position + 1).copied().unwrap_or(text.len())];
            if !left.trim().is_empty() {
                let spaces = left.len() - left.trim_end_matches([' ', '\t']).len();
                if spaces != 1 {
                    compact.push(span.clone());
                }
                if spaces > 0 {
                    tight.push(span.clone());
                }
            }
            if !right.trim().is_empty() {
                let spaces = right.len() - right.trim_start_matches([' ', '\t']).len();
                if spaces != 1 {
                    compact.push(span.clone());
                }
                if spaces > 0 {
                    tight.push(span);
                }
            }
        }
    }
    let configured = s.config.string(60, "style", "any");
    let (mut errors, mut style) = (aligned, "aligned");
    if configured == "compact" || (configured == "any" && compact.len() < errors.len()) {
        errors = compact;
        style = "compact";
    }
    if configured == "tight" || (configured == "any" && tight.len() < errors.len()) {
        errors = tight;
        style = "tight";
    }
    for span in errors {
        checkpoint(request, started)?;
        out.add(60, span, format!("Style: {style}"));
    }
    Ok(())
}
