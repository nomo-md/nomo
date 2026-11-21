use super::*;
use crate::nomo_mark::protocol::{DocumentWindow, WindowContext};

fn parse(text: &str) -> ParsedWindow {
    ParsedWindow::parse(DocumentWindow {
        document_id: "clipboard-test".into(), revision: 0, from_byte: 0,
        to_byte: text.len() as u64, text: text.into(), inline_templates: Vec::new(),
    }, "clipboard-test", 0, WindowContext::Complete).unwrap()
}

#[test]
fn clipboard_partial_nested_code_separates_text_and_formatting() {
    let parsed = parse("before **`hello`** after");
    let start = parsed.source.find("ell").unwrap();
    let payload = selected(&parsed, start..start + 3, CopyFormat::Readable).unwrap();
    assert_eq!(payload.text, "ell");
    assert_eq!(payload.source, "**`ell`**");
    assert_eq!(payload.html, "<p><strong><code>ell</code></strong></p>");
    assert_eq!(selected(&parsed, start..start + 3, CopyFormat::MarkdownOnly).unwrap().text, "**`ell`**");
    let rich = super::super::clipboard_fragment::encode(&payload.text, &payload.html, &payload.source).unwrap();
    assert_eq!(from_rich_clipboard(&payload.text, &rich), "**`ell`**");
}

#[test]
fn clipboard_partial_selection_omits_adjacent_hidden_markers() {
    for (source, end_marker, expected, expected_html) in [
        ("<strong>a</strong><strong>b*c</strong><strong>d*ef</strong>", "c", "<strong>b*c*</strong>", "<p><strong>b<em>c</em></strong></p>"),
        ("<strong>a</strong><em>bc</em><u>d</u>", "<u>", "<em>bc</em>", "<p><em>bc</em></p>"),
    ] {
        let parsed = parse(source);
        let start = source.find("</strong>").unwrap();
        let end = source.find(end_marker).unwrap() + end_marker.len();
        for format in [CopyFormat::Readable, CopyFormat::Markdown, CopyFormat::MarkdownOnly] {
            let payload = selected(&parsed, start..end, format).unwrap();
            assert_eq!(payload.text, if format == CopyFormat::Readable { "bc" } else { expected });
            if format != CopyFormat::MarkdownOnly {
                assert_eq!(payload.source, expected);
                assert_eq!(payload.html, expected_html);
                let rich = super::super::clipboard_fragment::encode(&payload.text, &payload.html, &payload.source).unwrap();
                assert_eq!(from_rich_clipboard(&payload.text, &rich), expected);
            }
        }
    }
}

#[test]
fn clipboard_default_format_retains_rich_data_and_explicit_copy_is_fixed() {
    let parsed = parse("before **`hello`** after");
    let start = parsed.source.find("ell").unwrap();
    for default_markdown in [false, true] {
        let format = CopyFormat::for_command(false, default_markdown);
        let ordinary = selected(&parsed, start..start + 3, format).unwrap();
        assert_eq!(ordinary.text, if default_markdown { "**`ell`**" } else { "ell" });
        assert_eq!(ordinary.html, "<p><strong><code>ell</code></strong></p>");
        assert_eq!(ordinary.source, "**`ell`**");
        let rich = super::super::clipboard_fragment::encode(&ordinary.text, &ordinary.html, &ordinary.source).unwrap();
        assert_eq!(from_rich_clipboard(&ordinary.text, &rich), "**`ell`**");

        let explicit = selected(&parsed, start..start + 3, CopyFormat::for_command(true, default_markdown)).unwrap();
        assert_eq!(explicit.text, "**`ell`**");
        assert!(explicit.html.is_empty());
        assert!(explicit.source.is_empty());
    }
}

#[test]
fn clipboard_code_symbols_and_unicode_remain_literal() {
    let parsed = parse("**`中😀 *x* #tag \\path`**");
    let start = parsed.source.find('😀').unwrap();
    let end = parsed.source.find("path").unwrap() + 4;
    let copied = selected(&parsed, start..end, CopyFormat::Readable).unwrap();
    assert_eq!(copied.text, "😀 *x* #tag \\path");
    assert!(copied.html.contains("<code>😀 *x* #tag \\path</code>"));
}

#[test]
fn clipboard_partial_code_with_literal_ticks_has_safe_delimiters() {
    let source = "**`` `x` tail ``**";
    let parsed = parse(source);
    let start = source.find("`x").unwrap();
    let copied = selected(&parsed, start..start + 3, CopyFormat::Readable).unwrap();
    assert_eq!(copied.text, "`x`");
    assert_eq!(copied.html, "<p><strong><code>`x`</code></strong></p>");
    let reparsed = parse(&copied.source);
    assert_eq!(selected(&reparsed, 0..copied.source.len(), CopyFormat::Readable).unwrap().text, copied.text);
}

#[test]
fn clipboard_inline_padding_is_not_visible_content() {
    for (source, expected) in [
        ("` hello `", "hello"),
        ("`  hello  `", " hello "),
        ("`   `", "   "),
        ("`中\r\n😀`", "中 😀"),
        ("`` `x` ``", "`x`"),
    ] {
        let parsed = parse(source);
        assert_eq!(selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap().text, expected, "{source:?}");
    }
}

