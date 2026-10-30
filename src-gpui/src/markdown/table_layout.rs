//! 表格测量只使用语义内容；图片按旧版的测量宽度参与分配，再适配最终单元格。
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

pub(super) fn measure(
    spec: &ParaSpec,
    map: &[MapSeg],
    revealed: &[Range<usize>],
    images: &[ImageMetric],
    env: &RenderEnv,
    window: &Window,
) -> (f32, f32) {
    // 公式和注释标签作为完整行内对象；图片、代码和长链接均可适配列宽。
    let atomic = spec.objects.iter()
        .filter(|object| matches!(object.key, InlineAsset::Embed(_) | InlineAsset::Comment { .. }))
        .map(|object| object.width)
        .fold(0.0, f32::max);
    let mut replacements: Vec<_> = map.iter()
        .filter(|segment| !segment.text.is_empty()
            && revealed.contains(&(segment.source..segment.source_end)))
        .map(|segment| (segment.text.clone(), None))
        .collect();
    replacements.extend(images.iter().filter(|image| !image.text.is_empty())
        .map(|image| (image.text.clone(), Some(image.width.unwrap_or(12.0 * spec.font_size)))));
    if replacements.is_empty() {
        return (env.layout(spec, f32::MAX, window).max_line_width(), atomic);
    }
    replacements.sort_unstable_by_key(|(range, _)| (range.start, range.end));
    let mut semantic = ParaSpec {
        text: String::with_capacity(spec.text.len()),
        spans: Vec::with_capacity(spec.spans.len()),
        objects: Vec::with_capacity(spec.objects.len()),
        base: spec.base,
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
    (env.layout(&semantic, f32::MAX, window).max_line_width(), atomic)
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
            *style,
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
    if !spec.objects.iter().any(|object| matches!(object.key, InlineAsset::Image(_))) {
        return env.layout(spec, width, window);
    }
    let mut fitted = spec.clone();
    for object in &mut fitted.objects {
        if !matches!(object.key, InlineAsset::Image(_)) { continue; }
        let next = object.width_percent.take().map_or(object.width, |percent| width * percent / 100.0)
            .min(width);
        let scale = next / object.width.max(f32::EPSILON);
        object.width = next;
        object.ascent *= scale;
        object.descent *= scale;
    }
    env.layout(&fitted, width, window)
}
