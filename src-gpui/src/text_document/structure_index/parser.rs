//! Rust 权威结构解析：内核只处理有界片段，Nomo 容器和标题语义保留原文坐标。
#[path = "syntax.rs"]
mod syntax;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SYNTAX_VERSION: &str = "nomo-markdown-v1";
pub const MAX_SEMANTIC_BYTES: usize = 32 * 1024;
static NEXT_BLOCK: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Enhancements {
    pub inline_code: usize,
    pub inline_math: usize,
    pub images: usize,
    pub code_blocks: usize,
    pub diagrams: usize,
    pub block_math: usize,
    pub html_details: usize,
}
impl Enhancements {
    pub fn total(&self) -> usize {
        self.inline_code
            + self.inline_math
            + self.images
            + self.code_blocks
            + self.diagrams
            + self.block_math
            + self.html_details
    }
    pub fn add(&mut self, other: &Self) {
        self.inline_code += other.inline_code;
        self.inline_math += other.inline_math;
        self.images += other.images;
        self.code_blocks += other.code_blocks;
        self.diagrams += other.diagrams;
        self.block_math += other.block_math;
        self.html_details += other.html_details;
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub href: String,
    pub title: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub title: String,
    pub byte: u64,
    pub line: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Footnote {
    pub label: String,
    pub from_byte: u64,
    pub to_byte: u64,
    pub definition: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: String,
    pub version: u64,
    pub content_version: u64,
    pub from_byte: u64,
    pub to_byte: u64,
    pub from_line: u64,
    pub to_line: u64,
    pub kind: String,
    pub mode: String,
    pub layout_units: usize,
    pub enhancement_objects: usize,
    pub enhancements: Enhancements,
    pub metrics_complete: bool,
    pub semantic_complete: bool,
    pub headings: Vec<Heading>,
    pub references: Vec<String>,
    pub definitions: BTreeMap<String, Reference>,
    pub source_hash: Option<u32>,
    #[serde(default)]
    pub footnotes: Vec<Footnote>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub blocks: Vec<Block>,
    pub references: BTreeMap<String, Reference>,
    pub consumed_bytes: u64,
    pub complete: bool,
}

pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_OLD_FOOTNOTES
}
pub fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .to_uppercase()
}
pub fn new_block_id() -> String {
    format!(
        "native-block-{}",
        NEXT_BLOCK.fetch_add(1, Ordering::Relaxed)
    )
}
pub fn line_count(source: &str) -> u64 {
    let mut previous = 0;
    source
        .bytes()
        .filter(|ch| {
            let newline = *ch == b'\r' || (*ch == b'\n' && previous != b'\r');
            previous = *ch;
            newline
        })
        .count() as u64
}
fn line_start(source: &str, at: usize) -> usize {
    source[..at].rfind(['\r', '\n']).map(|i| i + 1).unwrap_or(0)
}
fn source_hash(source: &str) -> u32 {
    source.encode_utf16().fold(2166136261, |hash, ch| {
        (hash ^ ch as u32).wrapping_mul(16777619)
    })
}

