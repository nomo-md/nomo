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
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{collections::HashMap, rc::{Rc, Weak}};

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
    CodeLabel,
    CodeLineNumber,
    Keyword,
    Str,
    Number,
    Comment,
    Punct,
    CodeFunction,
    CodeTag,
    CodeParameter,
    CodeInserted,
    CodeDeleted,
    Accent,
    AccentStrong,
    Note,
    Tip,
    Important,
    Warning,
    Caution,
    OnAccent,
    Success,
    /// 飞书风格调色板文字色：0 红、1 橙、2 黄、3 绿、4 蓝、5 紫、6 灰。
    Palette(u8),
    /// 显式 HTML/CSS 色值（RGBA），不随主题量化。
    Rgba(u32),
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
            ColorRole::CodeLabel => theme.code_fg.opacity(0.82),
            ColorRole::CodeLineNumber => theme.code_fg.opacity(0.42),
            ColorRole::Keyword => theme.code_keyword,
            ColorRole::Str => theme.code_string,
            ColorRole::Number => theme.code_number,
            ColorRole::Comment => theme.code_comment,
            ColorRole::Punct => theme.code_punct,
            ColorRole::CodeFunction => theme.code_function,
            ColorRole::CodeTag => theme.code_tag,
            ColorRole::CodeParameter => theme.code_parameter,
            ColorRole::CodeInserted => theme.code_inserted,
            ColorRole::CodeDeleted => theme.code_deleted,
            ColorRole::Accent => theme.accent,
            ColorRole::AccentStrong => theme.accent_strong,
            ColorRole::Note => theme.callout_note_fg,
            ColorRole::Tip => theme.callout_tip_fg,
            ColorRole::Important => theme.callout_important_fg,
            ColorRole::Warning => theme.callout_warning_fg,
            ColorRole::Caution => theme.callout_caution_fg,
            ColorRole::OnAccent => theme.on_accent,
            ColorRole::Success => theme.success,
            ColorRole::Palette(index) => theme.palette_fg[(index as usize).min(6)],
            ColorRole::Rgba(value) => gpui::rgba(value).into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct InlineStyle {
    pub bold: bool,
    /// 表头沿用旧版的 800 字重；普通加粗仍为 700。
    pub extra_bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub strike: bool,
    pub underline: bool,
    /// 行内代码的圆角底色。
    pub code_bg: bool,
    pub footnote: bool,
    /// 语义层明确排除链接、代码或非英文语言；默认普通正文允许英文候选。
    pub no_hyphenation: bool,
    /// 文字底色：0 无，1 为 `<mark>` 默认高亮，2–8 对应调色板 0–6 的背景色。
    pub bg: u8,
    pub bg_color: Option<u32>,
    /// HTML 字号相对于段落字号的千分比；0 表示继承。
    pub font_scale: u16,
    /// 只随当前段落和有界布局缓存存活，不建立全局字体名称表。
    pub font_family: Option<gpui::SharedString>,
    /// 基线偏移：1 为上标，-1 为下标；字形与字宽同步缩至 0.75em。
    pub shift: i8,
    pub color: ColorRole,
}

impl InlineStyle {
    fn scale(&self) -> f32 { if self.font_scale == 0 { 1.0 } else { f32::from(self.font_scale) / 1000.0 } }

    fn font(&self, family: Family) -> gpui::Font {
        let mut font = font_for(family, self.bold, self.italic);
        if let Some(name) = &self.font_family { font.family = name.clone(); }
        if self.extra_bold { font.weight = gpui::FontWeight::EXTRA_BOLD; }
        font
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Wrap {
    None,
    Greedy,
    KnuthPlass,
    TableCell,
}

/// 行内对象的占位字符（U+FFFC OBJECT REPLACEMENT CHARACTER）。
pub const OBJECT_CHAR: char = '\u{FFFC}';

/// 注释的字形跟随所在段落释放，绘制时复用测量过的换行与字形。
#[derive(Clone)]
pub struct CommentLabel(pub Rc<TextLayout>);
impl std::fmt::Debug for CommentLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple("CommentLabel").field(&self.0.cache_key).finish() }
}
impl PartialEq for CommentLabel { fn eq(&self, other: &Self) -> bool { self.0.cache_key == other.0.cache_key } }
impl Eq for CommentLabel {}
impl Hash for CommentLabel { fn hash<H: Hasher>(&self, state: &mut H) { self.0.cache_key.hash(state); } }

