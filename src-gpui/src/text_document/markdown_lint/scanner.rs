use super::*;
use crate::text_document::chunk_reader::DocumentSnapshot;
use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, Tag};
use std::time::{Duration, Instant};

pub(super) struct Syntax<'a> {
    pub source: &'a str,
    pub lines: Vec<Range<usize>>,
    pub events: Vec<(Event<'a>, Range<usize>)>,
    pub code: Vec<bool>,
    pub html: Vec<bool>,
    pub tables: Vec<bool>,
    pub comments: Vec<bool>,
    pub body_line: usize,
    pub title: bool,
    pub missing: Vec<Range<usize>>,
    pub definitions: Vec<(String, String, Range<usize>)>,
    pub config: super::directives::Configuration,
}
impl Syntax<'_> {
    pub fn line_at(&self, byte: usize) -> usize {
        self.lines
            .partition_point(|line| line.start <= byte)
            .saturating_sub(1)
    }
    pub fn line(&self, index: usize) -> &str {
        &self.source[self.lines[index].clone()]
    }
    pub fn last_line(&self, range: &Range<usize>) -> usize {
        self.line_at(range.end.saturating_sub(1).max(range.start))
    }
    pub fn blank(&self, index: usize) -> bool {
        self.line(index)
            .trim_matches(|c: char| c.is_whitespace() || c == '>')
            .is_empty()
    }
}

pub(in crate::text_document) fn scan(
    snapshot: &DocumentSnapshot,
    report: Report,
    request: &Request,
) -> TextDocumentResult<Report> {
    let started = Instant::now();
    let mut bytes = Vec::with_capacity(snapshot.len() as usize);
    while (bytes.len() as u64) < snapshot.len() {
        checkpoint(request, started)?;
        bytes.extend(snapshot.read_range(
            bytes.len() as u64,
            (16 * 1024).min(snapshot.len() as usize - bytes.len()),
        )?);
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| TextDocumentError::new("lint-utf8", "检查快照不是有效 UTF-8"))?;
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    let raw = source.as_bytes();
    while at < raw.len() {
        if raw[at] == b'\r' || raw[at] == b'\n' {
            lines.push(start..at);
            if lines.len() >= MAX_LINES {
                return Ok(report.skip(SkipReason::LineBudget));
            }
            if raw[at] == b'\r' && raw.get(at + 1) == Some(&b'\n') {
                at += 1;
            }
            start = at + 1;
        }
        at += 1;
    }
    lines.push(start..source.len());
    let first = source[lines[0].clone()].trim_start_matches('\u{feff}');
    let mut body_line = 0;
    let mut title = false;
    if matches!(first, "---" | "+++") {
        if let Some(end) = (1..lines.len()).find(|index| {
            let line = source[lines[*index].clone()].trim_end();
            line == first || (first == "---" && line == "...")
        }) {
            title = lines[1..end].iter().any(|range| {
                let line = source[range.clone()].trim_start();
                line.strip_prefix("title")
                    .is_some_and(|rest| rest.trim_start().starts_with([':', '=']))
            });
            body_line = end + 1;
        }
    }
    let body_start = lines
        .get(body_line)
        .map_or(source.len(), |range| range.start);
    let body_start = if body_start == 0 && source.starts_with('\u{feff}') {
        3
    } else {
        body_start
    };
    let mut missing = Vec::new();
    let mut missing_overflow = false;
    let callback = |link: BrokenLink<'_>| {
        if matches!(link.link_type, LinkType::Reference | LinkType::Collapsed)
            && !link.reference.eq_ignore_ascii_case("x")
        {
            if missing.len() < MAX_EVENTS {
                missing.push(link.span.start + body_start..link.span.end + body_start);
            } else {
                missing_overflow = true;
            }
        }
        None
    };
    // 与原版 lint 的 GFM 语法相同范围；Nomo 的公式、Details 编辑模型不参与检查。
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;
    let parser =
        Parser::new_with_broken_link_callback(&source[body_start..], options, Some(callback));
    let definitions = parser
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            (
                label.to_owned(),
                def.dest.to_string(),
                def.span.start + body_start..def.span.end + body_start,
            )
        })
        .collect();
    let mut events = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        checkpoint(request, started)?;
        if events.len() == MAX_EVENTS {
            return Ok(report.skip(SkipReason::SyntaxBudget));
        }
        events.push((event, range.start + body_start..range.end + body_start));
    }
    if missing_overflow {
        return Ok(report.skip(SkipReason::SyntaxBudget));
    }
    let mut syntax = Syntax {
        source: &source,
        code: vec![false; lines.len()],
        html: vec![false; lines.len()],
        tables: vec![false; lines.len()],
        comments: vec![false; lines.len()],
        lines,
        events,
        body_line,
        title,
        missing,
        definitions,
        config: super::directives::Configuration::default(),
    };
    for (event, range) in &syntax.events {
        let first = syntax.line_at(range.start);
        let last = syntax.last_line(range);
        match event {
            Event::Start(Tag::CodeBlock(_)) => syntax.code[first..=last].fill(true),
            Event::Start(Tag::HtmlBlock) => syntax.html[first..=last].fill(true),
            Event::Start(Tag::Table(_)) => syntax.tables[first..=last].fill(true),
            Event::Html(text) if text.trim_start().starts_with("<!--") => {
                syntax.comments[first..=last].fill(true)
            }
            _ => {}
        }
    }
    syntax.config = super::directives::read(&syntax, report.rule_set, request, started)?;
    let mut sink = Sink::new(&source, &syntax.lines, &syntax.config.lines, report);
    rules::check(&syntax, &mut sink, request, started)?;
    checkpoint(request, started)?;
    Ok(sink.finish())
}

pub(super) fn checkpoint(request: &Request, started: Instant) -> TextDocumentResult<()> {
    request.check()?;
    if started.elapsed() > Duration::from_secs(3) {
        return Err(TextDocumentError::new(
            "lint-timeout",
            "格式检查超过 3 秒，已停止；可以手动重查",
        ));
    }
    Ok(())
}