#[derive(Clone, Debug)]
struct Span {
    from: usize,
    to: usize,
    kind: String,
    body: Option<Range<usize>>,
}
fn kind(tag: &Tag<'_>) -> &'static str {
    match tag {
        Tag::Paragraph => "paragraph_open",
        Tag::Heading { .. } => "heading_open",
        Tag::BlockQuote(_) => "blockquote_open",
        Tag::List(None) => "bullet_list_open",
        Tag::List(Some(_)) => "ordered_list_open",
        Tag::CodeBlock(CodeBlockKind::Fenced(_)) => "fence",
        Tag::CodeBlock(_) => "code_block",
        Tag::HtmlBlock => "html_block",
        Tag::Table(_) => "table_open",
        Tag::FootnoteDefinition(_) => "footnote_def_open",
        _ => "fragment",
    }
}

/// 把内核顶层块合并为 Nomo 容器。只在内核确认不是代码的位置识别扩展。
fn spans(source: &str, document_start: bool) -> Vec<Span> {
    if document_start {
        if let Some(end) = syntax::front_matter_end(source) {
            let mut result = vec![Span {
                from: 0,
                to: end,
                kind: "front_matter".into(),
                body: None,
            }];
            result.extend(spans(&source[end..], false).into_iter().map(|mut span| {
                span.from += end;
                span.to += end;
                if let Some(body) = &mut span.body {
                    body.start += end;
                    body.end += end;
                }
                span
            }));
            return result;
        }
    }
    let mut depth = 0usize;
    let mut roots = vec![];
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    roots.push((line_start(source, range.start), kind(&tag).to_string()));
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Rule if depth == 0 => roots.push((range.start, "hr".into())),
            _ => {}
        }
    }
    if roots.is_empty() && !source.is_empty() {
        roots.push((0, "fragment".into()));
    }
    let mut result: Vec<Span> = vec![];
    let mut skip_until = 0;
    for (index, (from, original_kind)) in roots.iter().enumerate() {
        if *from < skip_until {
            continue;
        }
        let tail = &source[*from..];
        let indent = tail.len() - tail.trim_start_matches(' ').len();
        let at = from + indent;
        let mut end = roots
            .get(index + 1)
            .map(|(from, _)| *from)
            .unwrap_or(source.len());
        let mut span = Span {
            from: *from,
            to: end,
            kind: original_kind.clone(),
            body: None,
        };
        if indent <= 3 && original_kind == "html_block" {
            if let Some(details) = syntax::details(source, at) {
                let line_end = syntax::line_end(source, details.end);
                if source[details.end..line_end].trim().is_empty() {
                    end = roots
                        .iter()
                        .find(|(start, _)| *start >= line_end)
                        .map(|(start, _)| *start)
                        .unwrap_or(source.len());
                    span.kind = "details_open".into();
                    span.to = end;
                    span.body = Some(details.body_from..details.body_to);
                    skip_until = end;
                }
            } else if source[at..syntax::line_end(source, at)].trim() == "<!-- toc -->" {
                let mut cursor = syntax::line_end(source, at);
                let mut fence = None;
                while cursor < source.len() {
                    let next = syntax::line_end(source, cursor);
                    let line = source[cursor..next].trim();
                    if let Some((marker, length)) = fence {
                        let run = line.bytes().take_while(|ch| *ch == marker).count();
                        if run >= length && line[run..].trim().is_empty() {
                            fence = None;
                        }
                    } else if line.starts_with("```") || line.starts_with("~~~") {
                        let marker = line.as_bytes()[0];
                        fence = Some((marker, line.bytes().take_while(|ch| *ch == marker).count()));
                    } else if line == "<!-- /toc -->" {
                        end = roots
                            .iter()
                            .find(|(start, _)| *start >= next)
                            .map(|(start, _)| *start)
                            .unwrap_or(source.len());
                        span.kind = "toc_block".into();
                        span.to = end;
                        skip_until = end;
                        break;
                    }
                    cursor = next;
                }
            }
        }
        if original_kind == "blockquote_open" {
            let line = tail.lines().next().unwrap_or("").trim_start();
            let marker = line.strip_prefix('>').unwrap_or("").trim_start();
            if [
                "[!NOTE]",
                "[!TIP]",
                "[!IMPORTANT]",
                "[!WARNING]",
                "[!CAUTION]",
            ]
            .iter()
            .any(|prefix| marker.starts_with(prefix))
            {
                span.kind = "callout_open".into();
            }
        }
        if original_kind == "paragraph_open" && tail.trim_start().starts_with("$$") {
            let events: Vec<_> = Parser::new_ext(&source[*from..end], options()).collect();
            if events
                .iter()
                .any(|event| matches!(event, Event::DisplayMath(_)))
                && !events
                    .iter()
                    .any(|event| matches!(event, Event::Text(text) if !text.trim().is_empty()))
            {
                span.kind = "math_display".into();
            }
        }
        result.push(span);
    }
    if let Some(first) = result.first_mut() {
        first.from = 0;
    }
    result
}