#[test]
fn clipboard_repeated_rich_paste_does_not_accumulate_spaces() {
    let original = "**`hello`**";
    let mut source = original.to_string();
    for _ in 0..20 {
        let parsed = parse(&source);
        let payload = selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap();
        assert_eq!(payload.text, "hello");
        let html = super::super::clipboard_fragment::encode(&payload.text, &payload.html, &payload.source).unwrap();
        source = from_rich_clipboard(&payload.text, &html);
        assert_eq!(source, original);
    }
}

#[test]
fn clipboard_external_code_html_roundtrips_without_padding_growth() {
    for text in ["hello", " hello ", "   ", "`x`", "中😀", "a  b"] {
        let html_source = format!("<p><code>{}</code></p>", html_escape::encode_text(text));
        let mut source = from_html(&html_source);
        for _ in 0..10 {
            let parsed = parse(&source);
            let copied = selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap();
            assert_eq!(copied.text, text);
            let next = from_html(&copied.html);
            assert_eq!(next, source);
            source = next;
        }
    }
}

#[test]
fn clipboard_code_block_selection_preserves_indent_crlf_and_literals() {
    let source = "```rust\r\n  let x = \"*中*\";\r\n\tprint(x);\r\n```\r\n";
    let parsed = parse(source);
    let start = source.find("  let").unwrap();
    let end = source.find("\r\n```").unwrap();
    let payload = selected(&parsed, start..end, CopyFormat::Readable).unwrap();
    assert_eq!(payload.text, &source[start..end]);
    assert!(payload.html.starts_with("<pre><code>"));
    assert!(!payload.html.contains("<em>"));
}

#[test]
fn clipboard_nested_code_excludes_container_markers() {
    let source = "> ```text\r\n> alpha\r\n>   beta\r\n> ```";
    let parsed = parse(source);
    let start = source.find("alpha").unwrap();
    let end = source.find("beta").unwrap() + 4;
    let payload = selected(&parsed, start..end, CopyFormat::Readable).unwrap();
    assert_eq!(payload.text, "alpha\r\n  beta");
    assert!(!payload.source.contains("\r\n>"));
}

#[test]
fn clipboard_plain_projection_handles_headings_links_and_escapes() {
    let source = "# Title\n\n**bold** [site](https://example.com) \\*literal\\* &amp; `a*b`";
    let parsed = parse(source);
    let payload = selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap();
    assert_eq!(payload.text, "Title\nbold site *literal* & a*b");
    assert!(payload.html.contains("<a href=\"https://example.com\">site</a>"));
}

#[test]
fn clipboard_plain_paste_escapes_markdown_without_erasing_characters() {
    let text = "**bold** [a] `code`\nx";
    let source = literal_paragraphs(text);
    let parsed = parse(&source);
    assert_eq!(selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap().text, text);
}

#[test]
fn clipboard_invalid_internal_fragment_falls_back_to_html() {
    let html = "<p><code>hello</code></p><!--Nomo-Clipboard:2:unknown-->";
    assert_eq!(from_rich_clipboard("hello", html), "`hello`");
}

#[test]
fn clipboard_internal_math_keeps_original_delimiters() {
    for source in [r"\(x^2\)", "$x^2$", "\\[\nx^2\n\\]"] {
        let parsed = parse(source);
        let copied = selected(&parsed, 0..source.len(), CopyFormat::Readable).unwrap();
        let html = super::super::clipboard_fragment::encode(&copied.text, &copied.html, &copied.source).unwrap();
        assert_eq!(from_rich_clipboard(&copied.text, &html), source);
    }
}

#[test]
fn clipboard_semantic_selection_spans_loaded_groups() {
    use crate::nomo_mark::{native_port::NativeWindowPort, session_window::WindowRequest};
    let root = std::env::temp_dir().join(format!("nomo-semantic-clipboard-{}", std::process::id()));
    let source = (0..180).map(|i| format!("**段落{i}😀**\r\n\r\n")).collect::<String>();
    let port = NativeWindowPort::open(root.clone(), "semantic-clipboard".into(), None, Some(source.clone())).unwrap();
    let request = WindowRequest::default();
    let groups = port.window_groups(0, &[], &request).unwrap();
    assert!(groups.len() > 1);
    let ordinary = from_session(&port, 0, 0..source.len() as u64, CopyFormat::Readable, &request).unwrap();
    assert!(!ordinary.text.contains("**"));
    assert!(ordinary.text.contains("段落179😀"));
    assert_eq!(ordinary.source, source);
    let default_markdown = from_session(&port, 0, 0..source.len() as u64, CopyFormat::Markdown, &request).unwrap();
    assert_eq!(default_markdown.text, source);
    assert_eq!(default_markdown.source, source);
    assert_eq!(default_markdown.html, ordinary.html);
    let markdown = from_session(&port, 0, 0..source.len() as u64, CopyFormat::MarkdownOnly, &request).unwrap();
    assert_eq!(markdown.text, source);
    assert!(markdown.html.is_empty());
    let cancelled = WindowRequest::default();
    cancelled.cancel();
    assert!(from_session(&port, 0, 0..source.len() as u64, CopyFormat::Readable, &cancelled).is_err());
    port.close(true).unwrap();
    drop(port);
    std::fs::remove_dir_all(root).unwrap();
}