/// 嵌在段落中的行内对象（如行内公式）：`at` 为占位字符的字节偏移，宽度与基线上下尺寸为逻辑像素，
/// `key` 由调用方解释（公式渲染缓存键）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InlineAsset {
    MathInput,
    Embed(u64),
    Image(std::path::PathBuf),
    Comment { label: Option<CommentLabel>, font_size: u32, zoom: u32 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct InlineObject {
    pub at: usize,
    pub width: f32,
    /// 图片百分比以最终段落／单元格宽度为基准；塑形前解析，不进入缓存状态。
    pub width_percent: Option<f32>,
    pub ascent: f32,
    pub descent: f32,
    pub key: InlineAsset,
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
            (object.at, object.width.to_bits(), object.ascent.to_bits(), object.descent.to_bits(), &object.key).hash(&mut hasher);
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
                runs.push((at..start, self.base.clone()));
            }
            if end > start {
                runs.push((start..end, style.clone()));
            }
            at = end;
        }
        if at < self.text.len() {
            runs.push((at..self.text.len(), self.base.clone()));
        }
        runs
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    index: usize,
    x: f32,
    ink_offset: f32,
    font_size: f32,
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
    trim_end: f32,
    hang: f32,
    hyphen: Option<HyphenGlyph>,
}

/// 连字符没有源码字节，只属于选定视觉行；不可加入 text 或源字形数组。
#[derive(Clone, Copy)]
struct HyphenGlyph {
    font_id: FontId,
    id: GlyphId,
    font_size: f32,
    width: f32,
    color: ColorRole,
    lift: f32,
}

struct DiscretionaryHyphen {
    at: usize,
    glyph: HyphenGlyph,
}

pub struct TextLayout {
    cache_key: u64,
    pub text: String,
    glyphs: Vec<Glyph>,
    natural_width: f32,
    max_width: f32,
    min_content_width: f32,
    pub lines: Vec<VisualLine>,
    pub line_height: f32,
    font_size: f32,
    baseline: f32,
    styles: Vec<(Range<usize>, InlineStyle)>,
    objects: Vec<InlineObject>,
}

impl TextLayout {
    pub(crate) fn first_baseline(&self) -> f32 { self.baseline }
    /// 对象点击返回占位符起点，而不是用于光标的最近左右边界。
    pub fn object_at_point(&self, x: f32, y: f32) -> Option<usize> {
        self.objects.iter().find_map(|object| {
            let row = self.lines.partition_point(|line| line.end <= object.at);
            let line = self.lines.get(row)?;
            if object.at < line.start { return None; }
            let left = self.x_in_line(line, object.at);
            let baseline = row as f32 * self.line_height + self.baseline;
            (x >= left && x <= left + object.width && y >= baseline - object.ascent && y <= baseline + object.descent)
                .then_some(object.at)
        })
    }

    /// 行内对象的绘制矩形（相对段落左上角）：(key, x, y, 宽, 高)，按基线对齐。
    pub fn object_rects(&self) -> Vec<(&InlineAsset, f32, f32, f32, f32)> {
        self.object_rects_clipped(f32::NEG_INFINITY..f32::INFINITY)
    }
    pub fn object_rects_clipped(&self, clip: Range<f32>) -> Vec<(&InlineAsset, f32, f32, f32, f32)> {
        self.object_rects_with_offsets(clip).map(|(_, key, x, y, w, h)| (key, x, y, w, h)).collect()
    }
    pub fn object_rects_with_offsets(&self, clip: Range<f32>) -> impl Iterator<Item = (usize, &InlineAsset, f32, f32, f32, f32)> {
        self.objects
            .iter()
            .filter_map(move |object| {
                let line_index = self.lines.partition_point(|line| line.end <= object.at);
                let line = self.lines.get(line_index)?;
                if object.at < line.start { return None; }
                let x = self.x_in_line(line, object.at);
                let baseline = line_index as f32 * self.line_height + self.baseline;
                if baseline - object.ascent > clip.end || baseline + object.descent < clip.start { return None; }
                Some((object.at, &object.key, x, baseline - object.ascent, object.width, object.ascent + object.descent))
            })
    }

    pub fn height(&self) -> f32 {
        self.lines.len().max(1) as f32 * self.line_height
    }

    pub fn estimated_bytes(&self) -> usize {
        self.text.len()
            + self.glyphs.len() * std::mem::size_of::<Glyph>()
            + self.lines.capacity() * std::mem::size_of::<VisualLine>()
            + self.lines.iter().map(|line| line.glues.capacity() * std::mem::size_of::<(usize, f32)>()).sum::<usize>()
            + self.styles.capacity() * std::mem::size_of::<(Range<usize>, InlineStyle)>()
            + self.styles.iter().map(|(_, style)| style.font_family.as_ref().map_or(0, |name| name.len())).sum::<usize>()
            + self.objects.capacity() * std::mem::size_of::<InlineObject>()
            + self.objects.iter().map(|o| match &o.key { InlineAsset::Image(path) => path.as_os_str().len(), InlineAsset::Comment { label: Some(label), .. } => label.0.estimated_bytes(), _ => 0 }).sum::<usize>()
    }

    /// 最宽视觉行的宽度，用于表格列宽和行内元素对齐。
    pub fn max_line_width(&self) -> f32 {
        self.max_width
    }

