use super::*;
use crate::nomo_mark::{clipboard_content, commands::{self, EditIntent}, protocol::CopyFormat};

fn toggle(parsed: &ParsedWindow, selection: DocumentSelection) -> (ParsedWindow, DocumentSelection) {
    let result = commands::apply_semantic(parsed, selection, EditIntent::ToggleInline(InlineFormat::Code)).unwrap();
    apply_result(parsed, result)
}

fn apply_result(parsed: &ParsedWindow, result: EditResult) -> (ParsedWindow, DocumentSelection) {
    let base = parsed.identity.from_byte;
    let mut source = parsed.source.to_string();
    for edit in result.edits.iter().rev() {
        source.replace_range((edit.from_byte - base) as usize..(edit.to_byte - base) as usize, &edit.inserted_text);
    }
    (crate::nomo_mark::parse::fixture(&source), result.selection)
}

#[test]
fn inline_code_toggle_source_mode_keeps_selected_source_literal() {
    let source = "`a`\\*x\\*";
    let parsed = crate::nomo_mark::parse::fixture(source);
    let selection = DocumentSelection { anchor_byte: 103, head_byte: 100 + source.len() as u64 };
    let result = commands::apply(&parsed, selection, EditIntent::ToggleInline(InlineFormat::Code)).unwrap();
    let (parsed, selection) = apply_result(&parsed, result);
    assert_eq!(visible(&parsed), "a\\*x\\*");
    assert_eq!(selected_text(&parsed, selection), "\\*x\\*");
}

#[test]
fn inline_code_toggle_keeps_external_format_and_separating_whitespace() {
    for source in ["`left` middle", "**middle**`right`", "`left`\r\n\r\nmiddle"] {
        let parsed = crate::nomo_mark::parse::fixture(source);
        let start = source.find("middle").unwrap() as u64 + 100;
        let selection = DocumentSelection { anchor_byte: start, head_byte: start + 6 };
        let expected = visible(&parsed);
        let (after, selection) = toggle(&parsed, selection);
        assert_eq!(visible(&after), expected);
        assert_eq!(selected_text(&after, selection), "middle");
        assert_eq!(after.inline.iter().filter(|span| span.format == InlineFormat::Code).count(), 2);
        if source.starts_with("**") { assert!(after.source.starts_with("**`middle`**")); }
        if source.contains("\r\n") { assert!(after.source.contains("\r\n\r\n")); }
    }
}

#[test]
fn inline_code_toggle_table_join_keeps_pipe_inside_cell() {
    for body in [r"a\|b", r"a\\\|b"] {
        let source = format!("| A | B |\n| --- | --- |\n| `{body}`tail | keep |\n");
        let parsed = crate::nomo_mark::parse::fixture(&source);
        let code = parsed.events.iter().find_map(|(event, _)| match event { Event::Code(text) => Some(text.to_string()), _ => None }).unwrap();
        let start = source.find("tail").unwrap() as u64 + 100;
        let selection = DocumentSelection { anchor_byte: start, head_byte: start + 4 };
        let (after, selection) = toggle(&parsed, selection);
        let after_code = after.events.iter().find_map(|(event, _)| match event { Event::Code(text) => Some(text.to_string()), _ => None }).unwrap();
        assert_eq!(after_code, format!("{code}tail"), "{}", after.source);
        assert_eq!(&after.source[(selection.range().start - 100) as usize..(selection.range().end - 100) as usize], "tail");
        assert_eq!(after.nodes.iter().filter(|node| node.kind == "table-cell").count(), 4);
        assert!(after.source.ends_with(" | keep |\n"));
    }
}

#[test]
fn inline_code_toggle_preserves_code_padding_and_literal_ticks() {
    for source in [" hello ", "  hello  ", "   ", "tick`x", "`"] {
        let parsed = crate::nomo_mark::parse::fixture(source);
        let selection = DocumentSelection { anchor_byte: 100, head_byte: 100 + source.len() as u64 };
        // 源码模式把选中的字符作为代码正文；语法填充不吞掉选中的空格。
        let result = commands::apply(&parsed, selection, EditIntent::ToggleInline(InlineFormat::Code)).unwrap();
        let (after, _) = apply_result(&parsed, result);
        assert_eq!(visible(&after), source);
    }
}

fn selected_text(parsed: &ParsedWindow, selection: DocumentSelection) -> String {
    let range = local_range(&parsed.source, parsed.identity.from_byte, selection.range()).unwrap();
    clipboard_content::selected(parsed, range, CopyFormat::Readable).unwrap().text
}

fn visible(parsed: &ParsedWindow) -> String {
    clipboard_content::selected(parsed, 0..parsed.source.len(), CopyFormat::Readable).unwrap().text
}

#[test]
fn inline_code_toggle_partial_roundtrip_preserves_selection_and_text() {
    for range in [3..7, 4..7, 4..8] {
        for backward in [false, true] {
            let source = "**`hello`**";
            let expected = &source[range.clone()];
            let mut parsed = crate::nomo_mark::parse::fixture(source);
            let (from, to) = (100 + range.start as u64, 100 + range.end as u64);
            let mut selection = if backward { DocumentSelection { anchor_byte: to, head_byte: from } }
                else { DocumentSelection { anchor_byte: from, head_byte: to } };
            for _ in 0..4 {
                (parsed, selection) = toggle(&parsed, selection);
                assert_eq!(selected_text(&parsed, selection), expected);
                assert_eq!(visible(&parsed), "hello");
                (parsed, selection) = toggle(&parsed, selection);
                assert_eq!(selected_text(&parsed, selection), expected, "{}", parsed.source);
                assert_eq!(visible(&parsed), "hello", "{}", parsed.source);
                assert_eq!(parsed.source.as_ref(), source);
                assert_eq!(selection.anchor_byte > selection.head_byte, backward);
            }
        }
    }
}

#[test]
fn inline_code_toggle_partial_literals_and_unicode_remain_visible() {
    for (source, chosen) in [
        ("**``中😀 *x* `y` tail``**", "*x* `y`"),
        ("**`(x) [y] &amp; tail`**", "(x) [y] &amp;"),
        ("**` hello `**", "hell"),
        ("**`  hello  `**", "hello"),
        ("**`a\r\n中😀z`**", "中😀"),
    ] {
        let mut parsed = crate::nomo_mark::parse::fixture(source);
        let expected = visible(&parsed);
        let start = source.find(chosen).unwrap() as u64 + 100;
        let mut selection = DocumentSelection { anchor_byte: start, head_byte: start + chosen.len() as u64 };
        for _ in 0..4 {
            (parsed, selection) = toggle(&parsed, selection);
            assert_eq!(selected_text(&parsed, selection), chosen, "{}", parsed.source);
            assert_eq!(visible(&parsed), expected);
            (parsed, selection) = toggle(&parsed, selection);
            assert_eq!(selected_text(&parsed, selection), chosen, "{}", parsed.source);
            assert_eq!(visible(&parsed), expected, "{}", parsed.source);
        }
    }
}
