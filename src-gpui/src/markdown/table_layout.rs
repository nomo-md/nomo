//! 语义内容的固有宽度参与自动分配；图片保留名义宽度并适配最终单元格。
use super::{InlineAsset, InlineObject, InlineStyle, MapSeg, OBJECT_CHAR, ParaSpec, RenderEnv, TextLayout};
use gpui::Window;
use std::{ops::Range, rc::Rc};

pub(super) struct ImageMetric {
    pub text: Range<usize>,
    pub width: Option<f32>,
    pub align: Option<pulldown_cmark::Alignment>,
}

pub(super) fn header_style(mut style: InlineStyle, header: bool) -> InlineStyle {
    if header {
        style.bold = true;
        style.extra_bold = true;
        if style.color == super::ColorRole::Fg {
            style.color = super::ColorRole::Heading;
        }
    }
    style
}

/// 保留单行基线，只将额外的内容高度向上对齐到设备像素。
/// 一行折返或收回时，下面的行整像素平移，边框之间的像素数保持一致。
pub(super) fn row_height(single_line: f32, measured: f32, scale: f32) -> f32 {
    single_line + ((measured - single_line).max(0.0) * scale).ceil() / scale
}

pub(super) fn measure(
    spec: &ParaSpec,
    map: &[MapSeg],
    revealed: &[Range<usize>],
    images: &[ImageMetric],
    env: &RenderEnv,
    window: &Window,
    widths: &mut crate::nomo_mark::table_widths::TableWidths,
) -> f32 {
    let natural = |spec: &ParaSpec, widths: &mut crate::nomo_mark::table_widths::TableWidths| {
        widths.measure(spec.cache_key(f32::MAX), || env.layout(spec, f32::MAX, window).max_line_width())
    };
    let mut replacements: Vec<_> = map.iter()
        .filter(|segment| !segment.text.is_empty()
            && super::is_revealed(revealed, segment.source..segment.source_end))
        .map(|segment| (segment.text.clone(), None))
        .collect();
    replacements.extend(images.iter().filter(|image| !image.text.is_empty())
        .map(|image| (image.text.clone(), Some(image.width.unwrap_or(12.0 * spec.font_size)))));
    if replacements.is_empty() {
        return natural(spec, widths);
    }
    replacements.sort_unstable_by_key(|(range, _)| (range.start, range.end));
    let mut semantic = ParaSpec {
        text: String::with_capacity(spec.text.len()),
        spans: Vec::with_capacity(spec.spans.len()),
        objects: Vec::with_capacity(spec.objects.len()),
        base: spec.base.clone(),
        family: spec.family,
        font_size: spec.font_size,
        line_height: spec.line_height,
        wrap: spec.wrap,
    };
    let mut cursor = 0;
    for (range, image_width) in replacements {
        if range.start < cursor { continue; }
        append_semantic(spec, &mut semantic, cursor..range.start);
        if let Some(width) = image_width {
            let at = semantic.text.len();
            semantic.text.push(OBJECT_CHAR);
            // 未解码或失败的图片也按同一个名义宽度测量，避免替代文字撑宽列。
            // 此对象只参与字体测量，不绘制、不查询资源，也不进入显示列表。
            semantic.objects.push(InlineObject {
                at, width, width_percent: None, ascent: 0.0, descent: 0.0,
                key: InlineAsset::Image(Default::default()),
            });
        }
        cursor = range.end;
    }
    append_semantic(spec, &mut semantic, cursor..spec.text.len());
    natural(&semantic, widths)
}

fn append_semantic(spec: &ParaSpec, semantic: &mut ParaSpec, range: Range<usize>) {
    if range.is_empty() { return; }
    let start = semantic.text.len();
    semantic.text.push_str(&spec.text[range.clone()]);
    let first = spec.spans.partition_point(|(span, _)| span.end <= range.start);
    for (span, style) in spec.spans[first..].iter().take_while(|(span, _)| span.start < range.end) {
        semantic.spans.push((
            start + span.start.max(range.start) - range.start
                ..start + span.end.min(range.end) - range.start,
            style.clone(),
        ));
    }
    let first = spec.objects.partition_point(|object| object.at < range.start);
    for object in spec.objects[first..].iter().take_while(|object| object.at < range.end) {
        let mut object = object.clone();
        object.at = start + object.at - range.start;
        semantic.objects.push(object);
    }
}

pub(super) fn layout(spec: &ParaSpec, width: f32, env: &RenderEnv, window: &Window) -> Rc<TextLayout> {
    layout_with_image_width(spec, width, width, env, window)
}

/// 正文悬挂留白只影响断行，图片百分比和缩放仍以未扣留白的内容区域为准。
pub(super) fn layout_with_image_width(spec: &ParaSpec, width: f32, image_width: f32, env: &RenderEnv, window: &Window) -> Rc<TextLayout> {
    // 公式异步就绪时列宽可能已固定；等比适配该格，防止绘入相邻列。
    let needs_fit = |object: &InlineObject| matches!(object.key, InlineAsset::Image(_))
        || (spec.wrap == super::Wrap::TableCell && object.width > width
            && matches!(object.key, InlineAsset::Embed(_) | InlineAsset::MathInput));
    if !spec.objects.iter().any(needs_fit) {
        return env.layout(spec, width, window);
    }
    let mut fitted = spec.clone();
    for object in &mut fitted.objects {
        if !needs_fit(object) { continue; }
        let next = object.width_percent.take().map_or(object.width, |percent| image_width * percent / 100.0)
            .min(image_width);
        let scale = next / object.width.max(f32::EPSILON);
        object.width = next;
        if !matches!(object.key, InlineAsset::MathInput) {
            object.ascent *= scale;
            object.descent *= scale;
        }
    }
    env.layout(&fitted, width, window)
}
