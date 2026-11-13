use super::{MathProjection, project_math_setext_with_links};
use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, Tag};
use std::{borrow::Cow, ops::Range};

struct DelimitedMath {
    range: Range<usize>,
    display: bool,
}

pub(super) fn project<'a>(
    source: &'a str,
    options: Options,
    resolves_link: impl Fn(&str, LinkType) -> bool,
) -> MathProjection<'a> {
    let original = || project_math_setext_with_links(source, options, &resolves_link);
    if !options.contains(Options::ENABLE_MATH)
        || (!source.contains(r"\(") && !source.contains(r"\["))
    {
        return original();
    }

    // Already parsed code, HTML and dollar math keep their original meaning.
    let base = original();
    let parser = Parser::new_with_broken_link_callback(
        base.as_ref(),
        options,
        Some(|link: BrokenLink<'_>| {
            resolves_link(&link.reference, link.link_type).then_some(("".into(), "".into()))
        }),
    );
    let mut protected = Vec::new();
    let mut text = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Code(_)
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) => protected.push(range),
            // In shortcut/collapsed links the visible label is also the lookup key.
            Event::Start(
                Tag::Link {
                    link_type:
                        LinkType::Shortcut
                        | LinkType::ShortcutUnknown
                        | LinkType::Collapsed
                        | LinkType::CollapsedUnknown,
                    ..
                }
                | Tag::Image {
                    link_type:
                        LinkType::Shortcut
                        | LinkType::ShortcutUnknown
                        | LinkType::Collapsed
                        | LinkType::CollapsedUnknown,
                    ..
                },
            ) => protected.push(range),
            Event::Text(_) => text.push(range),
            _ => {}
        }
    }
    let mut candidates = Vec::new();
    let mut opening = None;
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut protected_at = 0;
    let mut text_at = 0;
    while at + 1 < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        // Consume escaped backslashes together, so \\( and \\[ remain literal.
        let next = bytes[at + 1];
        if matches!(next, b'(' | b'[' | b')' | b']') {
            while protected_at < protected.len() && protected[protected_at].end <= at {
                protected_at += 1;
            }
            while text_at < text.len() && text[text_at].end <= at + 1 {
                text_at += 1;
            }
            // Escaped punctuation is a text event; URLs, titles and reference
            // definitions are not. They must not become math after projection.
            let in_text = text
                .get(text_at)
                .is_some_and(|range| range.contains(&(at + 1)))
                && !protected
                    .get(protected_at)
                    .is_some_and(|range| range.start <= at && at < range.end);
            if in_text {
                if matches!(next, b'(' | b'[') {
                    opening = Some((at, next == b'['));
                } else if let Some((start, display)) = opening
                    && next == if display { b']' } else { b')' }
                {
                    let body = &source[start + 2..at];
                    if display || (!body.is_empty() && !body.contains(['\r', '\n'])) {
                        candidates.push(DelimitedMath {
                            range: start..at + 2,
                            display,
                        });
                    }
                    opening = None;
                }
            }
        }
        at += 2;
    }
    if candidates.is_empty() {
        return base;
    }

    let mut unused = (1u8..=8).map(char::from).filter(|ch| !source.contains(*ch));
    let (Some(padding), Some(dollar)) = (unused.next(), unused.next()) else {
        return base;
    };
    let mut projected = source.as_bytes().to_vec();
    for candidate in &candidates {
        let Range { start, end } = candidate.range;
        projected[start..start + 2].copy_from_slice(&if candidate.display {
            *b"$$"
        } else {
            [b'$', padding as u8]
        });
        projected[end - 2..end].copy_from_slice(&if candidate.display {
            *b"$$"
        } else {
            [padding as u8, b'$']
        });
        // A dollar inside \(...\) or \[...\] is TeX content, not a closing delimiter.
        for byte in &mut projected[start + 2..end - 2] {
            if *byte == b'$' {
                *byte = dollar as u8;
            }
        }
    }
    let mut projected = String::from_utf8(projected).expect("ASCII-only math projection");
    loop {
        let math = project_math_setext_with_links(&projected, options, &resolves_link);
        let parser = Parser::new_with_broken_link_callback(
            math.as_ref(),
            options,
            Some(|link: BrokenLink<'_>| {
                resolves_link(&link.reference, link.link_type).then_some(("".into(), "".into()))
            }),
        );
        let mut spans = parser
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::InlineMath(_) => Some((range, false)),
                Event::DisplayMath(_) => Some((range, true)),
                _ => None,
            })
            .peekable();
        let mut rejected = Vec::new();
        candidates.retain(|candidate| {
            while spans
                .peek()
                .is_some_and(|(range, _)| range.start < candidate.range.start)
            {
                spans.next();
            }
            let accepted = spans.peek().is_some_and(|(range, display)| {
                *range == candidate.range && *display == candidate.display
            });
            if !accepted {
                rejected.push(candidate.range.clone());
            }
            accepted
        });
        drop(spans);
        if candidates.is_empty() {
            return base;
        }
        if rejected.is_empty() {
            let marker = math.marker;
            let source = match math.source {
                Cow::Owned(text) => text,
                Cow::Borrowed(_) => projected,
            };
            return MathProjection {
                source: Cow::Owned(source),
                marker,
                delimiters: Some((padding, dollar)),
            };
        }
        drop(math);
        // Rolling back a rejected pair can change a link or container boundary.
        for range in rejected {
            projected.replace_range(range.clone(), &source[range]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::project_math;
    use super::*;

    fn parse(source: &str) -> Vec<(Event<'static>, Range<usize>)> {
        let math = project_math(source, Options::ENABLE_MATH | Options::ENABLE_TABLES);
        assert_eq!(math.as_ref().len(), source.len());
        Parser::new_ext(math.as_ref(), Options::ENABLE_MATH | Options::ENABLE_TABLES)
            .into_offset_iter()
            .map(|(event, range)| (math.restore(event).into_static(), range))
            .collect()
    }

    #[test]
    fn latex_delimiters_preserve_tex_and_byte_ranges() {
        let source = r"中文 \(x_1+\text{$5}\) 和 \[\frac{a}{b}\]。";
        let events = parse(source);
        let math: Vec<_> = events
            .iter()
            .filter_map(|(event, range)| match event {
                Event::InlineMath(tex) | Event::DisplayMath(tex) => {
                    Some((tex.as_ref(), &source[range.clone()]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            math,
            vec![
                (r"x_1+\text{$5}", r"\(x_1+\text{$5}\)"),
                (r"\frac{a}{b}", r"\[\frac{a}{b}\]")
            ]
        );
        assert!(
            events
                .iter()
                .any(|(event, _)| matches!(event, Event::InlineMath(_)))
        );
        assert!(
            events
                .iter()
                .any(|(event, _)| matches!(event, Event::DisplayMath(_)))
        );
    }

    #[test]
    fn latex_display_math_supports_containers_and_setext_equals() {
        for newline in ["\n", "\r\n", "\r"] {
            for (prefix, indent) in [("", ""), ("> ", "> "), ("- ", "  ")] {
                let source = format!(
                    "{prefix}\\[{newline}{indent}x{newline}{indent}={newline}{indent}y{newline}{indent}\\]{newline}"
                );
                let events = parse(&source);
                let tex: Vec<_> = events
                    .iter()
                    .filter_map(|(event, _)| match event {
                        Event::DisplayMath(tex) => Some(tex.as_ref()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(tex.len(), 1, "{source:?}");
                assert!(tex[0].contains('='));
                assert!(
                    !events
                        .iter()
                        .any(|(event, _)| matches!(event, Event::Start(Tag::Heading { .. })))
                );
            }
        }
    }

    #[test]
    fn latex_delimiters_leave_code_html_escapes_and_incomplete_input_unchanged() {
        for source in [
            "\u{60}\\(x\\)\u{60} and \u{60}\\[y\\]\u{60}",
            "\u{60}\u{60}\u{60}\n\\[x\\]\n\u{60}\u{60}\u{60}\n",
            "    \\[x\\]\n",
            "<pre>\n\\[x\\]\n</pre>\n",
            r#"<span title="\(x\)">text</span>"#,
            r"$\text{\(x\)}$ and $$\text{\[x\]}$$",
            r"\\(x\\) and \\[y\\]",
            r"\(x",
            r"\[x",
            r"\(x\]",
            r"[text](https://example.com/\(x\))",
            "\\(x\n\ny\\)",
            "\\[x\n\ny\\]",
            "\\[x\n\n> y\\]",
            "[\\(x\\)]\n\n[\\(x\\)]: target\n",
            "[\\[x\\]][]\n\n[\\[x\\]]: target\n",
        ] {
            let expected: Vec<_> =
                Parser::new_ext(source, Options::ENABLE_MATH | Options::ENABLE_TABLES)
                    .into_offset_iter()
                    .map(|(event, range)| (event.into_static(), range))
                    .collect();
            assert_eq!(parse(source), expected, "{source:?}");
        }
    }

    #[test]
    fn latex_delimiters_mix_with_dollar_math_links_and_tables() {
        for source in [
            r"\(a\)\(b\)",
            r"$a$ \(b\) $$c$$ \[d\]",
            r"**\(a\)** and [\(b\)](target)",
            "| math |\n| --- |\n| \\(a\\) |\n",
            r"\\\(a\)",
            r"\(a\\)b\)",
        ] {
            assert!(
                parse(source)
                    .iter()
                    .any(|(event, _)| matches!(event, Event::InlineMath(_))),
                "{source:?}"
            );
        }
    }

    #[test]
    fn latex_delimiters_keep_the_no_math_fast_path() {
        for source in [
            "# Heading\ntext\n",
            "ordinary (x) and [y]",
            "$x$",
            r"\(unclosed",
        ] {
            assert!(matches!(
                project_math(source, Options::ENABLE_MATH).source,
                Cow::Borrowed(_)
            ));
        }
        let source = r"\(x\) \[y\]";
        assert_eq!(project_math(source, Options::empty()).as_ref(), source);
    }
}