fn prepare_scripts(source: &str) -> String {
    // 代码内部的定界符没有格式语义；在原文上处理脚本，防止解码后的转义符再次生效。
    let protected: Vec<_> = Parser::new_ext(source, options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) => Some(range),
            Event::InlineHtml(_) => {
                syntax::html_code(source, range.start).map(|(end, _)| range.start..end)
            }
            _ => None,
        })
        .collect();
    let mut result = String::new();
    let mut at = 0;
    let bytes = source.as_bytes();
    while at < source.len() {
        if let Some(range) = protected.iter().find(|range| range.start == at) {
            result.push_str(&source[range.clone()]);
            at = range.end;
            continue;
        }
        if bytes[at] == b'\\' && at + 1 < source.len() {
            let end = at + 1 + source[at + 1..].chars().next().unwrap().len_utf8();
            result.push_str(&source[at..end]);
            at = end;
            continue;
        }
        let marker = bytes[at];
        if matches!(marker, b'^' | b'~')
            && bytes.get(at + 1) != Some(&marker)
            && (at == 0 || bytes[at - 1] != marker)
        {
            let mut end = at + 1;
            while end < source.len() {
                if let Some(range) = protected.iter().find(|range| range.start == end) {
                    end = range.end;
                    continue;
                }
                let ch = source[end..].chars().next().unwrap();
                if ch == '\\' && end + 1 < source.len() && !matches!(bytes[end + 1], b'\r' | b'\n')
                {
                    end += 1 + source[end + 1..].chars().next().unwrap().len_utf8();
                    continue;
                }
                if ch.is_whitespace() || bytes[end] == marker {
                    break;
                }
                end += ch.len_utf8();
            }
            if end > at + 1
                && bytes.get(end) == Some(&marker)
                && bytes.get(end + 1) != Some(&marker)
            {
                result.push_str(
                    &source[at + 1..end]
                        .replace("\\ ", " ")
                        .replace("\\\t", "\t"),
                );
                at = end + 1;
                continue;
            }
        }
        let ch = source[at..].chars().next().unwrap();
        result.push(ch);
        at += ch.len_utf8();
    }
    result
}