    /// 表格正常断点之间的最大宽度；紧急拆词不参与列宽下限。
    pub(crate) fn min_content_width(&self) -> f32 { self.min_content_width }

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
        let edge = if offset == line.end {
            line.hyphen.map_or(0.0, |hyphen| hyphen.width) - line.trim_end
        } else { 0.0 };
        self.shaped_x(offset) - line.origin_x + extra + edge
    }

    pub fn line_for_offset(&self, offset: usize) -> usize {
        // 两行共用一个换行字节边界，行首命中必须映回刚命中的后一行。
        self.lines.partition_point(|line| line.start <= offset).saturating_sub(1)
    }

    pub fn caret_row(&self, offset: usize, upstream: bool) -> usize {
        let row = self.line_for_offset(offset);
        if upstream && row > 0 && self.lines[row].start == offset && self.lines[row - 1].end == offset { row - 1 } else { row }
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
        if let Some(hyphen) = line.hyphen {
            // 整个虚拟字形都命中原来的断词边界，不制造一个可移动到的新字符。
            let right = self.x_in_line(line, line.end);
            if x >= right - hyphen.width { return line.end; }
        }
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
        self.range_rects_in(range, 0..self.lines.len())
    }
    pub fn range_rects_in(&self, range: Range<usize>, rows: Range<usize>) -> Vec<(usize, f32, f32)> {
        let mut rects = Vec::new();
        for index in rows.start.min(self.lines.len())..rows.end.min(self.lines.len()) {
            let line = &self.lines[index];
            let start = range.start.max(line.start);
            let end = range.end.min(line.end);
            if start < end {
                rects.push((index, self.x_in_line(line, start), self.x_in_line(line, end)));
            } else if start == end && range.end > line.end && range.start <= line.end
                && self.lines.get(index + 1).is_some_and(|next| next.start > line.end) {
                // 只有实际换行／被折掉的空白才占源码。共享断词边界不能凭空多选一个字符。
                let x = self.x_in_line(line, line.end);
                rects.push((index, x, x + self.font_size * 0.3));
            }
        }
        rects
    }

    fn visible_lines(&self, clip: Range<f32>) -> impl Iterator<Item = (&VisualLine, f32)> {
        let first = ((clip.start / self.line_height) as usize).saturating_sub(1);
        self.lines.iter().enumerate().skip(first)
            .map(|(index, line)| (line, index as f32 * self.line_height))
            .take_while(move |&(_, top)| top <= clip.end)
            .filter(move |&(_, top)| top + self.line_height >= clip.start)
    }

    /// 绘制段落。`clip` 使用段落坐标。
    pub fn paint(&self, origin: Point<Pixels>, clip: Range<f32>, theme: &Theme, window: &mut Window) {
        self.paint_backgrounds(origin, clip.clone(), theme, window);
        self.paint_foreground(origin, clip, theme, window);
    }

    pub(crate) fn paint_backgrounds(&self, origin: Point<Pixels>, clip: Range<f32>, theme: &Theme, window: &mut Window) {
        for (line, top) in self.visible_lines(clip) {
            for (range, style) in &self.styles {
                if !(style.code_bg || style.footnote || style.bg > 0 || style.bg_color.is_some()) || range.end <= line.start || range.start >= line.end {
                    continue;
                }
                let inset = if style.code_bg || style.footnote { 0.0 } else { 1.0 };
                let x0 = self.x_in_line(line, range.start.max(line.start)) - inset;
                let x1 = self.x_in_line(line, range.end.min(line.end)) + inset;
                let pad = (self.line_height - self.font_size * style.scale() * 1.35).max(0.0) / 2.0;
                if style.footnote {
                    let h = self.font_size * 0.95;
                    let y = top + self.baseline - self.font_size * 1.1;
                    paint_box(Bounds::new(point(origin.x + px(x0), origin.y + px(y)), size(px((x1 - x0).max(0.0)), px(h))),
                        theme.surface, 3.0, Some((theme.border, 1.0)), window);
                    continue;
                }
                if style.code_bg {
                    paint_box(Bounds::new(point(origin.x + px(x0), origin.y + px(top + pad)), size(px((x1 - x0).max(0.0)), px(self.line_height - pad * 2.0))),
                        theme.inline_code_bg, 4.0, Some((theme.border, 1.0)), window);
                    continue;
                }
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(origin.x + px(x0), origin.y + px(top + pad)),
                            size(px(x1 - x0), px(self.line_height - pad * 2.0)),
                        ),
                        style.bg_color.map_or_else(|| match style.bg {
                            1 => theme.mark_bg,
                            index => theme.palette_bg[(index.saturating_sub(2) as usize).min(6)],
                        }, |value| gpui::rgba(value).into()),
                    )
                    .corner_radii(px(if style.code_bg { 4.0 } else { 2.0 })),
                );
            }
        }
    }

    pub(crate) fn paint_foreground(&self, origin: Point<Pixels>, clip: Range<f32>, theme: &Theme, window: &mut Window) {
        let default_style = InlineStyle::default();
        for (line, top) in self.visible_lines(clip) {
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
                let style = self.styles.get(style_at).map(|(_, style)| style).unwrap_or(&default_style);
                let color = style.color.resolve(theme);
                let lift = match style.shift {
                    1 => -self.font_size * style.scale() * 0.35,
                    -1 => self.font_size * style.scale() * 0.22,
                    _ => 0.0,
                };
                let font_size = px(glyph.font_size);
                let at = point(origin.x + px(glyph.x + glyph.ink_offset - line.origin_x + shift), origin.y + px(baseline + lift));
                let _ = if glyph.emoji {
                    window.paint_emoji(at, glyph.font_id, glyph.id, font_size)
                } else {
                    window.paint_glyph(at, glyph.font_id, glyph.id, font_size, color)
                };
            }
            if let Some(hyphen) = line.hyphen {
                let x = self.x_in_line(line, line.end) - hyphen.width;
                let _ = window.paint_glyph(point(origin.x + px(x), origin.y + px(baseline + hyphen.lift)),
                    hyphen.font_id, hyphen.id, px(hyphen.font_size), hyphen.color.resolve(theme));
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
                    let y = baseline - self.font_size * style.scale() * 0.3;
                    window.paint_quad(fill(Bounds::new(point(origin.x + px(x0), origin.y + px(y)), size(px(x1 - x0), px(1.2))), color));
                }
                if style.underline {
                    let y = baseline + self.font_size * style.scale() * 0.15;
                    window.paint_quad(fill(Bounds::new(point(origin.x + px(x0), origin.y + px(y)), size(px(x1 - x0), px(1.0))), color.opacity(0.55)));
                }
            }
        }
    }
}

