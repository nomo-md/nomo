//! 片段解析适配：不加载文件、不查询全文，解析结果由源码与版本共同标识。
use super::protocol::{ControlError, DocumentWindow, ParseContext, WindowContext, WindowIdentity};
use pulldown_cmark::{Event, Options, Tag, TagEnd};
use std::{ops::Range, sync::Arc};

pub use crate::markdown_templates::InlineFormat;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSpan {
    pub format: InlineFormat,
    pub open: Range<usize>,
    pub content: Range<usize>,
    pub close: Range<usize>,
}
impl InlineSpan {
    pub fn range(&self) -> Range<usize> {
        self.open.start..self.close.end
    }
    pub fn intersects(&self, selection: &Range<usize>) -> bool {
        if selection.is_empty() {
            selection.start >= self.open.start && selection.start <= self.close.end
        } else {
            selection.start < self.close.end && selection.end > self.open.start
        }
    }
}

#[derive(Clone, Debug)]
pub struct SemanticNode {
    pub kind: &'static str,
    pub source: Range<usize>,
    pub parent: Option<usize>,
}

#[derive(Clone)]
pub struct ParsedWindow {
    pub identity: WindowIdentity,
    pub source: Arc<str>,
    pub events: Vec<(Event<'static>, Range<usize>)>,
    pub inline: Vec<InlineSpan>,
    pub nodes: Vec<SemanticNode>,
    pub context: ParseContext,
    pub details: Vec<super::dialect::DetailsBlock>,
    pub html_tables: Vec<super::html_table::HtmlTable>,
    /// 原文中额外空行对应的可编辑空段落锚点；普通块间分隔换行不额外占行。
    pub blank_paragraphs: Vec<usize>,
}
impl ParsedWindow {
    pub fn parse(
        window: DocumentWindow,
        window_id: impl Into<String>,
        generation: u64,
        context: WindowContext,
    ) -> Result<Self, ControlError> {
        let mut context = match context {
            WindowContext::Complete => ParseContext {
                structural_boundary: true,
                ..Default::default()
            },
            WindowContext::Resolved(context) if context.structural_boundary => context,
            WindowContext::Resolved(_) => {
                return Err(ControlError::NeedsContext(
                    "片段不在可独立解析的结构边界".into(),
                ));
            }
            WindowContext::Missing { reason } => return Err(ControlError::NeedsContext(reason)),
        };
        if window.to_byte.checked_sub(window.from_byte) != Some(window.text.len() as u64) {
            return Err(ControlError::InvalidRange);
        }
        if !crate::markdown_templates::valid_set(&window.inline_templates)
            || window
                .inline_templates
                .iter()
                .any(|t| !t.valid_in(&window.text, window.from_byte))
        {
            return Err(ControlError::NeedsContext(
                "空模板范围或来源与窗口正文不一致".into(),
            ));
        }
        context.inline_templates = window.inline_templates.clone();
        let identity = WindowIdentity::from_window(&window, window_id, generation);
        Ok(Self::from_source_with_context(
            identity,
            Arc::from(window.text),
            context,
        ))
    }

