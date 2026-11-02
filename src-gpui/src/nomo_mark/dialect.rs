//! 将原版 Nomo 的结构边界适配成有真实源码范围的事件，布局阶段无需再次解析。
use super::{nomo_syntax, protocol::ParseContext};
use crate::text_document::normalize_reference_label;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{cell::RefCell, collections::BTreeSet, ops::Range};

#[derive(Clone, Debug)]
pub struct DetailsBlock {
    pub source: Range<usize>,
    pub summary: Range<usize>,
    pub body: Range<usize>,
    pub default_open: bool,
}
impl DetailsBlock {
    /// summary 保留完整标签供结构解析使用；输入落点只能位于标签内。
    pub fn title(&self, source: &str) -> Range<usize> {
        let Some(open) = nomo_syntax::html_tag(source, self.summary.start)
            .filter(|tag| tag.name == "summary" && !tag.closing) else { return self.summary.start..self.summary.start; };
        let end = source[open.end..self.summary.end].to_ascii_lowercase().rfind("</summary")
            .map_or(open.end, |at| open.end + at);
        open.end..end
    }

    /// 空正文使用标题分隔行之后的同一落点绘制光标和接收输入。
    pub fn empty_body_caret(&self, source: &str) -> Option<usize> {
        let body = &source[self.body.clone()];
        body.trim().is_empty().then(|| {
            self.body.start + body.match_indices('\n').take(2).last().map_or(0, |(at, _)| at + 1)
        })
    }