/// 只索引上一帧仍被显示列表持有的段落；弱引用不会延长正文或旧布局的生命周期。
#[derive(Default)]
pub struct TextLayoutReuse { entries: HashMap<u64, Weak<TextLayout>> }
impl TextLayoutReuse {
    pub fn remember(&mut self, layout: &Rc<TextLayout>) {
        if self.entries.len() < 1024 { self.entries.insert(layout.cache_key, Rc::downgrade(layout)); }
    }
    pub fn layout(&mut self, spec: &ParaSpec, width: f32, window: &Window) -> Rc<TextLayout> {
        let key = spec.cache_key(width);
        if let Some(layout) = self.entries.get(&key).and_then(Weak::upgrade) { return layout; }
        let layout = Rc::new(layout_text(spec, width, window));
        self.remember(&layout);
        layout
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
    let styled_font = |family, style: &InlineStyle| style.font(family);
    for (range, style) in &styles {
        let family = if style.mono { Family::Mono } else { spec.family };
        if style.font_family.is_some() || !matches!(family, Family::Mono | Family::Code) {
            fonts.push((range.len(), styled_font(family, style)));
            continue;
        }
        let text = &spec.text[range.clone()];
        let mut start = 0;
        let mut current: Option<bool> = None;
        // 字素簇必须使用同一字体；按字符拆分会把 Emoji 的 ZWJ/肤色修饰拆到另一个 run。
        for (offset, cluster) in text.grapheme_indices(true) {
            let mono = cluster.chars().all(mono_covers);
            if current.is_some_and(|value| value != mono) {
                let segment_family = if current == Some(true) { family } else { Family::Text };
                fonts.push((offset - start, styled_font(segment_family, style)));
                start = offset;
            }
            current = Some(mono);
        }
        if start < text.len() {
            let segment_family = if current != Some(false) { family } else { Family::Text };
            fonts.push((text.len() - start, styled_font(segment_family, style)));
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
                ink_offset: 0.0,
                font_size: spec.font_size,
                font_id: run.font_id,
                id: glyph.id,
                emoji: glyph.is_emoji,
                object: false,
            })
        })
        .collect();
    let mut glyphs = glyphs;
    let mut natural_width = f32::from(shaped.width);
    // CJK 回退字面通常不是两个拉丁单元宽。等宽区按 Unicode 列宽留位，
    // 只移动字形，不拉伸笔画；位置修正同时供绘制、命中与选区使用。
    if !spec.text.is_ascii() && (matches!(spec.family, Family::Mono | Family::Code) || styles.iter().any(|(_, style)| style.mono)) {
        let family = if matches!(spec.family, Family::Mono | Family::Code) { spec.family } else { Family::Mono };
        let probe = TextRun { len: 1, font: font_for(family, false, false), color: Hsla::black(),
            background_color: None, underline: None, strikethrough: None };
        let cell = f32::from(window.text_system().layout_line("0", px(spec.font_size), &[probe], None).width);
        let mut shift = 0.0;
        let mut style_at = 0;
        let mut glyph_at = 0;
        for (at, cluster) in spec.text.grapheme_indices(true) {
            while style_at + 1 < styles.len() && styles[style_at].0.end <= at { style_at += 1; }
            let style = &styles[style_at].1;
            let first = glyph_at;
            while glyph_at < glyphs.len() && glyphs[glyph_at].index < at + cluster.len() { glyph_at += 1; }
            let last = glyph_at;
            if first == last { continue; }
            let left = glyphs[first].x;
            let right = glyphs.get(last).map_or(natural_width, |glyph| glyph.x);
            let columns = cluster.width();
            let fallback = columns > 1 || cluster.chars().any(|ch| matches!(ch as u32, 0x2500..=0x259f | 0x2e80..=0xffff));
            let mono = style.font_family.is_none() && (style.mono || matches!(spec.family, Family::Mono | Family::Code));
            let delta = if mono && fallback && columns > 0 { cell * columns as f32 - (right - left) } else { 0.0 };
            for glyph in &mut glyphs[first..last] { glyph.x += shift; glyph.ink_offset += delta * 0.5; }
            shift += delta;
        }
        natural_width += shift;
    }
    // 保留整段塑形的字素/连字结果，按样式缩放字形和 advance；留白参与断行与光标几何。
    let transforms: Vec<_> = styles.iter().enumerate().map(|(index, (range, style))| {
        let first = glyphs.partition_point(|glyph| glyph.index < range.start);
        let last = glyphs.partition_point(|glyph| glyph.index < range.end);
        let left = glyphs.get(first).map_or(natural_width, |glyph| glyph.x);
        let right = glyphs.get(last).map_or(natural_width, |glyph| glyph.x);
        let scale = style.scale() * (if style.shift != 0 { 0.75 } else { 1.0 }) * if style.code_bg { 0.92 } else { 1.0 };
        let pad = spec.font_size * if style.footnote { 0.18 } else { 0.375 };
        let boxed = |s: &InlineStyle| s.code_bg || s.footnote;
        let before = if boxed(style) && (index == 0 || !boxed(&styles[index - 1].1)) { pad } else { 0.0 };
        let after = if boxed(style) && styles.get(index + 1).is_none_or(|(_, next)| !boxed(next)) { pad } else { 0.0 };
        (first, last, left, right, scale, before, after, range.start)
    }).collect();
    let mut advance = 0.0;
    for (first, last, left, right, scale, before, after, start) in transforms {
        for glyph in &mut glyphs[first..last] {
            glyph.x = left + advance + (glyph.x - left) * scale;
            glyph.ink_offset *= scale;
            if glyph.index == start { glyph.ink_offset += before; } else { glyph.x += before; }
            glyph.font_size = spec.font_size * scale;
        }
        advance += (right - left) * (scale - 1.0) + before + after;
    }
    natural_width += advance;
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
    let text_scale = styles.iter().map(|(_, style)| style.scale()).fold(1.0f32, f32::max);
    let ascent = spec.objects.iter().map(|o| o.ascent).fold(f32::from(shaped.ascent) * text_scale, f32::max);
    let descent = spec.objects.iter().map(|o| o.descent).fold(f32::from(shaped.descent) * text_scale, f32::max);
    let line_height = spec.line_height.max(ascent + descent);
    let baseline = ((line_height - (ascent + descent)) / 2.0).max(0.0) + ascent;
    let mut layout = TextLayout {
        cache_key: spec.cache_key(width),
        text: spec.text.clone(),
        glyphs,
        natural_width,
        max_width: 0.0,
        min_content_width: 0.0,
        lines: Vec::new(),
        line_height,
        font_size: spec.font_size,
        baseline,
        styles,
        objects: spec.objects.clone(),
    };
    let space_width = spec.font_size * 0.28;
    // 表格等行内投影保留显式换行；先按逻辑行分段，再复用原有自动折行。
    // split 保留首尾和连续空行，所有条目仍使用原段落字节坐标。
    let mut start = 0;
    for logical in layout.text.split('\n') {
        let text = logical.strip_suffix('\r').unwrap_or(logical);
        if spec.wrap == Wrap::None {
            layout.lines.push(VisualLine { start, end: start + text.len(), origin_x: layout.shaped_x(start), glues: Vec::new(), trim_end: 0.0, hang: 0.0, hyphen: None });
            start += logical.len() + 1;
            continue;
        }
        let x_at = |byte| layout.shaped_x(start + byte);
        let mut items = if spec.wrap == Wrap::TableCell {
            kp::build_table_items(text, x_at)
        } else {
            kp::build_items(text, spec.font_size, space_width, x_at)
        };
        for item in &mut items {
            item.from += start;
            item.to += start;
        }
        // 每个逻辑行的行尾空格都必须保留宽度和光标位置。
        for item in items.iter_mut().rev() {
            if item.from == item.to || !layout.text[item.from..item.to].chars().all(char::is_whitespace) {
                break;
            }
            item.kind = ItemKind::Box;
        }
        let mut atomic: Vec<Item> = Vec::with_capacity(items.len());
        let mut item_index = 0;
        let mut code_index = 0;
        let code: Vec<_> = layout.styles.iter().filter(|(_, style)|
            style.footnote || (style.code_bg && spec.wrap != Wrap::TableCell)).map(|(range, _)| range).collect();
        while item_index < items.len() {
            let item = items[item_index];
            while code_index < code.len() && code[code_index].end <= item.from { code_index += 1; }
            if let Some(range) = code.get(code_index).filter(|range| item.from < range.end && range.start < item.to) {
                let mut end = item_index + 1;
                while end < items.len() && items[end].from < range.end { end += 1; }
                let to = items[end - 1].to;
                atomic.push(Item { kind: ItemKind::Box, from: item.from, to, width: layout.shaped_x(to) - layout.shaped_x(item.from) });
                item_index = end;
            } else { atomic.push(item); item_index += 1; }
        }
        items = atomic;
        if spec.wrap == Wrap::TableCell {
            layout.min_content_width = layout.min_content_width.max(table_min_content_width(&items, &layout));
        }
        let hyphens = if spec.wrap == Wrap::KnuthPlass && items.len() <= kp::MAX_KP_ITEMS {
            english_breaks(&layout, start..start + text.len(), spec, window)
        } else { Vec::new() };
        split_at_hyphens(&mut items, &layout, &hyphens);
        split_oversized_boxes(&mut items, &layout, width, &hyphens, spec.wrap == Wrap::TableCell);
        let edges = if spec.wrap == Wrap::KnuthPlass { punctuation_edges(&items, &layout, window) } else { Vec::new() };
        let mut hyphen_index = 0;
        let breaks: Vec<_> = items.iter().enumerate().filter_map(|(index, item)| {
            while hyphen_index < hyphens.len() && hyphens[hyphen_index].at < item.to { hyphen_index += 1; }
            hyphens.get(hyphen_index).filter(|hyphen| hyphen.at == item.to
                && matches!(item.kind, ItemKind::Box) && item.from < item.to)
                .map(|hyphen| kp::Hyphen { at: index + 1, width: hyphen.glyph.width })
        }).collect();
        if items.is_empty() {
            layout.lines.push(VisualLine { start, end: start, origin_x: layout.shaped_x(start), glues: Vec::new(), trim_end: 0.0, hang: 0.0, hyphen: None });
        } else {
            let lines = match spec.wrap {
                Wrap::None => unreachable!("unwrapped lines are handled before line breaking"),
                Wrap::KnuthPlass => if edges.is_empty() && breaks.is_empty() { kp::break_lines(&items, width) }
                    else { kp::break_lines_with(&items, width, &edges, &breaks) },
                Wrap::Greedy | Wrap::TableCell => kp::greedy_lines(&items, width),
            };
            layout.lines.reserve(lines.len());
            for line in &lines {
                let visual = visual_line(&layout, &items, line, &edges, &hyphens);
                layout.lines.push(visual);
            }
        }
        start += logical.len() + 1;
    }
    layout.max_width = layout.lines.iter().map(|line| (layout.x_in_line(line, line.end) - line.hang).max(0.0)).fold(0.0, f32::max);
    layout
}

