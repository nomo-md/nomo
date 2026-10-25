//! 多样式段落排版：整段一次塑形，按贪心或 Knuth–Plass 断行，绘制时逐字形定位。
//!
//! 段落内的颜色、背景和删除线等装饰由本模块按字节区间自行绘制，GPUI 只负责塑形与字形光栅化，
//! 因而断行、两端对齐和光标几何都完全可控。布局结果不持有窗口或回调，可安全缓存。

use crate::kp::{self, Item, ItemKind};
use crate::theme::{Family, Theme, font_for};
use gpui::{
    Bounds, FontId, GlyphId, Hsla, Pixels, Point, TextRun, Window, fill, point, px, size,
};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// 布局代数：字体、代码行号等影响排版但不在段落规格内的设置变化时递增，使所有布局缓存键失效。
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorRole {
    #[default]
    Fg,
    Heading,
    Muted,
    Link,
    Marker,
    QuoteFg,
    InlineCode,
    CodeFg,
    Keyword,
    Str,
    Number,
    Comment,
    Punct,
    Accent,
    Note,
    Tip,
    Important,
    Warning,
    Caution,
    OnAccent,
    /// 飞书风格调色板文字色：0 红、1 橙、2 黄、3 绿、4 蓝、5 紫、6 灰。
    Palette(u8),
}

