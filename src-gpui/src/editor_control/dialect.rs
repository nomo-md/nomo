//! 将原版 Nomo 的结构边界适配成有真实源码范围的事件，布局阶段无需再次解析。
use super::{nomo_syntax, protocol::ParseContext};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct DetailsBlock {
    pub source: Range<usize>,
    pub summary: Range<usize>,
    pub body: Range<usize>,
    pub default_open: bool,
}
pub fn events(
    source: &str,
    offset: usize,
    document_start: bool,
    options: Options,
    context: &ParseContext,
    details: &mut Vec<DetailsBlock>,
) -> Vec<(Event<'static>, Range<usize>)> {
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
    let mut resolve = |link: pulldown_cmark::BrokenLink<'_>| {
        context
            .references
            .get(&link.reference.to_lowercase())
            .map(|target| {
                (
                    target.destination.clone().into(),
                    target.title.clone().into(),
                )
            })
    };
    let parser =
        Parser::new_with_broken_link_callback(&source[from..], options, Some(&mut resolve));
    let mut skip: Option<Range<usize>> = None;
    for (event, range) in parser.into_offset_iter() {
        let range = from + range.start..from + range.end;
        if skip
            .as_ref()
            .is_some_and(|skip| skip.start <= range.start && range.start < skip.end)
        {
            continue;
        }
        if matches!(event, Event::Start(Tag::HtmlBlock)) {
            if let Some(block) = nomo_syntax::details(source, range.start) {
                if let Some(opening) = nomo_syntax::html_tag(source, range.start) {
                    let mut child_context = context.clone();
                    for (label, target) in parser_definitions(&source[from..], options) {
                        child_context.references.insert(label, target);
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
    result
}
fn parser_definitions(
    source: &str,
    options: Options,
) -> Vec<(String, super::protocol::ReferenceTarget)> {
    let parser = Parser::new_ext(source, options);
    parser
        .reference_definitions()
        .iter()
        .map(|(label, target)| {
            (
                label.to_lowercase(),
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
        let parsed = crate::editor_control::parse::fixture(&text);
        assert_eq!(parsed.details.len(), 12);
        assert_eq!(
            parsed
                .inline
                .iter()
                .filter(|span| span.format == crate::editor_control::parse::InlineFormat::Strong)
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
        let input = crate::editor_control::protocol::DocumentWindow {
            inline_templates: Vec::new(),
            document_id: "d".into(),
            revision: 0,
            from_byte: 0,
            to_byte: source.len() as u64,
            text: source.into(),
        };
        let parsed = crate::editor_control::parse::ParsedWindow::parse(
            input,
            "w",
            0,
            crate::editor_control::protocol::WindowContext::Complete,
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