/// 与实际折行共用条目；允许缩小的图片不撑大最小列宽，公式等对象保持完整。
fn table_min_content_width(items: &[Item], layout: &TextLayout) -> f32 {
    let mut minimum = 0.0_f32;
    let mut unbroken = 0.0;
    let mut after_break = false;
    let first = items.first().map_or(0, |item| item.from);
    let mut object = layout.objects.partition_point(|object| object.at < first);
    for (index, item) in items.iter().enumerate() {
        if matches!(item.kind, ItemKind::Glue { .. }) {
            if index > 0 && matches!(items[index - 1].kind, ItemKind::Box) {
                minimum = minimum.max(unbroken);
                unbroken = 0.0;
                after_break = true;
            } else if !after_break {
                // 首行缩进在实际排版中保留；仅断点之后的空白可被跳过。
                unbroken += item.width;
            }
            continue;
        }
        after_break = false;
        let mut width = item.width;
        while let Some(inline) = layout.objects.get(object).filter(|inline| inline.at < item.to) {
            if inline.at >= item.from && matches!(inline.key, InlineAsset::Image(_)) {
                width -= inline.width;
            }
            object += 1;
        }
        unbroken += width.max(0.0);
    }
    minimum.max(unbroken)
}

/// 单次可见段落最多识别 64 KiB、4,096 个断词候选；不缓存正文词语。
fn english_breaks(layout: &TextLayout, range: Range<usize>, spec: &ParaSpec, window: &Window) -> Vec<DiscretionaryHyphen> {
    if range.len() > 64 * 1024 || matches!(spec.family, Family::Mono | Family::Code) { return Vec::new(); }
    let mut output = Vec::new();
    let mut glyph_cache = HashMap::new();
    let default_style = InlineStyle::default();
    kp::hyphenation::for_each_word(&layout.text[range.clone()], |word| {
        if output.len() >= 4096 { return; }
        let word = range.start + word.start..range.start + word.end;
        let first_style = layout.styles.partition_point(|(span, _)| span.end <= word.start);
        if layout.styles[first_style..].iter().take_while(|(span, _)| span.start < word.end)
            .any(|(_, style)| style.no_hyphenation || style.mono || style.code_bg || style.footnote || style.color == ColorRole::Link) {
            return;
        }
        for offset in kp::hyphenation::points(&layout.text[word.clone()]) {
            if output.len() >= 4096 { break; }
            let at = word.start + offset;
            let next = layout.glyphs.partition_point(|glyph| glyph.index < at);
            if layout.glyphs.get(next).is_none_or(|glyph| glyph.index != at) || next == 0 { continue; }
            let previous = layout.glyphs[next - 1];
            // 连字内部或零 advance 边界不参与断词，维持既有塑形簇的点击与 IME 坐标。
            if previous.object || layout.glyphs[next].x <= previous.x { continue; }
            let style_index = layout.styles.partition_point(|(span, _)| span.end <= at - 1);
            let style = layout.styles.get(style_index).map(|(_, style)| style).unwrap_or(&default_style);
            let key = (style.bold, style.extra_bold, style.italic, previous.font_size.to_bits(), style.font_family.clone());
            if !glyph_cache.contains_key(&key) {
                if glyph_cache.len() >= 64 { continue; }
                let font = style.font(spec.family);
                let run = TextRun { len: 1, font, color: Hsla::black(), background_color: None, underline: None, strikethrough: None };
                let shaped = window.text_system().layout_line("-", px(previous.font_size), &[run], None);
                let mark = shaped.runs.first().filter(|run| shaped.runs.len() == 1 && run.glyphs.len() == 1)
                    .filter(|_| f32::from(shaped.width).is_finite() && f32::from(shaped.width) > 0.0)
                    .map(|run| HyphenGlyph { font_id: run.font_id, id: run.glyphs[0].id,
                        font_size: previous.font_size, width: f32::from(shaped.width), color: style.color, lift: 0.0 });
                glyph_cache.insert(key.clone(), mark);
            }
            if let Some(mut glyph) = glyph_cache.get(&key).copied().flatten() {
                glyph.color = style.color;
                glyph.lift = match style.shift { 1 => -layout.font_size * style.scale() * 0.35, -1 => layout.font_size * style.scale() * 0.22, _ => 0.0 };
                output.push(DiscretionaryHyphen { at, glyph });
            }
        }
    });
    output
}

