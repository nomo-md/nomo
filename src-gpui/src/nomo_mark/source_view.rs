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

#[cfg(test)]
mod tests {
    use super::*;
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
