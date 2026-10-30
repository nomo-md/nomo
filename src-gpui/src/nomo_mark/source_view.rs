//! 源码行与块上下文。只使用适配层已经解析的范围，不重新解析 Markdown。
use super::parse::ParsedWindow;
use crate::markdown::highlight::LineContext;
use pulldown_cmark::{CodeBlockKind, Event, Tag};
use std::ops::Range;

pub struct SourceLine<'a> {
    pub range: Range<usize>,
    pub context: LineContext,
    pub language: &'a str,
}
pub fn lines(parsed: &ParsedWindow) -> Vec<SourceLine<'_>> {
    let mut blocks = Vec::new();
    for (event, range) in &parsed.events {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => blocks.push((
                range.clone(),
                LineContext::Code,
                match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or(""),
                    _ => "",
                },
                matches!(kind, CodeBlockKind::Fenced(_)),
            )),
            Event::DisplayMath(_) => blocks.push((range.clone(), LineContext::Math, "", false)),
            _ => {}
        }
    }
    if parsed.identity.from_byte == 0 {
        if let Some(end) = super::nomo_syntax::front_matter_end(&parsed.source) {
            blocks.push((0..end, LineContext::FrontMatter, "", false));
        }
    }
    blocks.sort_by_key(|(range, ..)| range.start);
    let mut result = Vec::new();
    let mut at = 0;
    let mut block_index = 0;
    for raw in parsed.source.split('\n') {
        let text = raw.strip_suffix('\r').unwrap_or(raw);
        while blocks
            .get(block_index)
            .is_some_and(|(range, ..)| range.end <= at)
        {
            block_index += 1;
        }
        let (context, language) = blocks
            .get(block_index)
            .filter(|(range, ..)| range.start <= at && at < range.end)
            .map_or(
                (LineContext::Normal, ""),
                |(range, context, language, fenced)| {
                    let marker = *fenced
                        && (at == super::commands::line_start(&parsed.source, range.start)
                            || (at + raw.len() + 1 >= range.end
                                && (text.trim_start().starts_with("```")
                                    || text.trim_start().starts_with("~~~"))));
                    (
                        if marker {
                            LineContext::FenceMarker
                        } else {
                            *context
                        },
                        *language,
                    )
                },
            );
        result.push(SourceLine {
            range: at..at + text.len(),
            context,
            language,
        });
        at += raw.len() + 1;
    }
    result
}

/// 同一代码节点连续着色，围栏与相邻节点重置状态；仅持有当前加载窗口的样式。
pub fn line_spans(parsed: &ParsedWindow, lines: &[SourceLine<'_>], auxiliary: Option<&str>) -> Vec<crate::markdown::highlight::Spans> {
    use crate::markdown::highlight;
    let mut result = Vec::with_capacity(lines.len());
    let mut at = 0;
    while at < lines.len() {
        let line = &lines[at];
        if auxiliary.is_some() || line.context == LineContext::Code {
            let language = auxiliary.unwrap_or(line.language);
            let mut end = at + 1;
            while end < lines.len() && (auxiliary.is_some() || (lines[end].context == LineContext::Code && lines[end].language == language)) { end += 1; }
            let text = &parsed.source[line.range.start..lines[end - 1].range.end];
            let highlighted = highlight::code_block(text, language);
            result.extend(highlighted.iter().cloned());
            at = end;
        } else {
            result.push(highlight::markdown_line(&parsed.source[line.range.clone()], line.context, line.language));
            at += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fences_reset_lexical_state_without_changing_source_offsets() {
        use crate::layout::ColorRole;
        let parsed = super::super::parse::fixture("```rust\r\n/* 注释\r\n继续\r\n```\r\n\r\n```rust\r\nlet x = 1;\r\n```\r\n");
        let source = lines(&parsed);
        let spans = line_spans(&parsed, &source, None);
        assert_eq!(source.len(), spans.len());
        assert_eq!(spans[2][0].1.color, ColorRole::Comment);
        assert!(spans[6].iter().any(|(range, style)| range == &(0..3) && style.color == ColorRole::Keyword));
        for (line, spans) in source.iter().zip(spans) {
            let text = &parsed.source[line.range.clone()];
            assert!(spans.iter().all(|(range, _)| text.is_char_boundary(range.start) && text.is_char_boundary(range.end)));
        }
    }

    #[test]
    fn mixed_endings_keep_original_offsets_and_code_context() {
        let parsed = super::super::parse::fixture("# 标题\r\n\r\n```rust\r\nlet x = 1;\n```\r\n\n");
        let lines = lines(&parsed);
        assert_eq!(lines[2].context, LineContext::FenceMarker);
        assert_eq!(lines[3].context, LineContext::Code);
        assert_eq!(lines[3].language, "rust");
        assert_eq!(&parsed.source[lines[3].range.clone()], "let x = 1;");
        assert!(lines.last().unwrap().range.is_empty());
        assert_eq!(lines.last().unwrap().range.end, parsed.source.len());
    }
}
