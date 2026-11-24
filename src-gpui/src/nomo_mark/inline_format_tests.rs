use super::*;
use crate::nomo_mark::{clipboard_content, commands::{self, EditIntent}, mapping::text_segments, protocol::CopyFormat};

const FORMATS: [InlineFormat; 7] = [InlineFormat::Strong, InlineFormat::Emphasis, InlineFormat::Strike,
    InlineFormat::Underline, InlineFormat::Highlight, InlineFormat::Superscript, InlineFormat::Subscript];

fn visible(parsed: &ParsedWindow, range: Range<usize>) -> String {
    clipboard_content::selected(parsed, range, CopyFormat::Readable).unwrap().text
}

fn appearance(parsed: &ParsedWindow) -> Vec<(char, [bool; 7])> {
    let mut result = Vec::new();
    for (event, range) in &parsed.events {
        let Event::Text(text) = event else { continue; };
        for segment in text_segments(text, &parsed.source[range.clone()], range.start) {
            for (offset, ch) in text[segment.display.clone()].char_indices() {
                let at = segment.source.start + if segment.exact { offset } else { 0 };
                result.push((ch, FORMATS.map(|format| parsed.inline.iter()
                    .any(|span| span.format == format && span.content.contains(&at)))));
            }
        }
    }
    result
}

fn toggle(parsed: &ParsedWindow, selection: DocumentSelection, format: InlineFormat) -> (ParsedWindow, DocumentSelection) {
    let result = commands::apply_semantic(parsed, selection, EditIntent::ToggleInline(format)).unwrap();
    let mut source = parsed.source.to_string();
    for edit in result.edits.iter().rev() {
        source.replace_range((edit.from_byte - 100) as usize..(edit.to_byte - 100) as usize, &edit.inserted_text);
    }
    (crate::nomo_mark::parse::fixture(&source), result.selection)
}

fn roundtrip(source: &str, chosen: &str, format: InlineFormat, backward: bool) {
    let mut parsed = crate::nomo_mark::parse::fixture(source);
    let original = appearance(&parsed);
    let whole = visible(&parsed, 0..source.len());
    let first = source.find(chosen).unwrap();
    let start = 100 + first as u64;
    let end = start + chosen.len() as u64;
    let mut selection = if backward { DocumentSelection { anchor_byte: end, head_byte: start } }
        else { DocumentSelection { anchor_byte: start, head_byte: end } };
    let selected_chars = visible(&parsed, 0..first).chars().count()..visible(&parsed, 0..first + chosen.len()).chars().count();
    let format_index = FORMATS.iter().position(|item| *item == format).unwrap();
    assert!(original[selected_chars.clone()].iter().all(|(_, formats)| formats[format_index]), "fixture {source}");
    for cycle in 0..3 {
        for removed in [true, false] {
            (parsed, selection) = toggle(&parsed, selection, format);
            let range = local_range(&parsed.source, 100, selection.range()).unwrap();
            assert_eq!(visible(&parsed, range), chosen, "selection: {source} -> {}, cycle {cycle}, removed {removed}", parsed.source);
            assert_eq!(visible(&parsed, 0..parsed.source.len()), whole, "visible: {source} -> {}", parsed.source);
            assert_eq!(selection.anchor_byte > selection.head_byte, backward);
            let mut expected = original.clone();
            if removed {
                for (_, formats) in &mut expected[selected_chars.clone()] { formats[format_index] = false; }
            }
            assert_eq!(appearance(&parsed), expected, "format: {source} -> {}, cycle {cycle}, removed {removed}", parsed.source);
        }
    }
}

macro_rules! format_roundtrip {
    ($name:ident, $format:expr) => {
        #[test]
        fn $name() {
            let format = $format;
            for pair in [format.markers(), format.html_markers()] {
                let source = format!("{}hello{}", pair.0, pair.1);
                for chosen in ["hell", "ell", "ello"] {
                    for backward in [false, true] { roundtrip(&source, chosen, format, backward); }
                }
            }
        }
    };
}

format_roundtrip!(partial_strong_roundtrip, InlineFormat::Strong);
format_roundtrip!(partial_emphasis_roundtrip, InlineFormat::Emphasis);
format_roundtrip!(partial_strike_roundtrip, InlineFormat::Strike);
format_roundtrip!(partial_underline_roundtrip, InlineFormat::Underline);
format_roundtrip!(partial_highlight_roundtrip, InlineFormat::Highlight);
format_roundtrip!(partial_superscript_roundtrip, InlineFormat::Superscript);
format_roundtrip!(partial_subscript_roundtrip, InlineFormat::Subscript);