impl ColorRole {
    pub fn resolve(self, theme: &Theme) -> Hsla {
        match self {
            ColorRole::Fg => theme.fg,
            ColorRole::Heading => theme.heading,
            ColorRole::Muted => theme.muted,
            ColorRole::Link => theme.link,
            ColorRole::Marker => theme.marker,
            ColorRole::QuoteFg => theme.quote_fg,
            ColorRole::InlineCode => theme.inline_code_fg,
            ColorRole::CodeFg => theme.code_fg,
            ColorRole::Keyword => theme.code_keyword,
            ColorRole::Str => theme.code_string,
            ColorRole::Number => theme.code_number,
            ColorRole::Comment => theme.code_comment,
            ColorRole::Punct => theme.code_punct,
            ColorRole::Accent => theme.accent,
            ColorRole::Note => theme.callout_note,
            ColorRole::Tip => theme.callout_tip,
            ColorRole::Important => theme.callout_important,
            ColorRole::Warning => theme.callout_warning,
            ColorRole::Caution => theme.callout_caution,
            ColorRole::OnAccent => theme.on_accent,
            ColorRole::Palette(index) => theme.palette_fg[(index as usize).min(6)],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub strike: bool,
    pub underline: bool,
    /// 行内代码的圆角底色。
    pub code_bg: bool,
    /// 文字底色：0 无，1 为 `<mark>` 默认高亮，2–8 对应调色板 0–6 的背景色。
    pub bg: u8,
    /// 基线偏移：1 为上标，-1 为下标。整段只能用一个字号塑形，上下标只做位移不缩小。
    pub shift: i8,
    pub color: ColorRole,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Wrap {
    Greedy,
    KnuthPlass,
}

/// 行内对象的占位字符（U+FFFC OBJECT REPLACEMENT CHARACTER）。
pub const OBJECT_CHAR: char = '\u{FFFC}';

/// 嵌在段落中的行内对象（如行内公式）：`at` 为占位字符的字节偏移，宽度与基线上下尺寸为逻辑像素，
/// `key` 由调用方解释（公式渲染缓存键）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineObject {
    pub at: usize,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub key: u64,
}

/// 段落输入。`spans` 按起点递增且互不重叠，未覆盖的字节使用 `base` 样式。
#[derive(Clone, Debug)]
pub struct ParaSpec {
    pub text: String,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
    pub base: InlineStyle,
    pub family: Family,
    pub font_size: f32,
    pub line_height: f32,
    pub wrap: Wrap,
    /// 行内对象，按 `at` 递增。
    pub objects: Vec<InlineObject>,
}

impl ParaSpec {
    pub fn plain(text: String, base: InlineStyle, family: Family, font_size: f32, line_height: f32, wrap: Wrap) -> Self {
        Self { text, spans: Vec::new(), base, family, font_size, line_height, wrap, objects: Vec::new() }
    }

    /// 布局缓存键：内容、样式、字号与行宽共同决定排版结果。
    pub fn cache_key(&self, width: f32) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut hasher);
        self.spans.hash(&mut hasher);
        self.base.hash(&mut hasher);
        self.family.hash(&mut hasher);
        self.font_size.to_bits().hash(&mut hasher);
        self.line_height.to_bits().hash(&mut hasher);
        self.wrap.hash(&mut hasher);
        width.to_bits().hash(&mut hasher);
        generation().hash(&mut hasher);
        for object in &self.objects {
            (object.at, object.width.to_bits(), object.ascent.to_bits(), object.descent.to_bits(), object.key).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// 补齐空隙后的完整样式区间，覆盖 `0..text.len()`。
    fn coverage(&self) -> Vec<(Range<usize>, InlineStyle)> {
        let mut runs = Vec::with_capacity(self.spans.len() * 2 + 1);
        let mut at = 0;
        for (range, style) in &self.spans {
            let start = range.start.clamp(at, self.text.len());
            let end = range.end.clamp(start, self.text.len());
            if start > at {
                runs.push((at..start, self.base));
            }
            if end > start {
                runs.push((start..end, *style));
            }
            at = end;
        }
        if at < self.text.len() {
            runs.push((at..self.text.len(), self.base));
        }
        runs
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    index: usize,
    x: f32,
    font_id: FontId,
    id: GlyphId,
    emoji: bool,
    /// 行内对象的占位字形：只占位置，不绘制。
    object: bool,
}

/// 一行视觉行：段内字节 `start..end`；`glues` 是 (生效字节, 累计偏移) 递增表。
pub struct VisualLine {
    pub start: usize,
    pub end: usize,
    origin_x: f32,
    glues: Vec<(usize, f32)>,
}

pub struct TextLayout {
    pub text: String,
    glyphs: Vec<Glyph>,
    natural_width: f32,
    pub lines: Vec<VisualLine>,
    pub line_height: f32,
    font_size: f32,
    baseline: f32,
    styles: Vec<(Range<usize>, InlineStyle)>,
    objects: Vec<InlineObject>,
}

impl TextLayout {
    /// 行内对象的绘制矩形（相对段落左上角）：(key, x, y, 宽, 高)，按基线对齐。
    pub fn object_rects(&self) -> Vec<(u64, f32, f32, f32, f32)> {
        self.objects
            .iter()
            .filter_map(|object| {
                let line_index = self.lines.iter().position(|line| object.at >= line.start && object.at < line.end)?;
                let line = &self.lines[line_index];
                let x = self.x_in_line(line, object.at);
                let baseline = line_index as f32 * self.line_height + self.baseline;
                Some((object.key, x, baseline - object.ascent, object.width, object.ascent + object.descent))
            })
            .collect()
    }

    pub fn height(&self) -> f32 {
        self.lines.len().max(1) as f32 * self.line_height
    }

    pub fn estimated_bytes(&self) -> usize {
        self.text.len()
            + self.glyphs.len() * std::mem::size_of::<Glyph>()
            + self.lines.iter().map(|line| 48 + line.glues.len() * 16).sum::<usize>()
            + self.styles.len() * 48
    }

    /// 最宽视觉行的宽度，用于表格列宽和行内元素对齐。
    pub fn max_line_width(&self) -> f32 {
        self.lines
            .iter()
            .map(|line| self.x_in_line(line, line.end))
            .fold(0.0, f32::max)
    }

    fn shaped_x(&self, byte: usize) -> f32 {
        let at = self.glyphs.partition_point(|glyph| glyph.index < byte);
        self.glyphs.get(at).map_or(self.natural_width, |glyph| glyph.x)
    }

    pub fn x_in_line(&self, line: &VisualLine, offset: usize) -> f32 {
        let offset = offset.clamp(line.start, line.end);
        let extra = line
            .glues
            .iter()
            .take_while(|(at, _)| *at <= offset)
            .last()
            .map_or(0.0, |(_, shift)| *shift);
        self.shaped_x(offset) - line.origin_x + extra
    }

    pub fn line_for_offset(&self, offset: usize) -> usize {
        self.lines
            .iter()
            .position(|line| offset <= line.end)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// 光标所在视觉行与横坐标（相对段落左上角）。
    pub fn caret(&self, offset: usize) -> (usize, f32) {
        let line = self.line_for_offset(offset);
        (line, self.x_in_line(&self.lines[line], offset))
    }

    /// 段内坐标命中到最近的字形边界。
    pub fn offset_at(&self, x: f32, y: f32) -> usize {
        let line_index = ((y / self.line_height).floor().max(0.0) as usize).min(self.lines.len() - 1);
        let line = &self.lines[line_index];
        let from = self.glyphs.partition_point(|glyph| glyph.index < line.start);
        let mut best = (line.start, f32::INFINITY);
        let candidates = self.glyphs[from..]
            .iter()
            .take_while(|glyph| glyph.index < line.end)
            .map(|glyph| glyph.index)
            .chain(std::iter::once(line.end));
        for offset in candidates {
            let distance = (self.x_in_line(line, offset) - x).abs();
            if distance < best.1 {
                best = (offset, distance);
            }
        }
        best.0
    }

    /// 区间在各视觉行上的覆盖：(行号, x0, x1)。
    pub fn range_rects(&self, range: Range<usize>) -> Vec<(usize, f32, f32)> {
        let mut rects = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let start = range.start.max(line.start);
            let end = range.end.min(line.end);
            if start < end {
                rects.push((index, self.x_in_line(line, start), self.x_in_line(line, end)));
            } else if start == end && range.end > line.end && range.start <= line.end && line.start < line.end {
                // 选区跨越本行末尾：补一个窄条，表示换行符被选中。
                let x = self.x_in_line(line, line.end);
                rects.push((index, x, x + self.font_size * 0.3));
            }
        }
        rects
    }

    /// 绘制段落。`clip` 为段落坐标系下需要绘制的纵向区间，区间外的视觉行直接跳过。
    pub fn paint(&self, origin: Point<Pixels>, clip: Range<f32>, theme: &Theme, window: &mut Window) {
        let font_size = px(self.font_size);
        for (line_index, line) in self.lines.iter().enumerate() {
            let top = line_index as f32 * self.line_height;
            if top + self.line_height < clip.start || top > clip.end {
                continue;
            }
            // 行内代码与高亮底色先画，保证在文字之下。
            for (range, style) in &self.styles {
                if !(style.code_bg || style.bg > 0) || range.end <= line.start || range.start >= line.end {
                    continue;
                }
                let inset = if style.code_bg { 3.0 } else { 1.0 };
                let x0 = self.x_in_line(line, range.start.max(line.start)) - inset;
                let x1 = self.x_in_line(line, range.end.min(line.end)) + inset;
                let pad = (self.line_height - self.font_size * 1.35).max(0.0) / 2.0;
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(origin.x + px(x0), origin.y + px(top + pad)),
                            size(px(x1 - x0), px(self.line_height - pad * 2.0)),
                        ),
                        match (style.code_bg, style.bg) {
                            (true, _) => theme.inline_code_bg,
                            (false, 1) => theme.mark_bg,
                            (false, index) => theme.palette_bg[(index as usize - 2).min(6)],
                        },
                    )
                    .corner_radii(px(if style.code_bg { 4.0 } else { 2.0 })),
                );
            }
            let baseline = top + self.baseline;
            let first = self.glyphs.partition_point(|glyph| glyph.index < line.start);
            let mut glue = 0;
            let mut shift = 0.0;
            let mut style_at = self.styles.partition_point(|(range, _)| range.end <= line.start);
            for glyph in self.glyphs[first..].iter().take_while(|glyph| glyph.index < line.end) {
                while glue < line.glues.len() && line.glues[glue].0 <= glyph.index {
                    shift = line.glues[glue].1;
                    glue += 1;
                }
                if glyph.object {
                    continue;
                }
                while style_at + 1 < self.styles.len() && self.styles[style_at].0.end <= glyph.index {
                    style_at += 1;
                }
                let style = self.styles.get(style_at).map(|(_, style)| *style).unwrap_or_default();
                let color = style.color.resolve(theme);
                let lift = match style.shift {
                    1 => -self.font_size * 0.35,
                    -1 => self.font_size * 0.22,
                    _ => 0.0,
                };
                let at = point(origin.x + px(glyph.x - line.origin_x + shift), origin.y + px(baseline + lift));
                let _ = if glyph.emoji {
                    window.paint_emoji(at, glyph.font_id, glyph.id, font_size)
                } else {
                    window.paint_glyph(at, glyph.font_id, glyph.id, font_size, color)
                };
            }
            // 删除线与下划线按样式区间逐行绘制。
            for (range, style) in &self.styles {
                if !(style.strike || style.underline) || range.end <= line.start || range.start >= line.end {
                    continue;
                }
                let x0 = self.x_in_line(line, range.start.max(line.start));
                let x1 = self.x_in_line(line, range.end.min(line.end));
                let color = style.color.resolve(theme);
                if style.strike {
                    let y = baseline - self.font_size * 0.3;
                    window.paint_quad(fill(Bounds::new(point(origin.x + px(x0), origin.y + px(y)), size(px(x1 - x0), px(1.2))), color));
                }
                if style.underline {
                    let y = baseline + self.font_size * 0.15;
                    window.paint_quad(fill(Bounds::new(point(origin.x + px(x0), origin.y + px(y)), size(px(x1 - x0), px(1.0))), color.opacity(0.55)));
                }
            }
        }
    }
}

