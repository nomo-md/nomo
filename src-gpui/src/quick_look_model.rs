//! 有界的 Markdown 事件流，供 AppKit 排版；增强渲染失败保留可读源码。
use super::native;
use base64::Engine;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use std::time::{Duration, Instant};

pub const MAX_INPUT: usize = 1024 * 1024;
const MAX_EVENTS: usize = 30_000;
const MAX_TEXT: usize = 20 * 1024 * 1024;
const MAX_PIXELS: usize = 8 * 1024 * 1024; // 32 MiB 解码像素，总量按请求计算。

#[derive(Serialize)]
pub struct Token {
    kind: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<[u8; 3]>,
}
impl Token {
    fn new(kind: &'static str, text: impl Into<String>) -> Self { Self { kind, text: text.into(), value: None, color: None } }
    fn number(kind: &'static str, value: f64) -> Self { Self { value: Some(value), ..Self::new(kind, "") } }
}
struct Budget<F> { started: Instant, cancelled: F, events: Vec<Token>, bytes: usize, pixels: usize, enhancements: usize }
impl<F: Fn() -> bool> Budget<F> {
    fn check(&self) -> Result<(), String> {
        if (self.cancelled)() { return Err("Preview cancelled.".into()); }
        if self.started.elapsed() > Duration::from_secs(8) { return Err("Preview exceeded its time budget.".into()); }
        Ok(())
    }
    fn push(&mut self, token: Token) -> Result<(), String> {
        self.check()?;
        self.bytes += token.text.len();
        if self.events.len() >= MAX_EVENTS || self.bytes > MAX_TEXT { return Err("Preview exceeds its content budget.".into()); }
        self.events.push(token); Ok(())
    }
    fn image(&mut self, source: &str, display: bool, mermaid: bool, dark: bool) -> Result<Token, String> {
        self.enhancements += 1;
        if self.enhancements > 64 || source.len() > 8192 { return Err("enhancement budget".into()); }
        self.check()?;
        let (svg, depth) = if mermaid {
            // 首次字体加载属于预览总预算，不应耗尽后续图表计算的 150 ms。
            let _ = native::mermaid_fonts();
            self.check()?;
            let palette = dark.then_some(native::MermaidPalette { mode: 2,
                colors: [0x202020, 0x303030, 0xe6e6e6, 0x888888, 0xc0c0c0, 0x262626, 0x303030, 0x7dbbff, 0xffffff] });
            (native::mermaid_svg_with_palette(source, dark, palette,
                merman::OperationControl::new().with_deadline(Duration::from_millis(150)))?, 0.0)
        }
            else { native::math_svg(source, display, if dark { [225, 225, 230] } else { [30, 30, 35] })? };
        if svg.len() > 2 * 1024 * 1024 { return Err("SVG budget".into()); }
        let options = resvg::usvg::Options {
            fontdb: native::font_database(),
            // Mermaid 的 image 节点也不得绕过 Swift 的本地资源预算。
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
        let scale = (2048.0 / tree.size().width().max(tree.size().height())).min(2.0);
        let width = (tree.size().width() * scale).ceil() as u32;
        let height = (tree.size().height() * scale).ceil() as u32;
        let pixels = width as usize * height as usize;
        if self.pixels + pixels > MAX_PIXELS { return Err("pixel budget".into()); }
        self.check()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or("empty image")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let png = pixmap.encode_png().map_err(|e| e.to_string())?;
        self.pixels += pixels;
        Ok(Token { value: Some(f64::from(depth)), ..Token::new(if display { "block_raster" } else { "raster" }, base64::engine::general_purpose::STANDARD.encode(png)) })
    }
    fn code(&mut self, source: &str, language: &str, dark: bool) -> Result<(), String> {
        use two_face::re_exports::syntect::{easy::HighlightLines, util::LinesWithEndings};
        self.push(Token::new("code_start", language))?;
        // 语法库仅首次代码预览加载；不缓存正文、跨度或着色结果。
        static SYNTAXES: std::sync::OnceLock<two_face::re_exports::syntect::parsing::SyntaxSet> = std::sync::OnceLock::new();
        let syntaxes = SYNTAXES.get_or_init(two_face::syntax::extra_newlines);
        let syntax = syntaxes.find_syntax_by_token(language);
        if let Some(syntax) = syntax.filter(|_| source.len() <= 64 * 1024) {
            let themes = two_face::theme::extra();
            let theme = &themes[if dark { two_face::theme::EmbeddedThemeName::Dracula } else { two_face::theme::EmbeddedThemeName::InspiredGithub }];
            let mut highlighter = HighlightLines::new(syntax, theme);
            for line in LinesWithEndings::from(source) {
                self.check()?;
                if line.len() > 8192 { self.push(Token::new("text", line))?; continue; }
                match highlighter.highlight_line(line, syntaxes) {
                    Ok(spans) => for (style, text) in spans {
                        self.push(Token { color: Some([style.foreground.r, style.foreground.g, style.foreground.b]), ..Token::new("text", text) })?;
                    },
                    Err(_) => self.push(Token::new("text", line))?,
                }
            }
        } else { self.push(Token::new("text", source))?; }
        self.push(Token::new("code_end", ""))
    }
}

fn decode(bytes: &[u8]) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(rest.to_vec()).map_err(|_| "Invalid UTF-8 Markdown.".into());
    }
    let encoding = if bytes.starts_with(&[0xff, 0xfe]) { Some(encoding_rs::UTF_16LE) }
        else if bytes.starts_with(&[0xfe, 0xff]) { Some(encoding_rs::UTF_16BE) } else { None };
    if let Some(encoding) = encoding {
        let (text, errors) = encoding.decode_without_bom_handling(&bytes[2..]);
        return if errors { Err("Invalid UTF-16 Markdown.".into()) } else { Ok(text.into_owned()) };
    }
    if let Ok(text) = std::str::from_utf8(bytes) { return Ok(text.into()); }
    let (text, errors) = encoding_rs::GBK.decode_without_bom_handling(bytes);
    if errors { Err("Unsupported Markdown encoding.".into()) } else { Ok(text.into_owned()) }
}

pub fn preview(bytes: &[u8], dark: bool, cancelled: impl Fn() -> bool) -> Result<Vec<Token>, String> {
    if bytes.len() > MAX_INPUT { return Err("Quick Look supports Markdown files up to 1 MiB.".into()); }
    let mut b = Budget { started: Instant::now(), cancelled, events: Vec::new(), bytes: 0, pixels: 0, enhancements: 0 };
    b.check()?;
    let source = decode(bytes)?;
    if source.contains('\0') { return Err("The file contains binary data.".into()); }
    let mut markdown = source.as_str();
    // 文首 YAML 保留为独立元数据代码块，避免被解释成分隔线和 Setext 标题。
    if source.lines().next() == Some("---") {
        let first = source.find('\n').map_or(source.len(), |at| at + 1);
        let mut end = first;
        for line in source[first..].split_inclusive('\n') {
            end += line.len();
            if matches!(line.trim_end_matches(['\r', '\n']), "---" | "...") {
                b.code(&source[first..end - line.len()], "yaml", dark)?;
                markdown = &source[end..]; break;
            }
        }
    }
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES | Options::ENABLE_MATH | Options::ENABLE_GFM;
    let mut code: Option<(String, String)> = None;
    let mut image_depth = 0;
    let mut depth = 0usize;
    for event in Parser::new_ext(markdown, options) {
        b.check()?;
        if let Some((language, text)) = &mut code {
            match event {
                Event::End(TagEnd::CodeBlock) => {
                    if language.eq_ignore_ascii_case("mermaid") {
                        match b.image(text, true, true, dark) { Ok(token) => b.push(token)?, Err(_) => b.code(text, language, dark)? }
                    } else { b.code(text, language, dark)?; }
                    code = None; depth -= 1;
                }, Event::Text(value) => text.push_str(&value), _ => {}
            }
            continue;
        }
        if image_depth > 0 {
            match event { Event::Start(_) => image_depth += 1, Event::End(_) => { image_depth -= 1; if image_depth == 0 { b.push(Token::new("image_end", ""))?; } }, Event::Text(t) | Event::Code(t) => b.push(Token::new("image_alt", t))?, _ => {} }
            continue;
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > 64 { return Err("Markdown nesting exceeds the preview budget.".into()); }
                let token = match tag {
                    Tag::Paragraph => Token::new("paragraph", ""), Tag::Heading { level, .. } => Token::number("heading", level as u8 as f64),
                    Tag::BlockQuote(_) => Token::new("quote", ""), Tag::List(n) => Token::number("list", n.map_or(-1.0, |n| n as f64)),
                    Tag::Item => Token::new("item", ""), Tag::Emphasis => Token::new("emphasis", ""), Tag::Strong => Token::new("strong", ""),
                    Tag::Strikethrough => Token::new("strike", ""), Tag::Link { dest_url, .. } => Token::new("link", dest_url),
                    Tag::Image { dest_url, .. } => { image_depth = 1; depth -= 1; Token::new("image", dest_url) },
                    Tag::CodeBlock(kind) => { code = Some((match kind { CodeBlockKind::Fenced(s) => s.split_whitespace().next().unwrap_or("").into(), _ => String::new() }, String::new())); continue; },
                    Tag::Table(columns) => {
                        if columns.len() > 32 { return Err("Preview tables support at most 32 columns.".into()); }
                        Token::number("table", columns.len() as f64)
                    }, Tag::TableHead => Token::new("table_head", ""),
                    Tag::TableRow => Token::new("row", ""), Tag::TableCell => Token::new("cell", ""),
                    Tag::FootnoteDefinition(label) => Token::new("footnote", label),
                    _ => Token::new("group", ""),
                };
                b.push(token)?;
            },
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                b.push(Token::new(match tag {
                    TagEnd::Paragraph => "paragraph_end", TagEnd::Heading(_) => "heading_end", TagEnd::BlockQuote(_) => "quote_end",
                    TagEnd::List(_) => "list_end", TagEnd::Item => "item_end", TagEnd::Emphasis => "emphasis_end", TagEnd::Strong => "strong_end",
                    TagEnd::Strikethrough => "strike_end", TagEnd::Link => "link_end", TagEnd::Table => "table_end",
                    TagEnd::TableHead => "table_head_end", TagEnd::TableRow => "row_end", TagEnd::TableCell => "cell_end",
                    TagEnd::FootnoteDefinition => "footnote_end", _ => "group_end",
                }, ""))?;
            },
            Event::Text(t) => b.push(Token::new("text", t))?, Event::Code(t) => b.push(Token::new("inline_code", t))?,
            Event::Html(t) | Event::InlineHtml(t) => b.push(Token::new("literal", t))?,
            Event::SoftBreak => b.push(Token::new("text", "\n"))?, Event::HardBreak => b.push(Token::new("text", "\n"))?,
            Event::Rule => b.push(Token::new("rule", ""))?, Event::TaskListMarker(checked) => b.push(Token::new("text", if checked { "☑ " } else { "☐ " }))?,
            Event::FootnoteReference(t) => b.push(Token::new("footnote_ref", t))?,
            Event::InlineMath(ref t) | Event::DisplayMath(ref t) => {
                let display = matches!(event, Event::DisplayMath(_));
                match b.image(&t, display, false, dark) { Ok(token) => b.push(token)?, Err(_) => b.push(Token::new("inline_code", format!("${t}$")))? }
            },
        }
    }
    Ok(b.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn preserves_structure_unicode_and_safe_links() {
        let events = preview("# 中文\n\n- [x] **完成**\n\n| A | B |\n|---|---|\n|1|2|\n\n[link](javascript:alert)".as_bytes(), false, || false).unwrap();
        assert!(events.iter().any(|e| e.kind == "heading"));
        assert!(events.iter().any(|e| e.kind == "table" && e.value == Some(2.0)));
        assert!(events.iter().any(|e| e.text == "中文"));
        // URL 只作为数据；是否可点击由 AppKit 的显式协议白名单决定。
        assert!(events.iter().any(|e| e.kind == "link" && e.text == "javascript:alert"));
    }
    #[test] fn encoding_and_limits() {
        assert_eq!(decode(&[0xff, 0xfe, 0x2d, 0x4e]).unwrap(), "中");
        assert_eq!(decode(&[0xfe, 0xff, 0x4e, 0x2d]).unwrap(), "中");
        let (gbk, _, _) = encoding_rs::GBK.encode("中文"); assert_eq!(decode(&gbk).unwrap(), "中文");
        assert!(preview(b"text", false, || true).is_err());
        assert!(preview(&vec![b'a'; MAX_INPUT + 1], false, || false).is_err());
        assert!(preview(b"abc\0def", false, || false).is_err());
    }
    #[test] fn renders_math_and_mermaid_without_a_view() {
        let events = preview(b"$x^2$\n\n```mermaid\ngraph TD\nA-->B\n```", false, || false).unwrap();
        assert!(events.iter().any(|e| e.kind == "raster"));
        assert!(events.iter().any(|e| e.kind == "block_raster"));
    }
    #[test] fn unsupported_math_remains_readable() {
        let events = preview(b"$\\nomoUnknown{x}$", false, || false).unwrap();
        assert!(events.iter().any(|e| e.kind == "inline_code" && e.text.contains("nomoUnknown")));
    }
}