#[test]
fn partial_nested_formats_roundtrip() {
    for format in FORMATS {
        for outer in FORMATS {
            if outer == format || opposite(format) == Some(outer) { continue; }
            let (open, close) = outer.html_markers();
            let (inner_open, inner_close) = format.markers();
            let source = format!("{open}{inner_open}hello{inner_close}{close}");
            for chosen in ["hell", "ell", "ello"] { roundtrip(&source, chosen, format, false); }
            let source = format!("{inner_open}{open}hello{close}{inner_close}");
            roundtrip(&source, "ell", format, true);
        }
    }
}

fn mixed_selection(source: &str, range: Range<usize>, format: InlineFormat) {
    let mut parsed = crate::nomo_mark::parse::fixture(source);
    let whole = visible(&parsed, 0..source.len());
    let chosen = visible(&parsed, range.clone());
    let selected_chars = visible(&parsed, 0..range.start).chars().count()
        ..visible(&parsed, 0..range.end).chars().count();
    let mut expected = appearance(&parsed);
    let mut selection = DocumentSelection { anchor_byte: 100 + range.end as u64, head_byte: 100 + range.start as u64 };
    let index = FORMATS.iter().position(|item| *item == format).unwrap();
    for cycle in 0..6 {
        let remove = expected[selected_chars.clone()].iter().all(|(_, formats)| formats[index]);
        for (_, formats) in &mut expected[selected_chars.clone()] {
            formats[index] = !remove;
            if !remove {
                if let Some(other) = opposite(format) { formats[FORMATS.iter().position(|item| *item == other).unwrap()] = false; }
            }
        }
        (parsed, selection) = toggle(&parsed, selection, format);
        let range = local_range(&parsed.source, 100, selection.range()).unwrap();
        assert_eq!(visible(&parsed, range), chosen, "selection: {source} -> {}, cycle {cycle}", parsed.source);
        assert_eq!(visible(&parsed, 0..parsed.source.len()), whole, "visible: {source} -> {}", parsed.source);
        assert_eq!(appearance(&parsed), expected, "format: {source} -> {}, cycle {cycle}", parsed.source);
        assert!(selection.anchor_byte > selection.head_byte);
    }
}

#[test]
fn partial_crossing_format_boundaries() {
    for (source, format) in [("**ab*cd*ef**", InlineFormat::Strong), ("*ab**cd**ef*", InlineFormat::Emphasis),
        ("~~ab*cd*ef~~", InlineFormat::Strike), ("<u>ab**cd**ef</u>", InlineFormat::Underline),
        ("***abcdef***", InlineFormat::Strong), ("***abcdef***", InlineFormat::Emphasis)] {
        for (from, to) in [('b', 'c'), ('d', 'e'), ('b', 'e')] {
            let start = source.find(from).unwrap();
            let end = source.find(to).unwrap() + 1;
            mixed_selection(source, start..end, format);
        }
    }
    for format in FORMATS {
        let (open, close) = format.html_markers();
        let inner = if format == InlineFormat::Emphasis { "**" } else { "*" };
        let source = format!("{open}ab{inner}cd{inner}ef{close}");
        for (from, to) in [(source.find("ab").unwrap() + 1, source.find("cd").unwrap() + 1),
            (source.find("cd").unwrap() + 1, source.find("ef").unwrap() + 1)] {
            mixed_selection(&source, from..to, format);
        }
    }
}

#[test]
fn partial_mixed_formatted_and_plain_ranges() {
    for format in FORMATS {
        let (open, close) = format.markers();
        let source = format!("{open}ab{close}cd{open}ef{close}");
        // The last occurrence avoids letters inside HTML tags such as <mark>.
        let from = source.find("ab").unwrap() + 1;
        let to = source.rfind("ef").unwrap() + 1;
        mixed_selection(&source, from..to, format);
    }
}

#[test]
fn partial_whitespace_unicode_and_alternate_markers() {
    for format in FORMATS {
        let (open, close) = format.html_markers();
        for (body, chosen) in [("a b c", " b "), ("hello.world", "."), ("a中😀e\u{301}z", "中😀e\u{301}")] {
            roundtrip(&format!("{open}{body}{close}"), chosen, format, false);
        }
    }
    for (source, format) in [("__hello__", InlineFormat::Strong), ("_hello_", InlineFormat::Emphasis),
        ("***hello***", InlineFormat::Strong)] {
        roundtrip(source, "ell", format, true);
    }
}
