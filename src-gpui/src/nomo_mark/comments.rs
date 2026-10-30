//! 行内注释的原版标签、局部源码边界与显示尺寸；不保存文档或编辑历史。
use super::{html_table::InlineAtom, parse::ParsedWindow};
use crate::{layout::{InlineAsset, InlineObject}, theme::{Family, Theme}};
use gpui::{Bounds, ContentMask, Hsla, Pixels, Window, point, px, size};
use std::ops::Range;

pub(crate) fn content(raw: &str) -> Option<String> {
    let inner = raw.trim().strip_prefix("<!--")?.strip_suffix("-->")?;
    if inner.trim().eq_ignore_ascii_case("toc") || inner.trim().eq_ignore_ascii_case("/toc") { return None; }
    let inner = inner.replace("\r\n", "\n");
    let body = if inner.starts_with('\n') && inner.ends_with('\n') && inner.len() >= 2 {
        &inner[1..inner.len() - 1]
    } else {
        let body = inner.strip_prefix([' ', '\t']).unwrap_or(&inner);
        body.strip_suffix([' ', '\t']).unwrap_or(body)
    };
    Some(body.into())
}

pub(crate) fn at(parsed: &ParsedWindow, at: usize) -> Option<(Range<usize>, String)> {
    if let Some(run) = super::table_inline::atom_at(parsed, at) {
        if let Some(InlineAtom::Comment(text)) = run.atom { return Some((run.source, text)); }
    }
    parsed.events.iter().find_map(|(event, range)| {
        if matches!(event, pulldown_cmark::Event::InlineHtml(_)) && range.start <= at {
            if let Some(run) = span_at(&parsed.source, range.start).filter(|run| run.source.contains(&at)) {
                if let Some(InlineAtom::Comment(text)) = run.atom { return Some((run.source, text)); }
            }
        }
        if !range.contains(&at) { return None; }
        let raw = match event {
            pulldown_cmark::Event::InlineHtml(raw) => raw,
            pulldown_cmark::Event::Html(raw) if !raw.trim().contains('\n') => raw,
            _ => return None,
        };
        let text = content(raw)?;
        // 块事件末尾的换行不属于标签，退出时光标仍留在同一行。
        let source = &parsed.source[range.clone()];
        let start = range.start + source.len() - source.trim_start().len();
        let end = start + source.trim().len();
        (start <= at && at < end).then_some((start..end, text))
    })
}

pub(crate) fn span_at(source: &str, at: usize) -> Option<super::html_table::InlineRun> {
    if !source.get(at..at + 5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("<span")) { return None; }
    super::html_table::atom_at(source, at).filter(|run| matches!(run.atom, Some(InlineAtom::Comment(_))))
}

/// 多行内容用原版已有的行内 HTML 形式，避免重新解析后变成块注释。
pub(crate) fn html(text: &str) -> String {
    let html = InlineAtom::Comment(String::new()).with_text(text).unwrap();
    super::table_inline::escape_html_in_markdown(&html)
}

pub(crate) fn preview(content: &str, empty: &str) -> String {
    let content = content.trim();
    if content.is_empty() { return empty.into(); }
    let mut chars = content.chars();
    let mut text: String = chars.by_ref().take(12).collect();
    if chars.next().is_some() { text.push('…'); }
    text.replace(['\r', '\n', '\t'], " ")
}

fn text_layout(text: &str, font_size: f32, window: &Window) -> crate::layout::TextLayout {
    let spec = crate::layout::ParaSpec::plain(text.into(), crate::layout::InlineStyle { color: crate::layout::ColorRole::Success, ..Default::default() },
        Family::Mono, font_size, font_size * 1.15, crate::layout::Wrap::Greedy);
    crate::layout::layout_text(&spec, 1_000_000.0, window)
}

pub(crate) fn object(content: &str, empty: &str, editing: bool, font_size: f32, zoom: f32, window: &Window) -> InlineObject {
    let font_size = font_size * 0.9;
    let label = if editing { String::new() } else { preview(content, empty) };
    let first = if editing { content.split('\n').next().unwrap_or("").trim_end_matches('\r') } else { &label };
    let measured = text_layout(first, font_size, window);
    let mut width = measured.max_line_width();
    let mut height = measured.height();
    if editing {
        for text in content.split('\n').skip(1) {
            let measured = text_layout(text.trim_end_matches('\r'), font_size, window);
            width = width.max(measured.max_line_width());
            height += measured.height();
        }
        width += 6.0 * zoom;
    }
    let viewport = f32::from(window.viewport_size().width);
    let max_width = if editing { (640.0 * zoom).min(font_size * 0.6 * 64.0).min(viewport * 0.84) }
        else { (font_size * 0.6 * 34.0).min(viewport * 0.72) };
    let inner_width = width.max(if editing { 16.0 * zoom } else { 0.0 }).min(max_width.max(16.0 * zoom));
    let ascent = measured.first_baseline() + 4.0 * zoom;
    InlineObject { at: 0, width: inner_width + 20.0 * zoom, width_percent: None,
        ascent, descent: (height + 8.0 * zoom - ascent).max(0.0),
        key: InlineAsset::Comment { label, font_size: font_size.to_bits(), zoom: zoom.to_bits(), editing } }
}

pub(crate) fn colors(theme: &Theme) -> (Hsla, Hsla) {
    // 与 color-mix(in srgb, success 12%/42%, surface/code-border) 一致。
    (crate::ui::motion::color(theme.surface, theme.success, 0.12),
        crate::ui::motion::color(theme.code_border, theme.success, 0.42))
}

pub(crate) fn paint(key: &InlineAsset, bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window) {
    let InlineAsset::Comment { label, font_size, zoom, editing } = key else { return; };
    let zoom = f32::from_bits(*zoom);
    let font_size = f32::from_bits(*font_size);
    let (background, border) = colors(theme);
    let chip = Bounds::new(bounds.origin + point(px(2.0 * zoom), px(0.0)), size(bounds.size.width - px(4.0 * zoom), bounds.size.height));
    crate::layout::paint_box(chip, background, 4.0 * zoom, Some((border, zoom)), window);
    if *editing { return; }
    let text_bounds = Bounds::new(chip.origin + point(px(8.0 * zoom), px(4.0 * zoom)),
        size((chip.size.width - px(16.0 * zoom)).max(px(0.0)), chip.size.height - px(8.0 * zoom)));
    window.with_content_mask(Some(ContentMask { bounds: text_bounds }), |window| {
        text_layout(label, font_size, window).paint(text_bounds.origin, 0.0..f32::from(text_bounds.size.height), theme, window);
    });
}