/// 标题文本独立于 slug。公式保留现有标题展示中的 $，code 的内容禁止二次解释。
pub fn heading_title(source: &str) -> String {
    let prepared = prepare_scripts(source);
    let source = prepared.as_str();
    let mut text = String::new();
    let mut skip = 0usize;
    let mut image_end = None;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        if range.start < skip {
            continue;
        }
        match event {
            Event::Text(value) => {
                let mut value = value.to_string();
                if let Some(end) = image_end.take() {
                    if source[end..].starts_with('{') {
                        if let Some(close) = source[end..].find('}') {
                            if !source[end + 1..end + close].trim().is_empty() {
                                skip = end + close + 1;
                                if range.end <= skip {
                                    continue;
                                }
                                value = source[skip..range.end].to_string();
                            }
                        }
                    }
                }
                text.push_str(&value);
            }
            Event::Code(value) | Event::InlineMath(value) => {
                let value = if matches!(source.as_bytes().get(range.start), Some(b'$')) {
                    source[range.clone()].to_string()
                } else {
                    value.to_string()
                };
                text.push_str(&value);
            }
            Event::End(TagEnd::Image) => {
                image_end = Some(range.end);
            }
            Event::InlineHtml(_) => {
                if let Some((end, literal)) = syntax::html_code(source, range.start) {
                    skip = end;
                    text.push_str(&literal);
                } else if let Some(tag) = syntax::html_tag(source, range.start) {
                    if tag.name == "br" {
                        text.push(' ');
                    }
                    if tag.name == "img" {
                        if let Some(alt) = tag.attrs.get("alt") {
                            text.push_str(alt);
                        }
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            Event::FootnoteReference(label) => {
                text.push_str("[^");
                text.push_str(&label);
                text.push(']');
            }
            _ => {}
        }
    }
    text.replace(['\r', '\n'], " ").trim().to_owned()
}

fn raw_heading(source: &str, range: Range<usize>, level: u8) -> String {
    let mut lines = source[range]
        .trim_end_matches(['\r', '\n'])
        .lines()
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let first = lines[0].trim_start();
    if first.starts_with(&"#".repeat(level as usize)) {
        let title = first[level as usize..].trim();
        let trimmed = title.trim_end_matches('#');
        if trimmed.len() < title.len() && (trimmed.is_empty() || trimmed.ends_with([' ', '\t'])) {
            return trimmed.trim().into();
        }
        return title.into();
    }
    if lines.len() > 1 {
        lines.pop();
    }
    lines.join("\n").trim().into()
}

fn metadata(source: &str, span: &Span, revision: u64, depth: usize) -> Block {
    let part = &source[span.from..span.to];
    let mut block = Block {
        id: new_block_id(),
        version: revision,
        content_version: revision,
        from_byte: span.from as u64,
        to_byte: span.to as u64,
        from_line: line_count(&source[..span.from]) + 1,
        to_line: line_count(&source[..span.to]) + 1,
        kind: span.kind.clone(),
        mode: "semantic".into(),
        layout_units: 0,
        enhancement_objects: 0,
        enhancements: Enhancements::default(),
        metrics_complete: true,
        semantic_complete: true,
        headings: vec![],
        references: vec![],
        definitions: BTreeMap::new(),
        source_hash: Some(source_hash(part)),
        footnotes: vec![],
    };
    if span.kind == "front_matter" || span.kind == "toc_block" {
        return block;
    }
    if let Some(body) = &span.body {
        block.enhancements.html_details = 1;
        if depth < 100 {
            for child in parse_inner(&source[body.clone()], revision, false, depth + 1) {
                block.layout_units += child.layout_units;
                block.enhancements.add(&child.enhancements);
                block.metrics_complete &= child.metrics_complete;
                block.semantic_complete &= child.semantic_complete;
                block
                    .headings
                    .extend(child.headings.into_iter().map(|mut h| {
                        h.byte += body.start as u64;
                        h.line += line_count(&source[..body.start]);
                        h
                    }));
                block
                    .footnotes
                    .extend(child.footnotes.into_iter().map(|mut note| {
                        note.from_byte += body.start as u64;
                        note.to_byte += body.start as u64;
                        note
                    }));
                block.references.extend(child.references);
                for (key, value) in child.definitions {
                    block.definitions.entry(key).or_insert(value);
                }
            }
        } else {
            block.metrics_complete = false;
            block.semantic_complete = false;
        }
        block.enhancement_objects = block.enhancements.total();
        return block;
    }
    let parser = Parser::new_ext(part, options());
    for (label, definition) in parser.reference_definitions().iter() {
        block.definitions.insert(
            normalize_label(label),
            Reference {
                href: definition.dest.to_string(),
                title: definition.title.as_deref().unwrap_or("").into(),
            },
        );
    }
    let mut text_ranges = vec![];
    let mut skip_until = 0;
    for (event, range) in parser.into_offset_iter() {
        if range.start < skip_until {
            continue;
        }
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let byte = line_start(part, range.start);
                let text = raw_heading(part, range, level as u8);
                let title = heading_title(&text);
                block.headings.push(Heading {
                    level: level as u8,
                    text,
                    title,
                    byte: (span.from + byte) as u64,
                    line: block.from_line + line_count(&part[..byte]),
                });
                block.layout_units += 1;
            }
            Event::Start(Tag::Paragraph | Tag::TableCell) => block.layout_units += 1,
            Event::Start(Tag::CodeBlock(code)) => {
                block.layout_units += 1;
                if matches!(code, CodeBlockKind::Fenced(ref language) if language.split_whitespace().next().unwrap_or("").eq_ignore_ascii_case("mermaid"))
                {
                    block.enhancements.diagrams += 1;
                } else {
                    block.enhancements.code_blocks += 1;
                }
            }
            Event::Start(Tag::Image { link_type, id, .. }) => {
                block.enhancements.images += 1;
                if !matches!(
                    link_type,
                    pulldown_cmark::LinkType::Inline
                        | pulldown_cmark::LinkType::Autolink
                        | pulldown_cmark::LinkType::Email
                ) {
                    block.references.push(normalize_label(&id));
                }
            }
            Event::Start(Tag::Link { link_type, id, .. }) => {
                if !matches!(
                    link_type,
                    pulldown_cmark::LinkType::Inline
                        | pulldown_cmark::LinkType::Autolink
                        | pulldown_cmark::LinkType::Email
                ) {
                    block.references.push(normalize_label(&id));
                }
            }
            Event::Start(Tag::FootnoteDefinition(label)) => block.footnotes.push(Footnote {
                label: label.to_string(),
                from_byte: (span.from + range.start) as u64,
                to_byte: (span.from + range.end) as u64,
                definition: true,
            }),
            Event::FootnoteReference(label) => block.footnotes.push(Footnote {
                label: label.to_string(),
                from_byte: (span.from + range.start) as u64,
                to_byte: (span.from + range.end) as u64,
                definition: false,
            }),
            Event::Code(_) => block.enhancements.inline_code += 1,
            Event::InlineMath(_) => block.enhancements.inline_math += 1,
            Event::DisplayMath(_) => {
                block.enhancements.block_math += 1;
                if span.kind != "math_display" {
                    block.layout_units += 1;
                }
            }
            Event::Start(Tag::HtmlBlock) => {
                block.enhancements.html_details += 1;
                block.metrics_complete = false;
            }
            Event::InlineHtml(_) => {
                if let Some((end, _)) = syntax::html_code(part, range.start) {
                    skip_until = end;
                    block.enhancements.inline_code += 1;
                } else if syntax::html_tag(part, range.start).is_some_and(|tag| tag.name == "img") {
                    block.enhancements.images += 1;
                }
            }
            Event::Text(_) => {
                if let Some(previous) = text_ranges
                    .last_mut()
                    .filter(|previous: &&mut Range<usize>| previous.end == range.start)
                {
                    previous.end = range.end;
                } else {
                    text_ranges.push(range);
                }
            }
            _ => {}
        }
    }
    // 未解析成功的候选也要登记，才能在后方定义新增时使当前块失效。
    for range in text_ranges {
        let text = &part[range.clone()];
        let mut at = 0;
        while let Some(open) = text[at..].find('[') {
            let open = at + open;
            let Some(close) = text[open + 1..].find(']') else {
                break;
            };
            let close = open + 1 + close;
            let label = &text[open + 1..close];
            if let Some(label) = label
                .strip_prefix('^')
                .filter(|label| !label.is_empty() && !label.chars().any(char::is_whitespace))
            {
                block.footnotes.push(Footnote {
                    label: label.to_string(),
                    from_byte: (span.from + range.start + open) as u64,
                    to_byte: (span.from + range.start + close + 1) as u64,
                    definition: false,
                });
            } else if !label.is_empty() {
                block.references.push(normalize_label(label));
            }
            at = close + 1;
        }
    }
    block.references = block
        .references
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    block.enhancement_objects = block.enhancements.total();
    block
}