    pub fn unwrapped(&self, source: &str) -> String {
        let eol = super::commands::line_ending(source, self.source.start);
        let title = source[self.title(source)].trim();
        let body = source[self.body.clone()].trim_matches(['\r', '\n']);
        match (title.is_empty(), body.is_empty()) {
            (true, _) => body.to_owned(),
            (_, true) => title.to_owned(),
            _ => format!("{title}{eol}{eol}{body}"),
        }
    }
}
pub fn events(
    source: &str,
    offset: usize,
    document_start: bool,
    options: Options,
    context: &ParseContext,
    details: &mut Vec<DetailsBlock>,
    unresolved: &mut Vec<Range<usize>>,
    reference_keys: &mut BTreeSet<String>,
) -> Vec<(Event<'static>, Range<usize>)> {
    let (projected, empty_items) = project_empty_list_items(source, options, document_start);
    if !empty_items.is_empty() {
        let details_from = details.len();
        let unresolved_from = unresolved.len();
        let mut result = events(&projected, offset, document_start, options, context, details, unresolved, reference_keys);
        let map = |at: usize| at - empty_items.partition_point(|item| offset + item.projected < at);
        for (event, range) in &mut result {
            if matches!(event, Event::Text(_)) {
                let index = empty_items.partition_point(|item| offset + item.projected < range.start);
                if let Some(item) = empty_items.get(index).filter(|item| offset + item.projected < range.end) {
                    *event = Event::Text("".into());
                    *range = offset + item.anchor..offset + item.anchor;
                    continue;
                }
            }
            *range = map(range.start)..map(range.end);
            if let Event::Start(Tag::List(Some(number))) = event {
                let index = empty_items.partition_point(|item| offset + item.marker < range.start);
                if let Some(original) = empty_items.get(index).filter(|item| offset + item.line <= range.start && range.start <= offset + item.marker)
                    .and_then(|item| item.number)
                { *number = original; }
            }
        }
        for block in &mut details[details_from..] {
            block.source = map(block.source.start)..map(block.source.end);
            block.summary = map(block.summary.start)..map(block.summary.end);
            block.body = map(block.body.start)..map(block.body.end);
        }
        for range in &mut unresolved[unresolved_from..] { *range = map(range.start)..map(range.end); }
        return result;
    }
    let mut result = Vec::new();
    let front = if document_start {
        nomo_syntax::front_matter_end(source)
    } else {
        None
    };
    let from = front.unwrap_or(0);
    if let Some(end) = front {
        let range = offset..offset + end;
        result.push((Event::Start(Tag::HtmlBlock), range.clone()));
        result.push((Event::Html(source[..end].to_owned().into()), range.clone()));
        result.push((Event::End(TagEnd::HtmlBlock), range));
    }
    let keys = RefCell::new(BTreeSet::new());
    let mut resolve = |link: pulldown_cmark::BrokenLink<'_>| {
        let key = normalize_reference_label(&link.reference);
        if context.pending_references { keys.borrow_mut().insert(key.clone()); }
        if let Some(target) = context.references.get(&key) {
            Some((target.destination.clone().into(), target.title.clone().into()))
        } else if context.pending_references && matches!(link.link_type, pulldown_cmark::LinkType::Reference | pulldown_cmark::LinkType::Collapsed) {
            // 让解析器保留标签及其行内格式。空地址只承载事件，交互以独立待定范围为准。
            Some(("".into(), "".into()))
        } else {
            // 单独的 [文字] 也可能只是字面文本，不能提前隐藏括号。
            None
        }
    };
    let projected = nomo_syntax::project_comment_paragraphs(&source[from..], options);
    let parser =
        Parser::new_with_broken_link_callback(&projected, options, Some(&mut resolve));
    let mut skip: Option<Range<usize>> = None;
    for (mut event, range) in parser.into_offset_iter() {
        let range = from + range.start..from + range.end;
        event = nomo_syntax::restore_inline_comment(source, event, &range);
        if skip
            .as_ref()
            .is_some_and(|skip| skip.start <= range.start && range.start < skip.end)
        {
            continue;
        }
        if let Event::Start(Tag::Link { link_type, id, dest_url, title } | Tag::Image { link_type, id, dest_url, title }) = &mut event {
            if !matches!(link_type, pulldown_cmark::LinkType::Inline | pulldown_cmark::LinkType::Autolink | pulldown_cmark::LinkType::Email | pulldown_cmark::LinkType::WikiLink { .. }) {
                let key = normalize_reference_label(id);
                if context.pending_references && !context.references.contains_key(&key)
                    && matches!(link_type, pulldown_cmark::LinkType::ReferenceUnknown | pulldown_cmark::LinkType::CollapsedUnknown)
                {
                    unresolved.push(offset + range.start..offset + range.end);
                }
                if let Some(target) = context.references.get(&key) {
                    // 跨窗定义遵守全文首次定义，不能由窗口内较晚的同名定义覆盖。
                    *dest_url = target.destination.clone().into();
                    *title = target.title.clone().into();
                }
                if context.pending_references { keys.borrow_mut().insert(key); }
            }
        }
        if matches!(event, Event::Start(Tag::HtmlBlock)) {
            if let Some(tag) = nomo_syntax::html_tag(source, range.start)
                .filter(|tag| tag.name == "br" && !tag.closing && tag.end <= range.end
                    && source[tag.end..range.end].trim().is_empty())
            {
                // 空段落中的单个 <br> 会被解析成 HTML 块，仍应作为可编辑的段内换行。
                let absolute = offset + range.start..offset + range.end;
                result.push((Event::Start(Tag::Paragraph), absolute.clone()));
                result.push((
                    Event::InlineHtml(source[range.start..tag.end].to_owned().into()),
                    offset + range.start..offset + tag.end,
                ));
                result.push((Event::End(TagEnd::Paragraph), absolute));
                skip = Some(range);
                continue;
            }
            if let Some(block) = nomo_syntax::details(source, range.start) {
                if let Some(opening) = nomo_syntax::html_tag(source, range.start) {
                    let mut child_context = context.clone();
                    for (label, target) in parser_definitions(&source[from..], options) {
                        child_context.references.entry(label).or_insert(target);
                    }
                    let summary_start = opening.end + source[opening.end..block.body_from].len()
                        - source[opening.end..block.body_from].trim_start().len();
                    let whole = range.start..block.end;
                    details.push(DetailsBlock {
                        source: offset + whole.start..offset + whole.end,
                        summary: offset + summary_start..offset + block.body_from,
                        body: offset + block.body_from..offset + block.body_to,
                        default_open: opening.attrs.contains_key("open"),
                    });
                    let absolute = offset + whole.start..offset + whole.end;
                    result.push((Event::Start(Tag::HtmlBlock), absolute.clone()));
                    result.push((
                        Event::Html(source[range.start..opening.end].to_owned().into()),
                        offset + range.start..offset + opening.end,
                    ));
                    result.push((
                        Event::Html(source[summary_start..block.body_from].to_owned().into()),
                        offset + summary_start..offset + block.body_from,
                    ));
                    result.extend(events(
                        &source[block.body_from..block.body_to],
                        offset + block.body_from,
                        false,
                        options,
                        &child_context,
                        details,
                        unresolved,
                        reference_keys,
                    ));
                    result.push((
                        Event::Html(source[block.body_to..block.end].to_owned().into()),
                        offset + block.body_to..offset + block.end,
                    ));
                    result.push((Event::End(TagEnd::HtmlBlock), absolute));
                    skip = Some(whole);
                    continue;
                }
            }
        }
        result.push((
            event.into_static(),
            offset + range.start..offset + range.end,
        ));
    }
    reference_keys.extend(keys.into_inner());
    result
}

struct EmptyListItem {
    line: usize,
    marker: usize,
    insert: usize,
    anchor: usize,
    number: Option<u64>,
    projected: usize,
}