    pub fn from_source(identity: WindowIdentity, source: Arc<str>) -> Self {
        Self::from_source_with_context(
            identity,
            source,
            ParseContext {
                structural_boundary: true,
                ..Default::default()
            },
        )
    }
    pub fn from_source_with_context(
        identity: WindowIdentity,
        source: Arc<str>,
        context: ParseContext,
    ) -> Self {
        let masked = super::template_projection::masked_source(
            &source,
            identity.from_byte,
            &context.inline_templates,
        );
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_MATH
            | Options::ENABLE_GFM
            | Options::ENABLE_SUPERSCRIPT
            | Options::ENABLE_SUBSCRIPT;
        let mut raw_events: Vec<(Event<'static>, Range<usize>)> = Vec::new();
        // 保留 String 容量后再转为事件；逐事件重建整个前缀会让长代码块变成平方复制。
        let mut pending_text: Option<(String, Range<usize>)> = None;
        let flush = |pending: &mut Option<(String, Range<usize>)>, output: &mut Vec<_>| {
            if let Some((text, range)) = pending.take() {
                output.push((Event::Text(text.into()), range));
            }
        };
        let mut details = Vec::new();
        for (event, range) in super::dialect::events(
            &masked,
            0,
            identity.from_byte == 0,
            options,
            &context,
            &mut details,
        ) {
            // 解析库可把普通 ~、^ 分成相邻事件；先合并连续且逐字节对应的文本。
            if let Event::Text(ref text) = event {
                if masked.get(range.clone()) == Some(text.as_ref()) {
                    if let Some((previous, previous_range)) = pending_text.as_mut() {
                        if previous_range.end == range.start {
                            previous_range.end = range.end;
                            previous.push_str(text);
                            continue;
                        }
                    }
                    flush(&mut pending_text, &mut raw_events);
                    pending_text = Some((text.to_string(), range));
                    continue;
                }
            }
            flush(&mut pending_text, &mut raw_events);
            raw_events.push((event.into_static(), range));
        }
        flush(&mut pending_text, &mut raw_events);
        let raw_events = super::template_projection::restore_events(
            raw_events,
            &source,
            identity.from_byte,
            &context.inline_templates,
        );
        let mut events = Vec::with_capacity(raw_events.len());
        let mut in_code = false;
        for (event, range) in raw_events {
            match event {
                Event::Start(Tag::CodeBlock(_)) => in_code = true,
                Event::End(TagEnd::CodeBlock) => in_code = false,
                _ => {}
            }
            // Nomo 接受紧贴字母的上下标；pulldown-cmark 默认规则不会产生这些事件。
            if let Event::Text(ref text) = event {
                if !in_code && !text.is_empty() && source.get(range.clone()) == Some(text.as_ref())
                {
                    expand_script_text(&source, range, &mut events);
                    continue;
                }
            }
            events.push((event.into_static(), range));
        }
        let mut inline = Vec::new();
        let mut nodes = Vec::new();
        let mut parents = Vec::new();
        let mut html_stack: Vec<(InlineFormat, Range<usize>)> = Vec::new();
        for (event, range) in &events {
            match event {
                Event::Start(tag) => {
                    let node = nodes.len();
                    nodes.push(SemanticNode {
                        kind: node_kind(tag),
                        source: range.clone(),
                        parent: parents.last().copied(),
                    });
                    parents.push(node);
                    let marker = match tag {
                        Tag::Strong => Some((InlineFormat::Strong, 2, 2)),
                        Tag::Emphasis => Some((InlineFormat::Emphasis, 1, 1)),
                        Tag::Strikethrough => Some((InlineFormat::Strike, 2, 2)),
                        Tag::Superscript => Some((InlineFormat::Superscript, 1, 1)),
                        Tag::Subscript => Some((InlineFormat::Subscript, 1, 1)),
                        _ => None,
                    };
                    if let Some((format, left, right)) = marker {
                        if range.len() >= left + right {
                            inline.push(InlineSpan {
                                format,
                                open: range.start..range.start + left,
                                content: range.start + left..range.end - right,
                                close: range.end - right..range.end,
                            });
                        }
                    }
                    if matches!(tag, Tag::Link { .. }) {
                        let raw = &source[range.clone()];
                        if let Some(end) = raw.find("](").or_else(|| raw.find("][")) {
                            inline.push(InlineSpan {
                                format: InlineFormat::Link,
                                open: range.start..range.start + 1,
                                content: range.start + 1..range.start + end,
                                close: range.start + end..range.end,
                            });
                        }
                    }
                }
                Event::End(_) => {
                    parents.pop();
                }
                Event::Code(_) => {
                    let raw = &source[range.clone()];
                    let ticks = raw.bytes().take_while(|b| *b == 96).count();
                    if ticks > 0 && range.len() >= 2 * ticks {
                        inline.push(InlineSpan {
                            format: InlineFormat::Code,
                            open: range.start..range.start + ticks,
                            content: range.start + ticks..range.end - ticks,
                            close: range.end - ticks..range.end,
                        });
                    }
                }
                Event::InlineHtml(html) => {
                    if let Some((format, close)) = html_format(html) {
                        if close {
                            if let Some(index) =
                                html_stack.iter().rposition(|(kind, _)| *kind == format)
                            {
                                let (_, open) = html_stack.remove(index);
                                inline.push(InlineSpan {
                                    format,
                                    content: open.end..range.start,
                                    open,
                                    close: range.clone(),
                                });
                            }
                        } else {
                            html_stack.push((format, range.clone()));
                        }
                    }
                }
                _ => {}
            }
        }
        let mut html_tables = Vec::new();
        for (event, range) in &events {
            if matches!(event, Event::Start(Tag::HtmlBlock)) {
                if let Some(table) = super::html_table::HtmlTable::parse(&source, range.clone()) {
                    inline.extend(table.cells.iter().flat_map(|cell| cell.spans.clone()));
                    html_tables.push(table);
                }
            }
        }
        inline.sort_by_key(|span| (span.open.start, std::cmp::Reverse(span.close.end)));
        let blank_paragraphs = blank_paragraphs(&source, &events);
        Self {
            identity,
            source,
            events,
            inline,
            nodes,
            context,
            details,
            html_tables,
            blank_paragraphs,
        }
    }

    pub fn estimated_bytes(&self) -> usize {
        self.source.len()
            + self.context.inline_templates.capacity()
                * std::mem::size_of::<super::protocol::InlineTemplate>()
            + self.blank_paragraphs.capacity() * std::mem::size_of::<usize>()
            + self.events.capacity() * std::mem::size_of::<(Event<'static>, Range<usize>)>()
            + self
                .events
                .iter()
                .map(|(event, _)| match event {
                    Event::Text(s)
                    | Event::Code(s)
                    | Event::Html(s)
                    | Event::InlineHtml(s)
                    | Event::InlineMath(s)
                    | Event::DisplayMath(s)
                    | Event::FootnoteReference(s) => s.len(),
                    Event::Start(Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(info))) => {
                        info.len()
                    }
                    Event::Start(
                        Tag::Link {
                            dest_url,
                            title,
                            id,
                            ..
                        }
                        | Tag::Image {
                            dest_url,
                            title,
                            id,
                            ..
                        },
                    ) => dest_url.len() + title.len() + id.len(),
                    Event::Start(Tag::FootnoteDefinition(s)) => s.len(),
                    Event::Start(Tag::Table(columns)) => {
                        columns.capacity() * std::mem::size_of::<pulldown_cmark::Alignment>()
                    }
                    _ => 0,
                })
                .sum::<usize>()
            + self.inline.capacity() * std::mem::size_of::<InlineSpan>()
            + self.nodes.capacity() * std::mem::size_of::<SemanticNode>()
            + self.details.capacity() * std::mem::size_of::<super::dialect::DetailsBlock>()
            + self
                .context
                .references
                .iter()
                .map(|(key, value)| {
                    key.capacity()
                        + value.destination.capacity()
                        + value.title.capacity()
                        + std::mem::size_of_val(value)
                        + std::mem::size_of::<String>()
                        + 4 * std::mem::size_of::<usize>()
                })
                .sum::<usize>()
            + self.context.ancestors.capacity()
                * std::mem::size_of::<super::protocol::AncestorContainer>()
            + self
                .context
                .ancestors
                .iter()
                .map(|ancestor| match ancestor {
                    super::protocol::AncestorContainer::Details { summary, .. } => {
                        summary.capacity()
                    }
                    _ => 0,
                })
                .sum::<usize>()
            + self
                .html_tables
                .iter()
                .map(|table| {
                    std::mem::size_of::<super::html_table::HtmlTable>()
                        + table
                            .cells
                            .iter()
                            .map(|cell| {
                                std::mem::size_of::<super::html_table::Cell>()
                                    + cell.widths.len() * 4
                                    + cell.spans.len() * std::mem::size_of::<InlineSpan>()
                                    + cell
                                        .runs
                                        .iter()
                                        .map(|run| {
                                            std::mem::size_of::<super::html_table::InlineRun>()
                                                + run.text.len()
                                                + run.formats.len()
                                                    * std::mem::size_of::<InlineFormat>()
                                                + run.link.as_ref().map_or(0, String::len)
                                                + run.atom.as_ref().map_or(0, super::html_table::InlineAtom::estimated_bytes)
                                        })
                                        .sum::<usize>()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    pub fn markers_for(&self, selection: &Range<usize>) -> Vec<Range<usize>> {
        self.inline
            .iter()
            .filter(|span| span.intersects(selection))
            .flat_map(|span| [span.open.clone(), span.close.clone()])
            .collect()
    }

    /// 显示投影仍保留语义事件；只在真实语法位置插入当前需要显示的符号。
    pub fn display_events(&self, revealed: &[Range<usize>]) -> Vec<(Event<'static>, Range<usize>)> {
        if revealed.is_empty() {
            return self.events.clone();
        }
        let mut output = Vec::with_capacity(self.events.len() + revealed.len());
        let mut emitted = std::collections::HashSet::new();
        let revealed: std::collections::HashSet<_> =
            revealed.iter().map(|r| (r.start, r.end)).collect();
        let mut spans: std::collections::HashMap<_, Vec<&InlineSpan>> =
            std::collections::HashMap::new();
        for span in &self.inline {
            spans
                .entry((span.open.start, span.close.end))
                .or_default()
                .push(span);
        }
        for (event, range) in &self.events {
            if matches!(event, Event::InlineHtml(_))
                && revealed.contains(&(range.start, range.end))
                && emitted.insert((range.start, range.end))
            {
                output.push((
                    Event::Text(self.source[range.clone()].to_owned().into()),
                    range.clone(),
                ));
            }
            let relevant = spans
                .get(&(range.start, range.end))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let before = relevant.iter().filter_map(|span| match event {
                Event::Start(tag) if tag_format(tag) == Some(span.format) => {
                    Some(span.open.clone())
                }
                Event::End(tag) if end_format(tag) == Some(span.format) => Some(span.close.clone()),
                Event::Code(_) if span.format == InlineFormat::Code => Some(span.open.clone()),
                _ => None,
            });
            for marker in before {
                if revealed.contains(&(marker.start, marker.end))
                    && emitted.insert((marker.start, marker.end))
                {
                    output.push((
                        Event::Text(self.source[marker.clone()].to_owned().into()),
                        marker,
                    ));
                }
            }
            output.push((event.clone(), range.clone()));
            if matches!(event, Event::Code(_)) {
                for span in relevant
                    .iter()
                    .filter(|span| span.format == InlineFormat::Code)
                {
                    let marker = span.close.clone();
                    if revealed.contains(&(marker.start, marker.end))
                        && emitted.insert((marker.start, marker.end))
                    {
                        output.push((
                            Event::Text(self.source[marker.clone()].to_owned().into()),
                            marker,
                        ));
                    }
                }
            }
        }
        output
    }

    pub fn with_blank_paragraphs(
        &self,
        input: Vec<(Event<'static>, Range<usize>)>,
    ) -> Vec<(Event<'static>, Range<usize>)> {
        let mut output = Vec::with_capacity(input.len() + self.blank_paragraphs.len() * 3);
        let mut blanks = self.blank_paragraphs.iter().copied().peekable();
        let mut depth = 0usize;
        let append = |at, output: &mut Vec<_>| {
            output.push((Event::Start(Tag::Paragraph), at..at));
            output.push((Event::End(TagEnd::Paragraph), at..at));
        };
        for (event, range) in input {
            if depth == 0 {
                while blanks.peek().is_some_and(|at| *at <= range.start) {
                    append(blanks.next().unwrap(), &mut output);
                }
            }
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth = depth.saturating_sub(1),
                _ => {}
            }
            output.push((event, range));
        }
        for at in blanks {
            append(at, &mut output);
        }
        output
    }
}

fn blank_paragraphs(source: &str, events: &[(Event<'static>, Range<usize>)]) -> Vec<usize> {
    let mut roots = Vec::new();
    let mut depth = 0usize;
    for (event, range) in events {
        if depth == 0 && matches!(event, Event::Start(_) | Event::Rule | Event::DisplayMath(_)) {
            roots.push(range.clone());
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut result = Vec::new();
    let mut end = 0;
    for root in &roots {
        let gap = &source[end..root.start];
        if gap.trim().is_empty() {
            let lines: Vec<_> = gap
                .match_indices('\n')
                .map(|(at, _)| end + at + 1)
                .collect();
            let skip = if end == 0 { 0 } else { 1 };
            for i in (skip..lines.len()).step_by(2) {
                if i + 1 < lines.len() {
                    result.push(if i == 0 { end } else { lines[i - 1] });
                }
            }
        }
        end = root.end.max(end);
    }
    if source[end..].trim().is_empty() {
        let lines: Vec<_> = source[end..]
            .match_indices('\n')
            .map(|(at, _)| end + at + 1)
            .collect();
        if roots.is_empty() {
            result.push(0);
        }
        for i in (0..lines.len()).step_by(2) {
            result.push(lines[(i + 1).min(lines.len() - 1)]);
        }
    }
    result.dedup();
    result
}

fn tag_format(tag: &Tag<'_>) -> Option<InlineFormat> {
    match tag {
        Tag::Strong => Some(InlineFormat::Strong),
        Tag::Emphasis => Some(InlineFormat::Emphasis),
        Tag::Strikethrough => Some(InlineFormat::Strike),
        Tag::Superscript => Some(InlineFormat::Superscript),
        Tag::Subscript => Some(InlineFormat::Subscript),
        Tag::Link { .. } => Some(InlineFormat::Link),
        _ => None,
    }
}
fn end_format(tag: &TagEnd) -> Option<InlineFormat> {
    match tag {
        TagEnd::Strong => Some(InlineFormat::Strong),
        TagEnd::Emphasis => Some(InlineFormat::Emphasis),
        TagEnd::Strikethrough => Some(InlineFormat::Strike),
        TagEnd::Superscript => Some(InlineFormat::Superscript),
        TagEnd::Subscript => Some(InlineFormat::Subscript),
        TagEnd::Link => Some(InlineFormat::Link),
        _ => None,
    }
}

fn node_kind(tag: &Tag<'_>) -> &'static str {
    match tag {
        Tag::Paragraph => "paragraph",
        Tag::Heading { .. } => "heading",
        Tag::BlockQuote(_) => "quote",
        Tag::CodeBlock(_) => "code",
        Tag::List(_) => "list",
        Tag::Item => "item",
        Tag::Table(_) => "table",
        Tag::TableHead => "table-head",
        Tag::TableRow => "table-row",
        Tag::TableCell => "table-cell",
        Tag::HtmlBlock => "html",
        Tag::FootnoteDefinition(_) => "footnote",
        Tag::Image { .. } => "image",
        Tag::Link { .. } => "link",
        _ => "inline",
    }
}
fn html_format(html: &str) -> Option<(InlineFormat, bool)> {
    let tag = html.trim().to_ascii_lowercase();
    let close = tag.starts_with("</");
    let name = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split([' ', '\t', '>'])
        .next()?;
    let format = match name {
        "strong" | "b" => InlineFormat::Strong,
        "em" | "i" => InlineFormat::Emphasis,
        "code" => InlineFormat::Code,
        "u" => InlineFormat::Underline,
        "mark" => InlineFormat::Highlight,
        "sup" => InlineFormat::Superscript,
        "sub" => InlineFormat::Subscript,
        "del" | "s" | "strike" => InlineFormat::Strike,
        _ => return None,
    };
    Some((format, close))
}
fn expand_script_text(
    source: &str,
    range: Range<usize>,
    output: &mut Vec<(Event<'static>, Range<usize>)>,
) {
    let bytes = source.as_bytes();
    let mut at = range.start;
    let mut plain = at;
    while at < range.end {
        let marker = bytes[at];
        let escaped = at > 0
            && bytes[..at]
                .iter()
                .rev()
                .take_while(|b| **b == b'\\')
                .count()
                % 2
                == 1;
        if (marker == b'^' || marker == b'~')
            && !escaped
            && !(at > range.start && bytes[at - 1] == marker)
            && bytes.get(at + 1) != Some(&marker)
        {
            let close = (at + 1..range.end).find(|end| bytes[*end] == marker);
            if let Some(end) = close {
                let body = &source[at + 1..end];
                if !body.is_empty()
                    && !body.chars().any(char::is_whitespace)
                    && !body.contains('\\')
                    && bytes.get(end + 1) != Some(&marker)
                {
                    if at > plain {
                        output.push((Event::Text(source[plain..at].to_owned().into()), plain..at));
                    }
                    let (start, stop) = if marker == b'^' {
                        (Tag::Superscript, TagEnd::Superscript)
                    } else {
                        (Tag::Subscript, TagEnd::Subscript)
                    };
                    output.push((Event::Start(start), at..end + 1));
                    output.push((Event::Text(body.to_owned().into()), at + 1..end));
                    output.push((Event::End(stop), at..end + 1));
                    at = end + 1;
                    plain = at;
                    continue;
                }
            }
        }
        at += 1;
    }
    if plain < range.end {
        output.push((
            Event::Text(source[plain..range.end].to_owned().into()),
            plain..range.end,
        ));
    }
}

#[cfg(test)]
pub fn fixture(text: &str) -> ParsedWindow {
    ParsedWindow::parse(
        DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "doc".into(),
            revision: 4,
            from_byte: 100,
            to_byte: 100 + text.len() as u64,
            text: text.into(),
        },
        "window",
        2,
        WindowContext::Complete,
    )
    .unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_formats_have_real_delimiter_ranges() {
        let parsed = fixture("中**粗*斜*体** <u>下划线</u> `code`");
        assert_eq!(parsed.inline.len(), 4);
        for span in &parsed.inline {
            assert!(parsed.source.is_char_boundary(span.open.start));
            assert!(span.open.end <= span.content.start && span.content.end <= span.close.start);
        }
        let caret = parsed.source.find("斜").unwrap();
        assert_eq!(parsed.markers_for(&(caret..caret)).len(), 4);
    }
    #[test]
    fn nomo_script_dialect_excludes_code_and_escapes() {
        let parsed = fixture("H~2~O x^2^ `x^2^` \\^literal\\^");
        assert_eq!(
            parsed
                .inline
                .iter()
                .filter(|s| s.format == InlineFormat::Subscript)
                .count(),
            1
        );
        assert_eq!(
            parsed
                .inline
                .iter()
                .filter(|s| s.format == InlineFormat::Superscript)
                .count(),
            1
        );
    }
    #[test]
    fn incomplete_context_is_never_silently_parsed() {
        let input = DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "doc".into(),
            revision: 1,
            from_byte: 9,
            to_byte: 10,
            text: "x".into(),
        };
        assert!(matches!(
            ParsedWindow::parse(
                input,
                "w",
                0,
                WindowContext::Missing {
                    reason: "缺少引用定义".into()
                }
            ),
            Err(ControlError::NeedsContext(_))
        ));
    }
    #[test]
    fn fenced_code_keeps_literal_script_markers_and_crlf() {
        let parsed = fixture("```text\r\nH~2~O x^2^ **literal**\r\n```\r\n");
        assert!(parsed.inline.is_empty());
        let body: String = parsed
            .events
            .iter()
            .filter_map(|(event, _)| match event {
                Event::Text(s) => Some(s.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(body, "H~2~O x^2^ **literal**\n");
    }
    #[test]
    fn additional_empty_paragraphs_have_real_source_anchors() {
        for (source, expected) in [
            ("a\n\nb", vec![]),
            ("a\n\n\n\nb", vec![3]),
            ("\r\n\r\nb", vec![0]),
            ("a\n\n", vec![3]),
            ("", vec![0]),
        ] {
            let parsed = fixture(source);
            assert_eq!(parsed.blank_paragraphs, expected, "{source:?}");
            let events = parsed.with_blank_paragraphs(parsed.display_events(&[]));
            assert_eq!(
                events
                    .iter()
                    .filter(|(e, r)| r.is_empty() && matches!(e, Event::Start(Tag::Paragraph)))
                    .count(),
                expected.len()
            );
        }
    }
    #[test]
    fn host_reference_context_resolves_without_prefixing_or_moving_source() {
        let text = "[说明][ref]";
        let mut context = ParseContext {
            structural_boundary: true,
            ..Default::default()
        };
        context.references.insert(
            "ref".into(),
            super::super::protocol::ReferenceTarget {
                destination: "https://example.com".into(),
                title: "外部定义".into(),
            },
        );
        let parsed = ParsedWindow::parse(
            DocumentWindow {
                inline_templates: Vec::new(),
                document_id: "doc".into(),
                revision: 1,
                from_byte: 1000,
                to_byte: 1000 + text.len() as u64,
                text: text.into(),
            },
            "w",
            1,
            WindowContext::Resolved(context),
        )
        .unwrap();
        assert!(parsed.events.iter().any(|(event,range)|matches!(event,Event::Start(Tag::Link{dest_url,..})if dest_url.as_ref()=="https://example.com")&&range.start==0));
        assert_eq!(&*parsed.source, text);
    }
    #[test]
    fn reveal_keeps_semantic_nodes_and_emits_each_delimiter_once() {
        let parsed = fixture("**bold *nested***");
        let at = parsed.source.find("nested").unwrap();
        let events = parsed.display_events(&parsed.markers_for(&(at..at)));
        assert!(matches!(&events[0].0, Event::Start(Tag::Paragraph)));
        let display: String = events
            .iter()
            .filter_map(|(e, _)| {
                if let Event::Text(s) = e {
                    Some(s.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(display, &*parsed.source);
        assert!(
            events
                .iter()
                .any(|(event, _)| matches!(event, Event::Start(Tag::Strong)))
        );
    }
}
