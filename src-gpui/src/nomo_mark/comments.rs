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

fn text_layout(text: &str, font_size: f32, width: f32, window: &Window) -> crate::layout::TextLayout {
    let spec = crate::layout::ParaSpec::plain(text.into(), crate::layout::InlineStyle { color: crate::layout::ColorRole::AccentStrong, ..Default::default() },
        Family::Mono, font_size, font_size * 1.15, crate::layout::Wrap::Greedy);
    crate::layout::layout_text(&spec, width, window)
}

pub(crate) fn object(content: &str, empty: &str, editing: bool, font_size: f32, zoom: f32, window: &Window) -> InlineObject {
    let font_size = font_size * 0.9;
    let viewport = f32::from(window.viewport_size().width);
    let max_width = if editing { (640.0 * zoom).min(font_size * 0.6 * 64.0).min(viewport * 0.84) }
        else { (font_size * 0.6 * 34.0).min(viewport * 0.72) };
    let text = if !editing && content.trim().is_empty() { empty } else { content };
    measure(text, editing, font_size, zoom, max_width, window)
}

fn measure(text: &str, editing: bool, font_size: f32, zoom: f32, inner_limit: f32, window: &Window) -> InlineObject {
    let limit = inner_limit.max(1.0);
    let measured = text_layout(text, font_size, if editing { 1_000_000.0 } else { limit }, window);
    let width = if editing { (measured.max_line_width() + 6.0 * zoom).max(16.0 * zoom) } else { measured.max_line_width() };
    let height = measured.height();
    let ascent = measured.first_baseline() + 4.0 * zoom;
    InlineObject { at: 0, width: width.min(limit) + 20.0 * zoom, width_percent: None,
        ascent, descent: (height + 8.0 * zoom - ascent).max(0.0),
        key: InlineAsset::Comment { label: (!editing).then(|| crate::layout::CommentLabel(std::rc::Rc::new(measured))),
            font_size: font_size.to_bits(), zoom: zoom.to_bits() } }
}

pub(crate) fn fit(object: &mut InlineObject, width: f32, window: &Window) {
    let InlineAsset::Comment { label, font_size, zoom } = &object.key else { return; };
    if let Some(label) = label {
        let zoom = f32::from_bits(*zoom);
        *object = InlineObject { at: object.at, ..measure(&label.0.text, false, f32::from_bits(*font_size), zoom, width - 20.0 * zoom, window) };
    } else { object.width = object.width.min(width); }
}

pub(crate) fn colors(theme: &Theme) -> (Hsla, Hsla) {
    // 每帧从当前主题主色派生，展示态与编辑态共用背景和边框。
    (crate::ui::motion::color(theme.surface, theme.accent, 0.12),
        crate::ui::motion::color(theme.code_border, theme.accent, 0.42))
}

pub(crate) fn paint(key: &InlineAsset, bounds: Bounds<Pixels>, theme: &Theme, window: &mut Window) {
    let InlineAsset::Comment { label, zoom, .. } = key else { return; };
    let zoom = f32::from_bits(*zoom);
    let (background, border) = colors(theme);
    let chip = Bounds::new(bounds.origin + point(px(2.0 * zoom), px(0.0)), size(bounds.size.width - px(4.0 * zoom), bounds.size.height));
    crate::layout::paint_box(chip, background, 4.0 * zoom, Some((border, zoom)), window);
    let Some(label) = label else { return; };
    let text_bounds = Bounds::new(chip.origin + point(px(8.0 * zoom), px(4.0 * zoom)),
        size((chip.size.width - px(16.0 * zoom)).max(px(0.0)), chip.size.height - px(8.0 * zoom)));
    window.with_content_mask(Some(ContentMask { bounds: text_bounds }), |window| {
        let clip = window.content_mask().bounds;
        label.0.paint(text_bounds.origin, f32::from(clip.top() - text_bounds.top())..f32::from(clip.bottom() - text_bounds.top()), theme, window);
    });
}