/// Menlo 覆盖的字符范围（拉丁、希腊、西里尔、通用标点与制表符等，均在 CJK 部首区之前）。
fn mono_covers(ch: char) -> bool {
    (ch as u32) < 0x2E80
}

/// 排版一个段落。`width` 为可用行宽（px）。
pub fn layout_text(spec: &ParaSpec, width: f32, window: &Window) -> TextLayout {
    let styles = spec.coverage();
    // 按样式切出字体段；等宽段再按字符拆分，Menlo 不含的字符（汉字、全角标点、emoji 等）
    // 显式改用正文字体，而不交给 CoreText 级联回退：回退得到的苹方字形 ID 与 gpui 按 PostScript
    // 名复用的已加载字体不是同一字形表，光栅化会画出错字。
    let mut fonts: Vec<(usize, gpui::Font)> = Vec::with_capacity(styles.len());
    for (range, style) in &styles {
        let family = if style.mono { Family::Mono } else { spec.family };
        if family != Family::Mono {
            fonts.push((range.len(), font_for(family, style.bold, style.italic)));
            continue;
        }
        let text = &spec.text[range.clone()];
        let mut start = 0;
        let mut current: Option<bool> = None;
        for (offset, ch) in text.char_indices() {
            let mono = mono_covers(ch);
            if current.is_some_and(|value| value != mono) {
                let segment_family = if current == Some(true) { Family::Mono } else { Family::Text };
                fonts.push((offset - start, font_for(segment_family, style.bold, style.italic)));
                start = offset;
            }
            current = Some(mono);
        }
        if start < text.len() {
            let segment_family = if current != Some(false) { Family::Mono } else { Family::Text };
            fonts.push((text.len() - start, font_for(segment_family, style.bold, style.italic)));
        }
    }
    let runs: Vec<TextRun> = fonts
        .into_iter()
        .enumerate()
        .map(|(index, (len, font))| TextRun {
            len,
            font,
            // gpui 0.2.2 `WindowTextSystem::layout_line` 合并相邻 run 时只比较颜色与装饰，
            // 字体比较恒为真，导致同色的粗体、等宽 run 被并入前一个字体。相邻 run 交替使用
            // 两个肉眼无差的颜色强制切分；本模块自行着色，不读取这里的颜色。
            // 可在升级到修复该比较的 gpui 版本后移除。
            color: if index % 2 == 0 { Hsla::black() } else { Hsla { a: 0.999, ..Hsla::black() } },
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect();
    let shaped = window.text_system().layout_line(&spec.text, px(spec.font_size), &runs, None);
    let glyphs: Vec<Glyph> = shaped
        .runs
        .iter()
        .flat_map(|run| {
            run.glyphs.iter().map(|glyph| Glyph {
                index: glyph.index,
                x: f32::from(glyph.position.x),
                font_id: run.font_id,
                id: glyph.id,
                emoji: glyph.is_emoji,
                object: false,
            })
        })
        .collect();
    let mut glyphs = glyphs;
    let mut natural_width = f32::from(shaped.width);
    // 行内对象：占位字符按塑形得到的宽度与对象实际宽度之差，平移其后的所有字形。
    if !spec.objects.is_empty() {
        let original_x = |byte: usize| glyphs.get(glyphs.partition_point(|glyph| glyph.index < byte)).map_or(natural_width, |glyph| glyph.x);
        let deltas: Vec<(usize, f32)> = spec
            .objects
            .iter()
            .map(|object| {
                let end = object.at + OBJECT_CHAR.len_utf8();
                (object.at, object.width - (original_x(end) - original_x(object.at)))
            })
            .collect();
        for glyph in &mut glyphs {
            glyph.object = deltas.iter().any(|(at, _)| *at == glyph.index);
            glyph.x += deltas.iter().filter(|(at, _)| *at < glyph.index).map(|(_, delta)| delta).sum::<f32>();
        }
        natural_width += deltas.iter().map(|(_, delta)| delta).sum::<f32>();
    }
    let ascent = f32::from(shaped.ascent);
    let descent = f32::from(shaped.descent);
    let baseline = ((spec.line_height - (ascent + descent)) / 2.0).max(0.0) + ascent;
    let mut layout = TextLayout {
        text: spec.text.clone(),
        glyphs,
        natural_width,
        lines: Vec::new(),
        line_height: spec.line_height,
        font_size: spec.font_size,
        baseline,
        styles,
        objects: spec.objects.clone(),
    };
    let space_width = spec.font_size * 0.28;
    let mut items = kp::build_items(&layout.text, spec.font_size, space_width, |byte| layout.shaped_x(byte));
    split_oversized_boxes(&mut items, &layout, width);
    let lines = match spec.wrap {
        Wrap::KnuthPlass => kp::break_lines(&items, width),
        Wrap::Greedy => kp::greedy_lines(&items, width),
    };
    layout.lines = lines.iter().map(|line| visual_line(&layout, &items, line)).collect();
    if layout.lines.is_empty() {
        layout.lines.push(VisualLine { start: 0, end: 0, origin_x: 0.0, glues: Vec::new() });
    }
    layout
}

/// 把宽于整行的词盒拆成逐字素的盒，中间插入零宽不可伸缩胶，使超长 URL 或不可断串能按字符换行。
fn split_oversized_boxes(items: &mut Vec<Item>, layout: &TextLayout, width: f32) {
    if !items.iter().any(|item| matches!(item.kind, ItemKind::Box) && item.width > width) {
        return;
    }
    use unicode_segmentation::UnicodeSegmentation;
    let mut output = Vec::with_capacity(items.len() * 2);
    for item in items.drain(..) {
        if !(matches!(item.kind, ItemKind::Box) && item.width > width) {
            output.push(item);
            continue;
        }
        let word = &layout.text[item.from..item.to];
        let mut first = true;
        for (offset, grapheme) in word.grapheme_indices(true) {
            let from = item.from + offset;
            let to = from + grapheme.len();
            if !first {
                output.push(Item { kind: ItemKind::Glue { stretch: 0.0, shrink: 0.0 }, from, to: from, width: 0.0 });
            }
            first = false;
            output.push(Item { kind: ItemKind::Box, from, to, width: layout.shaped_x(to) - layout.shaped_x(from) });
        }
    }
    *items = output;
}

fn visual_line(layout: &TextLayout, items: &[Item], line: &kp::Line) -> VisualLine {
    if line.items.is_empty() {
        let at = items.get(line.items.start).map_or(layout.text.len(), |item| item.from);
        return VisualLine { start: at, end: at, origin_x: layout.shaped_x(at), glues: Vec::new() };
    }
    let start = items[line.items.start].from;
    let end = items[line.items.end - 1].to;
    let mut shift = 0.0;
    let mut glues = Vec::new();
    for item in &items[line.items.clone()] {
        if let ItemKind::Glue { stretch, shrink } = item.kind {
            shift += if line.ratio >= 0.0 { line.ratio * stretch } else { line.ratio * shrink };
            if shift != 0.0 {
                glues.push((item.to, shift));
            }
        }
    }
    VisualLine { start, end, origin_x: layout.shaped_x(start), glues }
}

/// 绘制一个圆角矩形（可带描边），坐标为窗口坐标。
pub fn paint_box(bounds: Bounds<Pixels>, background: Hsla, radius: f32, border: Option<(Hsla, f32)>, window: &mut Window) {
    let mut quad = fill(bounds, background).corner_radii(px(radius));
    if let Some((color, width)) = border {
        quad = quad.border_color(color).border_widths(px(width));
    }
    window.paint_quad(quad);
}