fn split_at_hyphens(items: &mut Vec<Item>, layout: &TextLayout, hyphens: &[DiscretionaryHyphen]) {
    if hyphens.is_empty() { return; }
    let mut output = Vec::with_capacity(items.len() + hyphens.len());
    let mut index = 0;
    for item in items.drain(..) {
        let mut from = item.from;
        while index < hyphens.len() && hyphens[index].at <= from { index += 1; }
        while matches!(item.kind, ItemKind::Box) && index < hyphens.len() && hyphens[index].at < item.to {
            let to = hyphens[index].at;
            output.push(Item { kind: ItemKind::Box, from, to, width: layout.shaped_x(to) - layout.shaped_x(from) });
            from = to;
            index += 1;
        }
        output.push(if from == item.from { item } else {
            Item { kind: item.kind, from, to: item.to, width: layout.shaped_x(item.to) - layout.shaped_x(from) }
        });
    }
    *items = output;
}

/// 紧急断行只处理超宽的盒，保留字素和塑形簇；表格中的行内代码允许折行。
fn split_oversized_boxes(
    items: &mut Vec<Item>,
    layout: &TextLayout,
    width: f32,
    hyphens: &[DiscretionaryHyphen],
    wrap_inline_code: bool,
) {
    let break_width = |at| hyphens.binary_search_by_key(&at, |hyphen| hyphen.at).ok().map_or(0.0, |index| hyphens[index].glyph.width);
    if !items.iter().any(|item| matches!(item.kind, ItemKind::Box) && item.width + break_width(item.to) > width) { return; }
    let mut output = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !matches!(item.kind, ItemKind::Box) || item.width + break_width(item.to) <= width {
            output.push(item);
            continue;
        }
        let first_style = layout.styles.partition_point(|(range, _)| range.end <= item.from);
        let mut styles = &layout.styles[first_style..];
        if !wrap_inline_code {
            if styles.iter().take_while(|(range, _)| range.start < item.to)
                .any(|(_, style)| style.code_bg || style.footnote) {
                output.push(item);
                continue;
            }
            styles = &[];
        }
        let word = &layout.text[item.from..item.to];
        let mut from = item.from;
        let mut previous = "";
        for (offset, grapheme) in word.grapheme_indices(true) {
            let at = item.from + offset;
            while styles.first().is_some_and(|(range, _)| range.end <= at) {
                styles = &styles[1..];
            }
            // 代码中的标点是字面内容；脚注标签仍保持完整。
            let can_break = match styles.first() {
                Some((range, style)) if range.start < at && style.footnote => false,
                Some((range, style)) if range.start < at && style.code_bg => true,
                _ => kp::can_break_between(previous, grapheme),
            };
            if at > from && can_break
                && layout.glyphs.get(layout.glyphs.partition_point(|glyph| glyph.index < at))
                    .is_some_and(|glyph| glyph.index == at) {
                output.push(Item { kind: ItemKind::Box, from, to: at, width: layout.shaped_x(at) - layout.shaped_x(from) });
                output.push(Item { kind: ItemKind::Glue { stretch: 0.0, shrink: 0.0 }, from: at, to: at, width: 0.0 });
                from = at;
            }
            previous = grapheme;
        }
        output.push(Item { kind: ItemKind::Box, from, to: item.to, width: layout.shaped_x(item.to) - layout.shaped_x(from) });
    }
    *items = output;
}

