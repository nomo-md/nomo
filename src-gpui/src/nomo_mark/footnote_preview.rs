//! 悬停时按锚点读取至多 8 KiB；提示销毁即取消，不缓存整篇脚注正文。
use super::{native_port::NativeWindowPort, protocol::ControlError, session_window::WindowRequest};
use gpui::{div, px, Context, IntoElement, ParentElement, Render, Styled, Window};
use crate::layout::{ColorRole, InlineStyle, ParaSpec, Wrap};

pub(crate) struct FootnotePreview {
    pub request: WindowRequest,
    pub text: String,
}
impl Drop for FootnotePreview {
    fn drop(&mut self) { self.request.cancel(); }
}
impl Render for FootnotePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<crate::theme::Theme>().clone();
        let layout = crate::layout::layout_text(&preview_spec(&self.text), 320.0, window);
        let height = layout.height().min(320.0);
        div().p(px(12.0)).w(px(344.0)).rounded(px(6.0)).border_1().border_color(theme.border)
            .bg(theme.surface).shadow_md().overflow_hidden()
            .child(gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
                layout.paint(bounds.origin, 0.0..height, &theme, window);
            }).w(px(320.0)).h(px(height)))
    }
}
pub(crate) fn read(port: &NativeWindowPort, revision: u64, label: &str, request: &WindowRequest) -> Result<String, ControlError> {
    let byte = port.resolve_anchor(revision, &format!("fn-{label}"), request)?;
    request.check()?;
    let end = (byte + 8192).min(port.status()?.byte_length);
    let input = port.read_window(revision, byte..end)?;
    request.check()?;
    let mut text = pulldown_cmark::Parser::new_ext(&input.text, pulldown_cmark::Options::ENABLE_FOOTNOTES).into_offset_iter()
        .find_map(|(event, range)| matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::FootnoteDefinition(ref id)) if id.as_ref() == label)
            .then(|| input.text[range].split_once("]:").map_or("", |(_, body)| body).trim_start().lines()
                .map(|line| line.strip_prefix("    ").or_else(|| line.strip_prefix('\t')).unwrap_or(line)).collect::<Vec<_>>().join("\n")))
        .unwrap_or_default();
    if text.trim().is_empty() { text = format!("脚注 {label} · 点击查看"); }
    if port.status()?.revision != revision { return Err(ControlError::Stale); }
    Ok(text)
}

fn preview_spec(markdown: &str) -> ParaSpec {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut spec = ParaSpec::plain(String::new(), InlineStyle::default(), crate::theme::Family::Text, 13.0, 21.0, Wrap::Greedy);
    let mut style = InlineStyle::default();
    let mut stack = Vec::new();
    let mut remaining = 360;
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_MATH) {
        let value = match event {
            Event::Start(tag) => {
                stack.push(style);
                match tag {
                    Tag::Strong | Tag::Heading { .. } => style.bold = true,
                    Tag::Emphasis => style.italic = true, Tag::Strikethrough => style.strike = true,
                    Tag::Link { .. } => { style.color = ColorRole::Link; style.underline = true; }
                    Tag::CodeBlock(_) => style.mono = true,
                    Tag::Item => { if !spec.text.is_empty() && !spec.text.ends_with('\n') { spec.text.push('\n'); } spec.text.push_str("• "); }
                    _ => {}
                }
                continue;
            }
            Event::End(tag) => {
                style = stack.pop().unwrap_or_default();
                if matches!(tag, TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock) && !spec.text.ends_with('\n') { spec.text.push('\n'); }
                continue;
            }
            Event::Text(value) | Event::InlineMath(value) | Event::DisplayMath(value) => value.into_string(),
            Event::Code(value) => {
                let code: String = value.chars().take(remaining).collect();
                remaining -= code.chars().count(); let start = spec.text.len(); spec.text.push_str(&code);
                spec.spans.push((start..spec.text.len(), InlineStyle { mono: true, code_bg: true, color: ColorRole::InlineCode, ..style }));
                if remaining == 0 { spec.text.push('…'); break; }
                continue;
            }
            Event::SoftBreak | Event::HardBreak => "\n".into(),
            _ => continue,
        };
        let value: String = value.chars().take(remaining).collect();
        remaining -= value.chars().count(); let start = spec.text.len(); spec.text.push_str(&value);
        spec.spans.push((start..spec.text.len(), style));
        if remaining == 0 { spec.text.push('…'); break; }
    }
    spec
}