fn parse_inner(source: &str, revision: u64, document_start: bool, depth: usize) -> Vec<Block> {
    spans(source, document_start)
        .iter()
        .map(|span| metadata(source, span, revision, depth))
        .collect()
}
pub fn parse_fragment(
    source: &str,
    revision: u64,
    document_start: bool,
) -> Result<Vec<Block>, String> {
    if source.len() > MAX_SEMANTIC_BYTES {
        return Err("结构语义片段超过 32 KiB".into());
    }
    Ok(parse_inner(source, revision, document_start, 0))
}

/// 尚未完整的 Nomo 容器禁止在最后一个普通 CommonMark 块之前提前提交。
pub fn stable_cutoff(source: &str, document_start: bool) -> usize {
    if document_start
        && (source.starts_with("---\n") || source.starts_with("---\r\n"))
        && syntax::front_matter_end(source).is_none()
    {
        return 0;
    }
    let spans = spans(source, document_start);
    let mut cut = spans.last().map(|span| span.from).unwrap_or(0);
    for span in &spans {
        let tail = source[span.from..].trim_start_matches([' ', '\t']);
        let start = source.len() - tail.len();
        if span.kind == "html_block"
            && ((tail.to_ascii_lowercase().starts_with("<details")
                && syntax::details(source, start).is_none())
                || tail
                    .lines()
                    .next()
                    .is_some_and(|line| line.trim() == "<!-- toc -->"))
        {
            cut = cut.min(span.from);
        }
    }
    cut
}