/// 字形墨迹边界限定可裁的留白，保留完整字形；测量缓存只活到本段完成，最多 128 项。
fn punctuation_edges(items: &[Item], layout: &TextLayout, window: &Window) -> Vec<kp::Edges> {
    if items.len() > kp::MAX_KP_ITEMS { return Vec::new(); }
    let mut metrics = HashMap::new();
    let mut edges = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        if !matches!(item.kind, ItemKind::Box) || item.from == item.to { continue; }
        let text = &layout.text[item.from..item.to];
        let opening = kp::no_line_end(text);
        let closing = kp::no_line_start(text);
        if !(opening || closing) || text.chars().count() != 1 { continue; }
        let style_index = layout.styles.partition_point(|(range, _)| range.end <= item.from);
        if layout.styles.get(style_index).is_some_and(|(_, style)| style.code_bg || style.footnote) { continue; }
        let index = layout.glyphs.partition_point(|glyph| glyph.index < item.from);
        let Some(glyph) = layout.glyphs.get(index).filter(|glyph| glyph.index == item.from && !glyph.object) else { continue; };
        let ch = text.chars().next().unwrap();
        let key = (glyph.font_id, glyph.font_size.to_bits(), ch);
        if !metrics.contains_key(&key) {
            if metrics.len() >= 128 { continue; }
            let bounds = window.text_system().typographic_bounds(glyph.font_id, px(glyph.font_size), ch).ok()
                .map(|bounds| (f32::from(bounds.origin.x), f32::from(bounds.origin.x + bounds.size.width)))
                .filter(|(left, right)| left.is_finite() && right.is_finite());
            metrics.insert(key, bounds);
        }
        let Some((left, right)) = metrics.get(&key).copied().flatten() else { continue; };
        let trim_start = if opening { left.max(0.0).min(glyph.font_size * 0.5).min(item.width.max(0.0)) } else { 0.0 };
        let trim_end = if closing { (item.width - right).max(0.0).min(glyph.font_size * 0.5).min(item.width.max(0.0)) } else { 0.0 };
        let hang = if "，。、；：！？".contains(ch) { (item.width - trim_end).max(0.0).min(glyph.font_size * 0.5) } else { 0.0 };
        if trim_start > 0.0 || trim_end > 0.0 || hang > 0.0 {
            if edges.is_empty() { edges.resize(items.len(), kp::Edges::default()); }
            edges[item_index] = kp::Edges { trim_start, trim_end, hang };
        }
    }
    edges
}