/// CommonMark 的空子项不能打断段落。临时补一个解析字符并映射回原文，维持编辑结构。
/// 仅有候选行时检查容器；投影及坐标表随本次窗口解析释放，不写回正文。
fn project_empty_list_items(source: &str, options: Options, document_start: bool) -> (std::borrow::Cow<'_, str>, Vec<EmptyListItem>) {
    if !source.contains('\n') { return (source.into(), Vec::new()); }
    let front = if document_start { nomo_syntax::front_matter_end(source).unwrap_or(0) } else { 0 };
    let mut candidates = Vec::new();
    let mut from = 0;
    for line in source.split_inclusive('\n') {
        let raw = line.trim_end_matches(['\r', '\n']);
        let prefix = raw.len() - raw.trim_start_matches([' ', '\t', '>']).len();
        let body = &raw[prefix..];
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        let marker = if body.starts_with(['-', '+', '*']) { 1 }
            else if (1..=9).contains(&digits) && matches!(body.as_bytes().get(digits), Some(b'.' | b')')) { digits + 1 }
            else { 0 };
        if from >= front && prefix > 0 && marker > 0 && body.len() > marker
            && body[marker..].bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        {
            candidates.push(EmptyListItem { line: from, marker: from + prefix, insert: from + prefix + marker + 1,
                anchor: from + raw.len(), number: if digits > 0 { body[..digits].parse().ok() } else { None }, projected: 0 });
        }
        from += line.len();
    }
    if candidates.is_empty() { return (source.into(), Vec::new()); }
    let mut containers = Parser::new_ext(source, options).into_offset_iter().filter_map(|(event, range)| match event {
        Event::Start(Tag::Item) => Some((true, range)),
        Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) => Some((false, range)),
        _ => None,
    }).peekable();
    let mut active = Vec::new();
    let mut items = Vec::new();
    for mut candidate in candidates {
        while containers.peek().is_some_and(|(_, range)| range.start <= candidate.insert) { active.push(containers.next().unwrap()); }
        active.retain(|(_, range)| range.end >= candidate.marker);
        let inside = active.iter().any(|(item, range)| *item && range.start < candidate.line && candidate.marker < range.end);
        let covered = active.iter().any(|(item, range)| {
            if *item { super::commands::line_start(source, range.start) == candidate.line && range.start <= candidate.marker && candidate.marker <= range.end }
            else { range.start <= candidate.marker && candidate.marker < range.end }
        });
        if inside && !covered {
            candidate.projected = candidate.insert + items.len();
            items.push(candidate);
        }
    }
    drop(containers);
    if items.is_empty() { return (source.into(), items); }
    let mut projected = String::with_capacity(source.len() + items.len());
    let mut from = 0;
    for item in &items {
        projected.push_str(&source[from..item.marker]);
        if item.number.is_some() {
            let digits = source[item.marker..].bytes().take_while(u8::is_ascii_digit).count();
            projected.push_str(&"0".repeat(digits - 1));
            projected.push('1');
            projected.push_str(&source[item.marker + digits..item.insert]);
        } else { projected.push_str(&source[item.marker..item.insert]); }
        projected.push('x');
        from = item.insert;
    }
    projected.push_str(&source[from..]);
    (projected.into(), items)
}
fn parser_definitions(
    source: &str,
    options: Options,
) -> Vec<(String, super::protocol::ReferenceTarget)> {
    let projected = nomo_syntax::project_comment_paragraphs(source, options);
    let parser = Parser::new_ext(&projected, options);
    parser
        .reference_definitions()
        .iter()
        .map(|(label, target)| {
            (
                normalize_reference_label(label),
                super::protocol::ReferenceTarget {
                    destination: target.dest.to_string(),
                    title: target
                        .title
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_details_use_original_boundaries_without_changing_source() {
        let mut text = "**中文**\r\n".to_owned();
        for _ in 0..12 {
            text = format!(
                "<details open>\r\n<summary>标题</summary>\r\n\r\n{text}\r\n</details>\r\n"
            );
        }
        let parsed = crate::nomo_mark::parse::fixture(&text);
        assert_eq!(parsed.details.len(), 12);
        assert_eq!(
            parsed
                .inline
                .iter()
                .filter(|span| span.format == crate::nomo_mark::parse::InlineFormat::Strong)
                .count(),
            1
        );
        assert_eq!(&*parsed.source, text);
        for block in &parsed.details {
            assert!(text[block.summary.clone()].starts_with("<summary>"));
            assert!(block.default_open);
        }
    }
    #[test]
    fn front_matter_is_recognized_only_at_document_start() {
        let source = "---\ntitle: test\n---\n\n# body\n";
        let input = crate::nomo_mark::protocol::DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "d".into(),
            revision: 0,
            from_byte: 0,
            to_byte: source.len() as u64,
            text: source.into(),
        };
        let parsed = crate::nomo_mark::parse::ParsedWindow::parse(
            input,
            "w",
            0,
            crate::nomo_mark::protocol::WindowContext::Complete,
        )
        .unwrap();
        assert!(matches!(parsed.events[1].0, Event::Html(_)));
        assert_eq!(parsed.events[1].1, 0..20);
        assert!(
            parsed
                .events
                .iter()
                .any(|(event, _)| matches!(event, Event::Start(Tag::Heading { .. })))
        );
    }
}