/// 超长无序列表只在顶层兄弟项之间分窗，嵌套列表与任务标记随所属项一起解析。
/// 有序列表仍保留整体边界，避免分窗后重新解释重复的起始编号。
pub fn bullet_list_cutoff(source: &str) -> usize {
    if !spans(source, false).first().is_some_and(|span| span.from == 0 && span.kind == "bullet_list_open") {
        return 0;
    }
    let mut depth = 0usize;
    let mut cut = 0;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 1 && tag == Tag::Item {
                    cut = line_start(source, range.start);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 { break; }
            }
            _ => {}
        }
    }
    cut
}

#[derive(Default)]
pub struct HeadingIds {
    used: BTreeSet<String>,
    next: BTreeMap<String, usize>,
}
impl HeadingIds {
    pub fn allocate(&mut self, title: &str, ordinal: usize) -> String {
        let filtered: String = title
            .to_lowercase()
            .chars()
            .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '-')
            .collect();
        let slug = filtered.split_whitespace().collect::<Vec<_>>().join("-");
        let base = if slug.is_empty() {
            format!("heading-{ordinal}")
        } else {
            slug
        };
        let mut id = base.clone();
        let suffix = self.next.entry(base.clone()).or_insert(2);
        while self.used.contains(&id) {
            id = format!("{base}-{}", *suffix);
            *suffix += 1;
        }
        self.used.insert(id.clone());
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_references_and_footnotes_are_dependencies_without_code_false_targets() {
        let blocks = parse_fragment(
            "[shown][late] ![image][pic] [^note] `[ignored] [^code]`\n",
            1,
            true,
        )
        .unwrap();
        assert!(blocks[0].references.contains(&"LATE".to_owned()));
        assert!(blocks[0].references.contains(&"PIC".to_owned()));
        assert!(!blocks[0].references.contains(&"IGNORED".to_owned()));
        assert_eq!(blocks[0].footnotes.len(), 1);
        assert_eq!(blocks[0].footnotes[0].label, "note");
    }
    #[test]
    fn exhausted_container_depth_is_not_reported_as_complete_semantics() {
        let source = format!(
            "{}# Hidden\n\n{}",
            "<details>\n<summary>Nested</summary>\n\n".repeat(102),
            "</details>\n\n".repeat(102)
        );
        let blocks = parse_fragment(&source, 0, true).unwrap();
        assert!(blocks.iter().any(|block| !block.semantic_complete));
    }
    #[test]
    fn title_literal_code_escapes_and_private_unicode_are_preserved() {
        assert_eq!(heading_title("\\^a\\^ x^a\\ b^"), "^a^ xa b");
        assert_eq!(
            heading_title("x^`a^b`^ y~**hi**~ \u{e000}0\u{e001} `ok`"),
            "xa^b yhi \u{e000}0\u{e001} ok"
        );
        assert_eq!(
            heading_title("<code>^a^ <b>x</b> &amp;</code> ^b^"),
            "^a^ <b>x</b> & b"
        );
    }
    #[test]
    fn headings_allocate_global_collisions_and_empty_fallback() {
        let mut ids = HeadingIds::default();
        let actual: Vec<_> = ["重复", "重复", "重复-2", "😀", "heading-4"]
            .iter()
            .enumerate()
            .map(|(i, title)| ids.allocate(title, i + 1))
            .collect();
        assert_eq!(
            actual,
            ["重复", "重复-2", "重复-2-2", "heading-4", "heading-4-2"]
        );
        assert_eq!(normalize_label("  ẞ\tX "), normalize_label("ss x"));
    }
}