fn visual_line(layout: &TextLayout, items: &[Item], line: &kp::Line, edges: &[kp::Edges], hyphens: &[DiscretionaryHyphen]) -> VisualLine {
    if line.items.is_empty() {
        let at = items.get(line.items.start).map_or(layout.text.len(), |item| item.from);
        return VisualLine { start: at, end: at, origin_x: layout.shaped_x(at), glues: Vec::new(), trim_end: 0.0, hang: 0.0, hyphen: None };
    }
    let start = items[line.items.start].from;
    let end = items[line.items.end - 1].to;
    let mut shift = 0.0;
    let mut glues = Vec::new();
    for item in &items[line.items.clone()] {
        if let ItemKind::Glue { stretch, shrink } = item.kind {
            shift += if line.ratio >= 0.0 { line.ratio * stretch } else { line.ratio * shrink };
            if shift != 0.0 { glues.push((item.to, shift)); }
        }
    }
    let first = edges.get(line.items.start).copied().unwrap_or_default();
    let last = edges.get(line.items.end - 1).copied().unwrap_or_default();
    let hyphen = (line.items.end < items.len())
        .then(|| hyphens.binary_search_by_key(&end, |hyphen| hyphen.at).ok().map(|index| hyphens[index].glyph)).flatten();
    VisualLine { start, end, origin_x: layout.shaped_x(start) + first.trim_start, glues,
        trim_end: last.trim_end, hang: last.hang, hyphen }
}

/// 绘制一个圆角矩形（可带描边），坐标为窗口坐标。
pub fn paint_box(bounds: Bounds<Pixels>, background: Hsla, radius: f32, border: Option<(Hsla, f32)>, window: &mut Window) {
    let mut quad = fill(bounds, background).corner_radii(px(radius));
    if let Some((color, width)) = border {
        quad = quad.border_color(color).border_widths(px(width));
    }
    window.paint_quad(quad);
}
