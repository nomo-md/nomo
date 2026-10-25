//! 块级语义渲染：pulldown-cmark 事件 → 与主题无关的显示列表。
//!
//! 显示列表只含相对块左上角的几何、文本布局和颜色角色，因此可按（块文本、行宽、缩放、断行模式、
//! 图片尺寸）缓存，切换主题不需要重排。文本项附带“渲染文本区间 → 块内源码偏移”的映射，
//! 用于点击定位、选区与搜索高亮。

use super::highlight;
use crate::embeds::{EmbedKind, EmbedState};
use crate::layout::{ColorRole, InlineObject, InlineStyle, OBJECT_CHAR, ParaSpec, TextLayout, Wrap, layout_text};
use crate::theme::{Family, Metrics};
use gpui::Window;
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// 渲染文本区间到块内源码偏移的映射段；`exact` 为真时逐字节对应，否则整段对应到 `source`。
#[derive(Clone, Debug)]
pub struct MapSeg {
    pub text: Range<usize>,
    pub source: usize,
    pub exact: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paint {
    Border,
    CodeBg,
    CodeBorder,
    QuoteBorder,
    TableBorder,
    TableHeader,
    Surface,
    Role(ColorRole),
}

pub enum DisplayItem {
    Text { x: f32, y: f32, layout: Rc<TextLayout>, map: Vec<MapSeg> },
    Rect { x: f32, y: f32, w: f32, h: f32, paint: Paint, radius: f32, border: Option<Paint> },
    Image { x: f32, y: f32, w: f32, h: f32, path: PathBuf },
    Checkbox { x: f32, y: f32, size: f32, checked: bool, source: usize },
    /// 代码块右上角的复制按钮；`text` 为代码正文。
    CopyButton { x: f32, y: f32, size: f32, text: Rc<str> },
    /// 已渲染的公式或 Mermaid 图（位图在 `EmbedStore` 中，以 `key` 查找）；`source` 为块内源码偏移。
    Embed { x: f32, y: f32, w: f32, h: f32, key: u64, source: usize },
}

pub struct BlockLayout {
    pub height: f32,
    pub items: Vec<DisplayItem>,
}

impl BlockLayout {
    pub fn estimated_bytes(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                DisplayItem::Text { layout, map, .. } => layout.estimated_bytes() + map.len() * 32,
                DisplayItem::CopyButton { text, .. } => text.len() + 64,
                DisplayItem::Embed { .. } => 64,
                _ => 64,
            })
            .sum()
    }

    /// 块内坐标命中到块内源码偏移；落在空白处时取纵向最近的文本项。
    pub fn source_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for item in &self.items {
            if let DisplayItem::Embed { x: ix, y: iy, w, h, source, .. } = item {
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h {
                    return Some(*source);
                }
                continue;
            }
            let DisplayItem::Text { x: ix, y: iy, layout, map } = item else { continue };
            let height = layout.height();
            let distance = if y < *iy { iy - y } else if y > iy + height { y - iy - height } else { 0.0 };
            if best.is_some_and(|(current, _)| current <= distance) {
                continue;
            }
            let offset = layout.offset_at(x - ix, (y - iy).clamp(0.0, height - 1.0));
            if let Some(source) = map_to_source(map, offset) {
                best = Some((distance, source));
            }
        }
        best.map(|(_, source)| source)
    }

    /// 块内源码区间在各文本项上的覆盖矩形（块坐标）。
    pub fn source_rects(&self, range: Range<usize>) -> Vec<(f32, f32, f32, f32)> {
        let mut rects = Vec::new();
        for item in &self.items {
            let DisplayItem::Text { x, y, layout, map } = item else { continue };
            for seg in map {
                let seg_end = seg.source + if seg.exact { seg.text.len() } else { 1 };
                let from = range.start.max(seg.source);
                let to = range.end.min(seg_end);
                if from >= to {
                    continue;
                }
                let text_range = if seg.exact {
                    seg.text.start + (from - seg.source)..seg.text.start + (to - seg.source)
                } else {
                    seg.text.clone()
                };
                for (line, x0, x1) in layout.range_rects(text_range) {
                    rects.push((x + x0, y + line as f32 * layout.line_height, x1 - x0, layout.line_height));
                }
            }
        }
        rects
    }
}

fn map_to_source(map: &[MapSeg], offset: usize) -> Option<usize> {
    let seg = map
        .iter()
        .find(|seg| offset >= seg.text.start && offset <= seg.text.end)
        .or_else(|| map.iter().rev().find(|seg| seg.text.end <= offset))
        .or(map.first())?;
    Some(if seg.exact { seg.source + (offset.clamp(seg.text.start, seg.text.end) - seg.text.start) } else { seg.source })
}

/// 渲染所需的外部环境：图片尺寸查询（未就绪返回 None 并由调用方排队解码）。
pub struct RenderEnv<'a> {
    pub metrics: Metrics,
    pub wrap: Wrap,
    pub base_dir: Option<&'a Path>,
    pub image_size: &'a dyn Fn(&Path) -> Option<(u32, u32)>,
    /// 宽内容（表格）可用的最大宽度：从正文左缘到编辑区右缘，允许超出正文宽度。
    pub wide_width: f32,
    /// 设置“代码块行号”。
    pub code_line_numbers: bool,
    /// 设置“渲染行内代码”：关闭时行内代码按源码（含反引号）显示。
    pub inline_code: bool,
    /// 查询公式或图表的渲染状态（返回缓存键与状态）；未请求过的条目由调用方登记后台渲染，此处视为渲染中。
    pub embed: &'a dyn Fn(EmbedKind, &str) -> (u64, EmbedState),
}

struct ParaBuilder {
    text: String,
    spans: Vec<(Range<usize>, InlineStyle)>,
    map: Vec<MapSeg>,
    objects: Vec<InlineObject>,
}

impl ParaBuilder {
    fn new() -> Self {
        Self { text: String::new(), spans: Vec::new(), map: Vec::new(), objects: Vec::new() }
    }

    /// 追加一个行内对象（占位字符），整体映射到 `source`。
    fn push_object(&mut self, object: InlineObject, style: InlineStyle, source: usize) {
        let at = self.text.len();
        self.push(&OBJECT_CHAR.to_string(), style, Some((source, false)));
        self.objects.push(InlineObject { at, ..object });
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn push(&mut self, text: &str, style: InlineStyle, source: Option<(usize, bool)>) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        match self.spans.last_mut() {
            Some((range, last)) if *last == style && range.end == start => range.end = end,
            _ => self.spans.push((start..end, style)),
        }
        if let Some((source, exact)) = source {
            self.map.push(MapSeg { text: start..end, source, exact });
        }
    }
}

#[derive(Clone, Default)]
struct InlineState {
    bold: u8,
    italic: u8,
    strike: u8,
    link: u8,
    underline: u8,
    mark: u8,
    sup: u8,
    sub: u8,
    /// `<span style>` / `<font color>` 的颜色栈：(文字调色板序号, 背景调色板序号)。
    colors: Vec<(Option<u8>, Option<u8>)>,
}

impl InlineState {
    fn style(&self, base: InlineStyle) -> InlineStyle {
        let mut style = base;
        style.bold |= self.bold > 0;
        style.italic |= self.italic > 0;
        style.strike |= self.strike > 0;
        style.underline |= self.underline > 0;
        if self.mark > 0 {
            style.bg = 1;
        }
        for (foreground, background) in &self.colors {
            if let Some(index) = foreground {
                style.color = ColorRole::Palette(*index);
            }
            if let Some(index) = background {
                style.bg = index + 2;
            }
        }
        if self.sup > 0 {
            style.shift = 1;
        } else if self.sub > 0 {
            style.shift = -1;
        }
        if self.link > 0 {
            style.color = ColorRole::Link;
            style.underline = true;
        }
        style
    }

    /// 识别常见行内 HTML 标签并调整样式计数；返回 true 表示已消费该标签，不再原样显示。
    fn apply_html(&mut self, html: &str) -> Option<bool> {
        let tag = html.trim().trim_start_matches('<').trim_end_matches('>').trim_end_matches('/').trim().to_ascii_lowercase();
        let (closing, name) = match tag.strip_prefix('/') {
            Some(name) => (true, name.trim().to_string()),
            None => (false, tag.split_whitespace().next().unwrap_or("").to_string()),
        };
        match name.as_str() {
            "span" | "font" if closing => {
                self.colors.pop();
                return Some(true);
            }
            "span" | "font" => {
                let style = html_attr(html, "style").unwrap_or_default().to_ascii_lowercase();
                let declaration = |property: &str| {
                    style.split(';').find_map(|part| {
                        let (key, value) = part.split_once(':')?;
                        (key.trim() == property).then(|| value.trim().to_string())
                    })
                };
                // 推荐写法为 class（Web 版 HTML 白名单保留 class、丢弃 style）：nomo-fg-<色名> / nomo-bg-<色名>。
                let classes = html_attr(html, "class").unwrap_or_default();
                let class_color = |prefix: &str| classes.split_whitespace().find_map(|class| class.strip_prefix(prefix)).and_then(palette_index);
                let foreground = class_color("nomo-fg-").or_else(|| declaration("color").or_else(|| html_attr(html, "color")).and_then(|value| palette_index(&value)));
                let background = class_color("nomo-bg-").or_else(|| declaration("background-color").or_else(|| declaration("background")).and_then(|value| palette_index(&value)));
                self.colors.push((foreground, background));
                return Some(true);
            }
            _ => {}
        }
        let counter = match name.as_str() {
            "b" | "strong" => &mut self.bold,
            "i" | "em" => &mut self.italic,
            "s" | "del" | "strike" => &mut self.strike,
            "u" | "ins" => &mut self.underline,
            "mark" => &mut self.mark,
            "sup" => &mut self.sup,
            "sub" => &mut self.sub,
            "br" => return Some(false),
            _ => return None,
        };
        *counter = if closing { counter.saturating_sub(1) } else { counter.saturating_add(1) };
        Some(true)
    }
}

/// 从 HTML 片段中取属性值（不处理转义，足以覆盖 `<img src="..." width="...">`）。
fn html_attr(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))?;
    let rest = &html[at + name.len() + 1..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        rest[1..].find(quote).map(|end| rest[1..1 + end].to_string())
    } else {
        Some(rest.split(|ch: char| ch.is_whitespace() || ch == '>').next().unwrap_or("").to_string())
    }
}

/// 把 CSS 颜色（#rgb、#rrggbb、rgb()、常见颜色名）归到最接近的调色板项：
/// 低饱和度为灰，其余按色相分段，与飞书的 7 色色板对应。
fn palette_index(value: &str) -> Option<u8> {
    let value = value.trim().trim_end_matches("!important").trim().to_ascii_lowercase();
    let named = match value.as_str() {
        "red" | "crimson" | "tomato" => Some(0),
        "orange" | "darkorange" | "coral" => Some(1),
        "yellow" | "gold" | "khaki" => Some(2),
        "green" | "lime" | "seagreen" | "limegreen" => Some(3),
        "blue" | "royalblue" | "dodgerblue" | "skyblue" | "lightblue" => Some(4),
        "purple" | "violet" | "magenta" | "orchid" => Some(5),
        "gray" | "grey" | "silver" | "darkgray" | "lightgray" => Some(6),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let (r, g, b) = if let Some(hex) = value.strip_prefix('#') {
        let expand = |part: &str| u8::from_str_radix(part, 16).ok();
        match hex.len() {
            3 | 4 => {
                let digit = |index: usize| expand(&hex[index..index + 1]).map(|value| value * 17);
                (digit(0)?, digit(1)?, digit(2)?)
            }
            6 | 8 => (expand(&hex[0..2])?, expand(&hex[2..4])?, expand(&hex[4..6])?),
            _ => return None,
        }
    } else if let Some(inner) = value.strip_prefix("rgb").map(|rest| rest.trim_start_matches('a')) {
        let numbers: Vec<u8> = inner.trim_matches(|ch| ch == '(' || ch == ')').split(',').take(3).filter_map(|part| part.trim().parse::<f32>().ok()).map(|value| value.clamp(0.0, 255.0) as u8).collect();
        if numbers.len() < 3 {
            return None;
        }
        (numbers[0], numbers[1], numbers[2])
    } else {
        return None;
    };
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if max < 0.08 || delta / max.max(0.001) < 0.18 {
        return Some(6);
    }
    let hue = if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Some(match hue {
        hue if !(15.0..330.0).contains(&hue) => 0,
        hue if hue < 42.0 => 1,
        hue if hue < 70.0 => 2,
        hue if hue < 170.0 => 3,
        hue if hue < 255.0 => 4,
        _ => 5,
    })
}

/// 识别紧随图片的属性块 `{align=center width=360}`（Web 版图片属性写法）。
fn parse_image_attrs(text: &str) -> Option<(Option<String>, Option<f32>)> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut align = None;
    let mut width = None;
    for part in inner.split_whitespace() {
        match part.split_once('=')? {
            ("align", value) => align = Some(value.trim_matches('"').to_string()),
            ("width", value) => width = value.trim_matches('"').trim_end_matches("px").parse().ok(),
            _ => return None,
        }
    }
    (align.is_some() || width.is_some()).then_some((align, width))
}

fn strip_tags(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => output.push(ch),
            _ => {}
        }
    }
    output
}

enum Container {
    List { next: Option<u64>, depth: usize },
    Item { marker_pending: bool, number: Option<u64>, depth: usize, task: Option<(bool, usize)> },
    Quote { start_y: f32, x: f32, role: Option<ColorRole> },
    Footnote,
}

struct Builder<'a> {
    env: &'a RenderEnv<'a>,
    window: &'a Window,
    width: f32,
    source: &'a str,
    items: Vec<DisplayItem>,
    y: f32,
    indent: f32,
    containers: Vec<Container>,
    para: ParaBuilder,
    inline: InlineState,
    base: InlineStyle,
    font_size: f32,
    line_height: f32,
    pending_gap: f32,
    heading: Option<u8>,
    images: Vec<PendingImage>,
    image_only: bool,
    in_html_block: bool,
    /// `<p align="...">` 包裹图片时的对齐方式。
    html_align: Option<String>,
}

struct PendingImage {
    url: String,
    alt: String,
    align: Option<String>,
    width: Option<f32>,
}

/// 布局一个顶层块。`source` 为块的完整源码（含块内换行）。
pub fn layout_block(source: &str, width: f32, env: &RenderEnv, window: &Window) -> BlockLayout {
    let metrics = env.metrics;
    let mut builder = Builder {
        env,
        window,
        width,
        source,
        items: Vec::new(),
        y: 0.0,
        indent: 0.0,
        containers: Vec::new(),
        para: ParaBuilder::new(),
        inline: InlineState::default(),
        base: InlineStyle::default(),
        font_size: metrics.body_size(),
        line_height: metrics.body_line_height(),
        pending_gap: 0.0,
        heading: None,
        images: Vec::new(),
        image_only: true,
        in_html_block: false,
        html_align: None,
    };
    if source.starts_with("---") && (source.ends_with("---") || source.ends_with("...")) && source.lines().count() >= 2 {
        builder.front_matter();
        return builder.finish();
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT;
    let mut events = Parser::new_ext(source, options).into_offset_iter().peekable();
    while let Some((event, range)) = events.next() {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let level = level as u8;
                    let (size, _, _) = metrics.heading(level);
                    builder.heading = Some(level);
                    builder.font_size = size;
                    builder.line_height = size * 1.35;
                    builder.base = InlineStyle { bold: true, color: ColorRole::Heading, ..builder.base_for_container() };
                }
                Tag::Paragraph => builder.begin_paragraph(),
                Tag::BlockQuote(kind) => builder.begin_quote(kind),
                Tag::CodeBlock(kind) => {
                    let lang = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_lowercase(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let mut text = String::new();
                    let mut text_start = None;
                    while let Some((event, inner)) = events.next() {
                        match event {
                            Event::Text(chunk) => {
                                text_start.get_or_insert(inner.start);
                                text.push_str(&chunk);
                            }
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    builder.code_block(&lang, &text, text_start.unwrap_or(range.start));
                }
                Tag::List(start) => {
                    builder.flush_paragraph();
                    let depth = builder.containers.iter().filter(|container| matches!(container, Container::List { .. })).count();
                    builder.containers.push(Container::List { next: start, depth });
                    if depth == 0 {
                        builder.gap(0.0);
                    }
                }
                Tag::Item => {
                    builder.flush_paragraph();
                    let (number, depth) = match builder.containers.iter_mut().rev().find(|container| matches!(container, Container::List { .. })) {
                        Some(Container::List { next, depth }) => {
                            let number = *next;
                            if let Some(value) = next {
                                *value += 1;
                            }
                            (number, *depth)
                        }
                        _ => (None, 0),
                    };
                    builder.indent += metrics.list_indent();
                    builder.containers.push(Container::Item { marker_pending: true, number, depth, task: None });
                }
                Tag::FootnoteDefinition(label) => {
                    builder.flush_paragraph();
                    builder.gap(metrics.paragraph_gap() * 0.5);
                    builder.containers.push(Container::Footnote);
                    builder.font_size = metrics.body_size() * 0.85;
                    builder.line_height = metrics.body_line_height() * 0.85;
                    let style = InlineStyle { color: ColorRole::Muted, ..Default::default() };
                    builder.para.push(&format!("{label}. "), style, Some((range.start, false)));
                }
                Tag::Table(alignments) => {
                    builder.flush_paragraph();
                    let table = collect_table(&mut events, alignments);
                    builder.table(table);
                }
                Tag::Emphasis => builder.inline.italic += 1,
                Tag::Strong => builder.inline.bold += 1,
                Tag::Strikethrough => builder.inline.strike += 1,
                Tag::Superscript => builder.inline.sup += 1,
                Tag::Subscript => builder.inline.sub += 1,
                Tag::Link { .. } => builder.inline.link += 1,
                Tag::Image { dest_url, .. } => {
                    let mut alt = String::new();
                    for (event, _) in events.by_ref() {
                        match event {
                            Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                            Event::End(TagEnd::Image) => break,
                            _ => {}
                        }
                    }
                    builder.images.push(PendingImage { url: dest_url.to_string(), alt: alt.clone(), align: None, width: None });
                    let style = InlineStyle { color: ColorRole::Link, ..Default::default() };
                    builder.para.push(&format!("［图片：{}］", if alt.is_empty() { dest_url.as_ref() } else { alt.as_str() }), style, Some((range.start, false)));
                }
                Tag::HtmlBlock => {
                    builder.begin_paragraph();
                    builder.in_html_block = true;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    builder.flush_paragraph();
                    builder.heading = None;
                    builder.font_size = metrics.body_size();
                    builder.line_height = metrics.body_line_height();
                    builder.base = builder.base_for_container();
                }
                TagEnd::Paragraph => builder.end_paragraph(),
                TagEnd::BlockQuote(_) => builder.end_quote(),
                TagEnd::List(_) => {
                    builder.flush_paragraph();
                    builder.containers.pop();
                }
                TagEnd::Item => {
                    builder.flush_paragraph();
                    builder.containers.pop();
                    builder.indent -= metrics.list_indent();
                    builder.y += 4.0 * metrics.zoom;
                }
                TagEnd::FootnoteDefinition => {
                    builder.flush_paragraph();
                    builder.containers.pop();
                    builder.font_size = metrics.body_size();
                    builder.line_height = metrics.body_line_height();
                }
                TagEnd::Emphasis => builder.inline.italic = builder.inline.italic.saturating_sub(1),
                TagEnd::Strong => builder.inline.bold = builder.inline.bold.saturating_sub(1),
                TagEnd::Strikethrough => builder.inline.strike = builder.inline.strike.saturating_sub(1),
                TagEnd::Superscript => builder.inline.sup = builder.inline.sup.saturating_sub(1),
                TagEnd::Subscript => builder.inline.sub = builder.inline.sub.saturating_sub(1),
                TagEnd::Link => builder.inline.link = builder.inline.link.saturating_sub(1),
                TagEnd::HtmlBlock => {
                    builder.flush_paragraph();
                    builder.in_html_block = false;
                    builder.gap(metrics.paragraph_gap());
                }
                _ => {}
            },
            Event::Text(text) => {
                // 图片后紧跟的属性块不显示，只用于尺寸与对齐。
                if let Some(image) = builder.images.last_mut() {
                    if let Some((align, width)) = parse_image_attrs(&text) {
                        image.align = align.or(image.align.take());
                        image.width = width.or(image.width);
                        continue;
                    }
                }
                builder.image_only &= text.trim().is_empty();
                let exact = text.len() == range.len() && source.get(range.clone()) == Some(text.as_ref());
                let style = builder.inline.style(builder.base);
                builder.para.push(&text, style, Some((range.start, exact)));
            }
            Event::Code(_) if !env.inline_code => {
                builder.image_only = false;
                let raw = &source[range.clone()];
                let style = builder.inline.style(builder.base);
                builder.para.push(raw, style, Some((range.start, true)));
            }
            Event::Code(code) => {
                builder.image_only = false;
                let slice = &source[range.clone()];
                let inner = slice.find(code.as_ref()).map_or(range.start, |at| range.start + at);
                let style = InlineStyle { mono: true, code_bg: true, color: ColorRole::InlineCode, ..builder.inline.style(builder.base) };
                builder.para.push(&code, style, Some((inner, slice.contains(code.as_ref()))));
            }
            Event::InlineMath(tex) => {
                builder.image_only = false;
                builder.inline_math(&tex, range.start + 1);
            }
            Event::DisplayMath(tex) => builder.display_math(&tex, range.start + 2),
            Event::InlineHtml(html) => match builder.inline.apply_html(&html) {
                Some(true) => {}
                Some(false) => builder.flush_paragraph_continued(),
                None => {
                    builder.image_only = false;
                    let style = InlineStyle { mono: true, color: ColorRole::Muted, ..Default::default() };
                    builder.para.push(&html, style, Some((range.start, html.len() == range.len())));
                }
            },
            Event::Html(html) => builder.html_line(&html, range.start),
            Event::FootnoteReference(label) => {
                builder.image_only = false;
                let style = InlineStyle { color: ColorRole::Link, ..builder.base };
                builder.para.push(&format!("[{label}]"), style, Some((range.start, false)));
            }
            Event::SoftBreak => {
                let left = builder.para.text.chars().last();
                let right = source[range.end..].chars().find(|ch| !ch.is_whitespace());
                if !left.is_some_and(is_cjk) && !right.is_some_and(is_cjk) {
                    builder.para.push(" ", builder.inline.style(builder.base), Some((range.start, false)));
                }
            }
            Event::HardBreak => builder.flush_paragraph_continued(),
            Event::Rule => builder.rule(),
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item { task, .. }) = builder.containers.last_mut() {
                    let at = source[range.clone()].find('[').map_or(range.start, |offset| range.start + offset);
                    *task = Some((checked, at));
                }
            }
        }
    }
    builder.flush_paragraph();
    builder.finish()
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

struct TableData {
    alignments: Vec<Alignment>,
    rows: Vec<(bool, Vec<ParaBuilder>)>,
}

fn collect_table<'a>(events: &mut impl Iterator<Item = (Event<'a>, Range<usize>)>, alignments: Vec<Alignment>) -> TableData {
    let mut table = TableData { alignments, rows: Vec::new() };
    let mut in_head = false;
    let mut inline = InlineState::default();
    for (event, range) in events.by_ref() {
        match event {
            Event::Start(Tag::TableHead) => {
                in_head = true;
                table.rows.push((true, Vec::new()));
            }
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => table.rows.push((in_head, Vec::new())),
            Event::Start(Tag::TableCell) => {
                if let Some((_, cells)) = table.rows.last_mut() {
                    cells.push(ParaBuilder::new());
                }
            }
            Event::Start(Tag::Strong) => inline.bold += 1,
            Event::End(TagEnd::Strong) => inline.bold = inline.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => inline.italic += 1,
            Event::End(TagEnd::Emphasis) => inline.italic = inline.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => inline.strike += 1,
            Event::End(TagEnd::Strikethrough) => inline.strike = inline.strike.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => inline.link += 1,
            Event::End(TagEnd::Link) => inline.link = inline.link.saturating_sub(1),
            Event::Text(text) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    let exact = text.len() == range.len();
                    cell.push(&text, inline.style(InlineStyle::default()), Some((range.start, exact)));
                }
            }
            Event::Code(code) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    let style = InlineStyle { mono: true, code_bg: true, color: ColorRole::InlineCode, ..Default::default() };
                    cell.push(&code, style, Some((range.start + 1, true)));
                }
            }
            Event::InlineMath(tex) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    let style = InlineStyle { mono: true, italic: true, color: ColorRole::Accent, ..Default::default() };
                    cell.push(&tex, style, Some((range.start + 1, true)));
                }
            }
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    table
}

impl Builder<'_> {
    fn base_for_container(&self) -> InlineStyle {
        for container in self.containers.iter().rev() {
            if let Container::Quote { role, .. } = container {
                return InlineStyle { color: role.unwrap_or(ColorRole::QuoteFg), ..Default::default() };
            }
        }
        InlineStyle::default()
    }

    fn gap(&mut self, amount: f32) {
        self.pending_gap = self.pending_gap.max(amount);
    }

    fn apply_gap(&mut self) {
        if !self.items.is_empty() {
            self.y += self.pending_gap;
        }
        self.pending_gap = 0.0;
    }

    fn begin_paragraph(&mut self) {
        self.flush_paragraph();
        let in_item = matches!(self.containers.last(), Some(Container::Item { marker_pending: false, .. }));
        if in_item {
            self.gap(self.env.metrics.paragraph_gap() * 0.5);
        }
        self.base = if self.heading.is_some() { self.base } else { self.base_for_container() };
        self.image_only = true;
        self.images.clear();
    }

    fn end_paragraph(&mut self) {
        // 只含图片的段落按块级图片渲染。
        if self.image_only && !self.images.is_empty() && self.env.base_dir.is_some() {
            let images = std::mem::take(&mut self.images);
            self.para = ParaBuilder::new();
            for image in images {
                self.image_with(&image.url, &image.alt, image.width, image.align.as_deref());
            }
        } else {
            self.flush_paragraph();
        }
        self.images.clear();
        self.gap(self.env.metrics.paragraph_gap());
    }

    fn spec(&self, para: ParaBuilder) -> (ParaSpec, Vec<MapSeg>) {
        let spec = ParaSpec {
            text: para.text,
            spans: para.spans,
            base: self.base,
            family: Family::Text,
            font_size: self.font_size,
            line_height: self.line_height,
            wrap: if self.heading.is_some() { Wrap::Greedy } else { self.env.wrap },
            objects: para.objects,
        };
        (spec, para.map)
    }

    fn flush_paragraph(&mut self) {
        if self.para.is_empty() {
            return;
        }
        self.apply_gap();
        let para = std::mem::replace(&mut self.para, ParaBuilder::new());
        let (spec, map) = self.spec(para);
        let width = (self.width - self.indent).max(40.0);
        let layout = Rc::new(layout_text(&spec, width, self.window));
        self.draw_marker();
        let height = layout.height();
        self.items.push(DisplayItem::Text { x: self.indent, y: self.y, layout, map });
        self.y += height;
    }

    /// 硬换行：结束当前视觉段，下一段紧贴（无段距），保持同一缩进与样式。
    fn flush_paragraph_continued(&mut self) {
        let gap = self.pending_gap;
        self.flush_paragraph();
        self.pending_gap = gap.min(0.0);
    }

    /// 在列表项第一段的首行左侧画项目符号、序号或任务复选框。
    fn draw_marker(&mut self) {
        let metrics = self.env.metrics;
        let indent = self.indent;
        let y = self.y;
        let line_height = self.line_height;
        let Some(Container::Item { marker_pending, number, depth, task }) = self.containers.iter_mut().rev().find(|container| matches!(container, Container::Item { .. })) else {
            return;
        };
        if !*marker_pending {
            return;
        }
        *marker_pending = false;
        if let Some((checked, source)) = *task {
            let size = 15.0 * metrics.zoom;
            let x = indent - size - 7.0 * metrics.zoom;
            let top = y + (line_height - size) / 2.0;
            self.items.push(DisplayItem::Checkbox { x, y: top, size, checked, source });
            if checked {
                let spec = ParaSpec::plain("✓".into(), InlineStyle { bold: true, color: ColorRole::OnAccent, ..Default::default() }, Family::Text, size * 0.8, size, Wrap::Greedy);
                let layout = Rc::new(layout_text(&spec, 100.0, self.window));
                let width = layout.max_line_width();
                self.items.push(DisplayItem::Text { x: x + (size - width) / 2.0, y: top, layout, map: Vec::new() });
            }
            return;
        }
        let text = match number {
            Some(number) => format!("{number}."),
            None => ["•", "◦", "▪"][*depth % 3].to_string(),
        };
        let spec = ParaSpec::plain(text, InlineStyle { color: ColorRole::Muted, ..Default::default() }, Family::Text, metrics.body_size(), line_height, Wrap::Greedy);
        let layout = Rc::new(layout_text(&spec, 200.0, self.window));
        let width = layout.max_line_width();
        let x = indent - width - 8.0 * metrics.zoom;
        self.items.push(DisplayItem::Text { x, y, layout, map: Vec::new() });
    }

    fn begin_quote(&mut self, kind: Option<BlockQuoteKind>) {
        self.flush_paragraph();
        self.apply_gap();
        let role = kind.map(|kind| match kind {
            BlockQuoteKind::Note => ColorRole::Note,
            BlockQuoteKind::Tip => ColorRole::Tip,
            BlockQuoteKind::Important => ColorRole::Important,
            BlockQuoteKind::Warning => ColorRole::Warning,
            BlockQuoteKind::Caution => ColorRole::Caution,
        });
        let x = self.indent;
        self.containers.push(Container::Quote { start_y: self.y, x, role });
        self.indent += 19.0 * self.env.metrics.zoom;
        self.y += 6.0 * self.env.metrics.zoom;
        if let (Some(kind), Some(role)) = (kind, role) {
            let title = match kind {
                BlockQuoteKind::Note => "ⓘ 提醒",
                BlockQuoteKind::Tip => "✓ 提示",
                BlockQuoteKind::Important => "！重要",
                BlockQuoteKind::Warning => "⚠ 警告",
                BlockQuoteKind::Caution => "⊘ 注意",
            };
            let spec = ParaSpec::plain(title.into(), InlineStyle { bold: true, color: role, ..Default::default() }, Family::Text, self.env.metrics.body_size() * 0.9, self.line_height, Wrap::Greedy);
            let layout = Rc::new(layout_text(&spec, self.width, self.window));
            let height = layout.height();
            self.items.push(DisplayItem::Text { x: self.indent, y: self.y, layout, map: Vec::new() });
            self.y += height;
        }
        self.pending_gap = 0.0;
    }

    fn end_quote(&mut self) {
        self.flush_paragraph();
        self.pending_gap = 0.0;
        self.y += 6.0 * self.env.metrics.zoom;
        if let Some(Container::Quote { start_y, x, role }) = self.containers.pop() {
            let paint = role.map_or(Paint::QuoteBorder, Paint::Role);
            self.items.push(DisplayItem::Rect { x, y: start_y, w: 3.0 * self.env.metrics.zoom, h: self.y - start_y, paint, radius: 1.5, border: None });
        }
        self.indent -= 19.0 * self.env.metrics.zoom;
        self.gap(self.env.metrics.paragraph_gap());
    }

    fn code_block(&mut self, lang: &str, text: &str, text_start: usize) {
        self.flush_paragraph();
        self.apply_gap();
        let mut failure = None;
        if lang == "mermaid" {
            match self.mermaid(text, text_start) {
                Ok(()) => {
                    self.gap(self.env.metrics.paragraph_gap());
                    return;
                }
                Err(message) => failure = Some(message),
            }
        }
        let metrics = self.env.metrics;
        let x = self.indent;
        let width = self.width - self.indent;
        let pad = 14.0 * metrics.zoom;
        let top = self.y;
        let header = 26.0 * metrics.zoom;
        let label = match lang {
            "" => "代码".to_string(),
            "mermaid" => format!("Mermaid · {}", failure.as_deref().unwrap_or("渲染失败")),
            other => other.to_string(),
        };
        let label_spec = ParaSpec::plain(label, InlineStyle { bold: true, color: ColorRole::Muted, ..Default::default() }, Family::Text, 12.0 * metrics.zoom, header, Wrap::Greedy);
        let label_layout = Rc::new(layout_text(&label_spec, width, self.window));
        let mut y = top + header;
        let mut lines = Vec::new();
        let mut offset = 0;
        let body = text.strip_suffix('\n').unwrap_or(text);
        // 行号槽：宽度按最大行号位数估算，行号右对齐、弱化显示。
        let line_count = body.split('\n').count();
        let gutter = if self.env.code_line_numbers { (line_count.to_string().len() as f32 * 0.62 + 1.2) * metrics.code_size() } else { 0.0 };
        for (number, line) in body.split('\n').enumerate() {
            if gutter > 0.0 {
                let label = ParaSpec::plain((number + 1).to_string(), InlineStyle { color: ColorRole::Marker, ..Default::default() }, Family::Mono, metrics.code_size(), metrics.code_line_height(), Wrap::Greedy);
                let label = Rc::new(layout_text(&label, gutter, self.window));
                let label_width = label.max_line_width();
                lines.push(DisplayItem::Text { x: x + pad + gutter - label_width - 0.8 * metrics.code_size(), y, layout: label, map: Vec::new() });
            }
            let spec = ParaSpec {
                text: line.to_string(),
                spans: highlight::code_line(line, lang),
                base: InlineStyle { color: ColorRole::CodeFg, ..Default::default() },
                family: Family::Mono,
                font_size: metrics.code_size(),
                line_height: metrics.code_line_height(),
                wrap: Wrap::Greedy,
                objects: Vec::new(),
            };
            let layout = Rc::new(layout_text(&spec, (width - pad * 2.0 - gutter).max(40.0), self.window));
            let height = layout.height();
            let map = vec![MapSeg { text: 0..line.len(), source: text_start + offset, exact: true }];
            lines.push(DisplayItem::Text { x: x + pad + gutter, y, layout, map });
            y += height;
            offset += line.len() + 1;
        }
        y += pad * 0.8;
        self.items.push(DisplayItem::Rect { x, y: top, w: width, h: y - top, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
        self.items.push(DisplayItem::Text { x: x + pad, y: top, layout: label_layout, map: Vec::new() });
        let button = 22.0 * metrics.zoom;
        self.items.push(DisplayItem::CopyButton { x: x + width - button - 6.0 * metrics.zoom, y: top + 3.0 * metrics.zoom, size: button, text: Rc::from(body) });
        self.items.extend(lines);
        self.y = y;
        self.gap(metrics.paragraph_gap());
    }

    /// 行内公式：渲染就绪时以占位字符嵌入正文并按基线对齐；高于 1.3 倍行高时等比缩小，
    /// 避免撑开行距。未就绪或失败时显示 TeX 源码（失败时用警示色）。
    fn inline_math(&mut self, tex: &str, source: usize) {
        let (key, state) = (self.env.embed)(EmbedKind::Math { display: false }, tex);
        match state {
            EmbedState::Ready(info) => {
                let limit = self.line_height * 1.3;
                let scale = if info.height > limit { limit / info.height } else { 1.0 };
                let object = InlineObject { at: 0, width: info.width * scale, ascent: (info.height - info.depth) * scale, descent: info.depth * scale, key };
                let style = self.inline.style(self.base);
                self.para.push_object(object, style, source);
            }
            state => {
                let color = if matches!(state, EmbedState::Failed(_)) { ColorRole::Caution } else { ColorRole::Accent };
                let style = InlineStyle { mono: true, italic: true, color, ..self.base };
                self.para.push(tex, style, Some((source, true)));
            }
        }
    }

    /// 块级公式：就绪时居中显示位图（过宽时等比缩小到可用宽度）；渲染中或失败时显示源码框，失败另附原因。
    fn display_math(&mut self, tex: &str, source: usize) {
        self.flush_paragraph();
        self.apply_gap();
        let metrics = self.env.metrics;
        let width = self.width - self.indent;
        let (key, state) = (self.env.embed)(EmbedKind::Math { display: true }, tex);
        if let EmbedState::Ready(info) = &state {
            let pad = 8.0 * metrics.zoom;
            let (x, w, h) = self.fit_wide(info.width, info.height);
            self.items.push(DisplayItem::Embed { x, y: self.y + pad, w, h, key, source });
            self.y += h + pad * 2.0;
            self.gap(metrics.paragraph_gap());
            return;
        }
        let spec = ParaSpec::plain(tex.trim().to_string(), InlineStyle { mono: true, italic: true, color: ColorRole::Accent, ..Default::default() }, Family::Text, metrics.body_size() * 1.05, metrics.body_line_height(), Wrap::Greedy);
        let layout = Rc::new(layout_text(&spec, width - 32.0, self.window));
        let text_width = layout.max_line_width();
        let note = match &state {
            EmbedState::Failed(message) => Some(Rc::new(self.note_layout(message, ColorRole::Caution, width - 32.0))),
            _ => None,
        };
        let note_height = note.as_ref().map_or(0.0, |note| note.height());
        let height = layout.height() + note_height + 20.0 * metrics.zoom;
        self.items.push(DisplayItem::Rect { x: self.indent, y: self.y, w: width, h: height, paint: Paint::CodeBg, radius: 8.0, border: None });
        let map = vec![MapSeg { text: 0..spec.text.len(), source, exact: false }];
        self.items.push(DisplayItem::Text { x: self.indent + (width - text_width) / 2.0, y: self.y + 10.0 * metrics.zoom, layout: layout.clone(), map });
        if let Some(note) = note {
            self.items.push(DisplayItem::Text { x: self.indent + 16.0, y: self.y + 10.0 * metrics.zoom + layout.height(), layout: note, map: Vec::new() });
        }
        self.y += height;
        self.gap(metrics.paragraph_gap());
    }

    /// 宽内容（公式、图表）的摆放：能放下时在正文宽度内居中；过宽时先等比缩小，缩放下限 0.6，
    /// 仍放不下的部分从正文左缘起向右延伸到编辑区右缘，再超出由编辑区裁剪（Web 版为横向滚动）。
    /// 返回 (x, 宽, 高)。
    fn fit_wide(&self, width: f32, height: f32) -> (f32, f32, f32) {
        let available = self.width - self.indent;
        if width <= available {
            return (self.indent + (available - width) / 2.0, width, height);
        }
        let wide = (self.env.wide_width - self.indent).max(available);
        let scale = (wide / width).clamp(0.6, 1.0);
        let (w, h) = (width * scale, height * scale);
        let x = if w <= wide { self.indent + (available - w).max(0.0) / 2.0 } else { self.indent };
        (x.min(self.indent + (wide - w).max(0.0)), w, h)
    }

    /// 小号提示文字（渲染失败原因等）。
    fn note_layout(&self, text: &str, color: ColorRole, width: f32) -> TextLayout {
        let metrics = self.env.metrics;
        let spec = ParaSpec::plain(text.to_string(), InlineStyle { color, ..Default::default() }, Family::Text, metrics.body_size() * 0.8, metrics.body_line_height() * 0.8, Wrap::Greedy);
        layout_text(&spec, width.max(40.0), self.window)
    }

    /// Mermaid 代码块：就绪时居中显示图（过宽时等比缩小）；渲染中显示占位框。
    /// 返回 Ok 表示已按图表处理；Err 携带失败原因，调用方退回代码块并在标签中显示原因。
    fn mermaid(&mut self, text: &str, source: usize) -> Result<(), String> {
        let metrics = self.env.metrics;
        let width = self.width - self.indent;
        let (key, state) = (self.env.embed)(EmbedKind::Mermaid, text);
        match state {
            EmbedState::Ready(info) => {
                let pad = 8.0 * metrics.zoom;
                let (x, w, h) = self.fit_wide(info.width, info.height);
                self.items.push(DisplayItem::Embed { x, y: self.y + pad, w, h, key, source });
                self.y += h + pad * 2.0;
                Ok(())
            }
            EmbedState::Pending => {
                let h = 120.0 * metrics.zoom;
                self.items.push(DisplayItem::Rect { x: self.indent, y: self.y, w: width, h, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
                let note = Rc::new(self.note_layout("正在渲染 Mermaid 图表…", ColorRole::Muted, width - 32.0));
                let note_width = note.max_line_width();
                self.items.push(DisplayItem::Text { x: self.indent + (width - note_width) / 2.0, y: self.y + (h - note.height()) / 2.0, layout: note, map: Vec::new() });
                self.y += h;
                Ok(())
            }
            EmbedState::Failed(message) => Err(message.to_string()),
        }
    }

    fn rule(&mut self) {
        self.flush_paragraph();
        self.apply_gap();
        let zoom = self.env.metrics.zoom;
        self.items.push(DisplayItem::Rect { x: self.indent, y: self.y + 12.0 * zoom, w: self.width - self.indent, h: 1.0, paint: Paint::Border, radius: 0.0, border: None });
        self.y += 25.0 * zoom;
        self.gap(self.env.metrics.paragraph_gap());
    }

    /// HTML 块的一行：折叠块标签只保留标题行，`<img>` 渲染为图片，其余按原样弱化显示。
    fn html_line(&mut self, html: &str, source: usize) {
        let line = html.trim_end_matches('\n');
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower == "<details>" || lower.starts_with("<details ") || lower == "</details>" {
            self.flush_paragraph();
            return;
        }
        // 目录标记由 Web 版维护，语义视图只显示其间的链接列表。
        if lower == "<!-- toc -->" || lower == "<!-- /toc -->" {
            return;
        }
        if lower.starts_with("<p") && lower.contains("align=") && !lower.contains("<img") {
            self.flush_paragraph();
            self.html_align = html_attr(trimmed, "align");
            return;
        }
        if lower == "</p>" {
            self.html_align = None;
            return;
        }
        if lower.starts_with("<summary") {
            self.flush_paragraph();
            let style = InlineStyle { bold: true, color: ColorRole::Heading, ..self.base_for_container() };
            let title = strip_tags(trimmed);
            self.para.push(&format!("▾ {}", title.trim()), style, Some((source, false)));
            self.flush_paragraph();
            self.gap(self.env.metrics.paragraph_gap() * 0.5);
            return;
        }
        if let Some(at) = lower.find("<img") {
            if let Some(src) = html_attr(&trimmed[at..], "src") {
                self.flush_paragraph();
                let alt = html_attr(&trimmed[at..], "alt").unwrap_or_default();
                let width = html_attr(&trimmed[at..], "width").and_then(|value| value.trim_end_matches("px").parse::<f32>().ok());
                let align = html_attr(trimmed, "align").or_else(|| self.html_align.clone());
                self.image_with(&src, &alt, width, align.as_deref());
                return;
            }
        }
        if trimmed.is_empty() {
            return;
        }
        let style = InlineStyle { mono: true, color: ColorRole::Muted, ..Default::default() };
        self.para.push(line, style, Some((source, true)));
        self.flush_paragraph_continued();
    }

    /// 块级图片：`max_width` 来自 width 属性（CSS 像素），`align` 为 left / center / right。
    fn image_with(&mut self, url: &str, alt: &str, max_width: Option<f32>, align: Option<&str>) {
        self.apply_gap();
        let metrics = self.env.metrics;
        let available = max_width.map_or(self.width - self.indent, |width| (width * metrics.zoom).min(self.width - self.indent));
        let local = !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:"));
        let path = self.env.base_dir.filter(|_| local).map(|dir| {
            let decoded = percent_decode(url);
            let candidate = Path::new(&decoded);
            if candidate.is_absolute() { candidate.to_path_buf() } else { dir.join(candidate) }
        });
        let size = path.as_deref().and_then(|path| (self.env.image_size)(path));
        match (path, size) {
            (Some(path), Some((w, h))) if w > 0 && h > 0 => {
                // 图片按原始像素的 1/2（视网膜屏逻辑尺寸）显示，且不超过可用宽度。
                let natural = w as f32 / 2.0 * metrics.zoom;
                let display_w = if max_width.is_some() { available } else { natural.min(available) };
                let display_h = display_w * h as f32 / w as f32;
                let free = (self.width - self.indent - display_w).max(0.0);
                let offset = match align {
                    Some("center") => free / 2.0,
                    Some("right") => free,
                    _ => 0.0,
                };
                self.items.push(DisplayItem::Image { x: self.indent + offset, y: self.y, w: display_w, h: display_h, path });
                self.y += display_h;
            }
            (path, _) => {
                let note = match path {
                    Some(path) if path.exists() => format!("正在加载图片：{alt}"),
                    Some(_) => format!("图片不存在：{url}"),
                    None => format!("远程图片未加载：{url}"),
                };
                let h = 64.0 * metrics.zoom;
                self.items.push(DisplayItem::Rect { x: self.indent, y: self.y, w: available, h, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
                let spec = ParaSpec::plain(note, InlineStyle { color: ColorRole::Muted, ..Default::default() }, Family::Text, metrics.body_size() * 0.85, h, Wrap::Greedy);
                let layout = Rc::new(layout_text(&spec, available - 24.0, self.window));
                self.items.push(DisplayItem::Text { x: self.indent + 12.0, y: self.y, layout, map: Vec::new() });
                self.y += h;
            }
        }
        self.gap(metrics.paragraph_gap());
    }

    fn front_matter(&mut self) {
        let metrics = self.env.metrics;
        let pad = 12.0 * metrics.zoom;
        let width = self.width;
        let top = self.y;
        let mut y = top + pad;
        let mut offset = 0;
        let mut texts = Vec::new();
        for line in self.source.split('\n') {
            let spec = ParaSpec::plain(line.to_string(), InlineStyle { color: if line.starts_with("---") || line.starts_with("...") { ColorRole::Marker } else { ColorRole::Muted }, ..Default::default() }, Family::Mono, metrics.code_size(), metrics.code_line_height(), Wrap::Greedy);
            let layout = Rc::new(layout_text(&spec, width - pad * 2.0, self.window));
            let height = layout.height();
            texts.push(DisplayItem::Text { x: pad, y, layout, map: vec![MapSeg { text: 0..line.len(), source: offset, exact: true }] });
            y += height;
            offset += line.len() + 1;
        }
        y += pad;
        self.items.push(DisplayItem::Rect { x: 0.0, y: top, w: width, h: y - top, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
        self.items.extend(texts);
        self.y = y;
    }

    fn table(&mut self, table: TableData) {
        self.apply_gap();
        let metrics = self.env.metrics;
        let columns = table.rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0).max(table.alignments.len());
        if columns == 0 {
            return;
        }
        // 表格可越过正文宽度延伸到编辑区右缘（Web 版为横向滚动）。
        let available = (self.env.wide_width - self.indent).max(self.width - self.indent);
        let pad_x = 11.0 * metrics.zoom;
        let pad_y = 8.0 * metrics.zoom;
        let font_size = 14.0 * metrics.zoom;
        let line_height = 14.0 * 1.55 * metrics.zoom;
        let spec_for = |cell: &ParaBuilder, header: bool| ParaSpec {
            text: cell.text.clone(),
            spans: cell.spans.clone(),
            base: InlineStyle { bold: header, color: if header { ColorRole::Heading } else { ColorRole::Fg }, ..Default::default() },
            family: Family::Text,
            font_size,
            line_height,
            wrap: Wrap::Greedy,
            objects: Vec::new(),
        };
        // 列宽：按自然宽度分配；总宽超出时按比例压缩超出部分，每列保底为自身自然宽度与 120px 的较小者，
        // 保证短表头不折行。
        let mut natural = vec![40.0f32; columns];
        for (header, cells) in &table.rows {
            for (column, cell) in cells.iter().enumerate() {
                let layout = layout_text(&spec_for(cell, *header), 10_000.0, self.window);
                natural[column] = natural[column].max(layout.max_line_width() + pad_x * 2.0 + 2.0);
            }
        }
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = if total <= available {
            natural
        } else {
            let floors: Vec<f32> = natural.iter().map(|width| width.min(120.0 * metrics.zoom)).collect();
            let floor_total: f32 = floors.iter().sum();
            let scale = (available - floor_total).max(0.0) / (total - floor_total).max(1.0);
            natural.iter().zip(&floors).map(|(width, floor)| floor + (width - floor) * scale).collect()
        };
        let table_width: f32 = widths.iter().sum();
        let top = self.y;
        let mut y = top;
        let mut cells_out = Vec::new();
        let mut row_bounds = Vec::new();
        for (header, cells) in &table.rows {
            let mut row_height = line_height;
            let mut laid = Vec::new();
            let mut x = self.indent;
            for column in 0..columns {
                let width = widths[column];
                if let Some(cell) = cells.get(column) {
                    let layout = Rc::new(layout_text(&spec_for(cell, *header), (width - pad_x * 2.0).max(20.0), self.window));
                    row_height = row_height.max(layout.height());
                    let text_width = layout.max_line_width();
                    let offset = match table.alignments.get(column) {
                        Some(Alignment::Center) => (width - pad_x * 2.0 - text_width).max(0.0) / 2.0,
                        Some(Alignment::Right) => (width - pad_x * 2.0 - text_width).max(0.0),
                        _ => 0.0,
                    };
                    laid.push((x + pad_x + offset, layout, cell.map.clone()));
                }
                x += width;
            }
            let height = row_height + pad_y * 2.0;
            row_bounds.push((*header, y, height));
            for (x, layout, map) in laid {
                cells_out.push(DisplayItem::Text { x, y: y + pad_y, layout, map });
            }
            y += height;
        }
        // 绘制顺序即压入顺序：外框底色 → 表头底色 → 分隔线 → 单元格文字。
        self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: table_width, h: y - top, paint: Paint::Surface, radius: 6.0, border: Some(Paint::TableBorder) });
        for (header, row_y, height) in &row_bounds {
            if *header {
                self.items.push(DisplayItem::Rect { x: self.indent + 1.0, y: row_y + 1.0, w: table_width - 2.0, h: height - 1.0, paint: Paint::TableHeader, radius: 5.0, border: None });
            }
            if row_y + height < y - 0.5 {
                self.items.push(DisplayItem::Rect { x: self.indent, y: row_y + height - 1.0, w: table_width, h: 1.0, paint: Paint::TableBorder, radius: 0.0, border: None });
            }
        }
        let mut x = self.indent;
        for width in &widths[..columns - 1] {
            x += width;
            self.items.push(DisplayItem::Rect { x, y: top, w: 1.0, h: y - top, paint: Paint::TableBorder, radius: 0.0, border: None });
        }
        self.items.extend(cells_out);
        self.y = y;
        self.gap(metrics.paragraph_gap());
    }

    fn finish(mut self) -> BlockLayout {
        self.flush_paragraph();
        BlockLayout { height: self.y.max(1.0), items: self.items }
    }
}

fn percent_decode(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&url[index + 1..index + 3], 16) {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| url.to_string())
}

/// 把 Markdown 转为纯文本（设置“复制时保留 Markdown 语法”关闭时用于复制）：
/// 保留文字、行内代码与公式内容，块之间以换行分隔，去掉所有标记。
pub fn plain_text(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_MATH | Options::ENABLE_GFM;
    let mut output = String::with_capacity(markdown.len());
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Text(text) | Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => output.push_str(&text),
            Event::SoftBreak => output.push(' '),
            Event::HardBreak => output.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock | TagEnd::TableRow | TagEnd::TableHead) => output.push('\n'),
            Event::End(TagEnd::TableCell) => output.push('\t'),
            _ => {}
        }
    }
    output.trim_end_matches('\n').to_string()
}

/// 源码行的语法高亮段落规格，供源码视图与语义视图的活动块共用。
pub fn source_line_spec(line: &str, context: highlight::LineContext, lang: &str, family: Family, font_size: f32, line_height: f32) -> ParaSpec {
    ParaSpec {
        text: line.to_string(),
        spans: highlight::markdown_line(line, context, lang),
        base: InlineStyle::default(),
        family,
        font_size,
        line_height,
        wrap: Wrap::Greedy,
        objects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    fn tags(source: &str) -> Vec<String> {
        let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_GFM | Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT | Options::ENABLE_MATH;
        Parser::new_ext(source, options)
            .filter_map(|event| match event {
                Event::Start(tag) => Some(format!("{tag:?}")),
                _ => None,
            })
            .collect()
    }

    /// 中文标点紧邻强调标记时仍按 CommonMark 规则识别。
    /// pulldown-cmark 0.13.5 对紧贴字母的 `H~2~O`、`x^2^` 不产生上下标事件，这里不作断言。
    #[test]
    fn cjk_emphasis_is_parsed() {
        let found = tags("普通文字，**粗体**，*斜体*，~~删除线~~，H~2~O，x^2^。");
        assert!(found.iter().any(|tag| tag.starts_with("Strong")), "{found:?}");
        assert!(found.iter().any(|tag| tag.starts_with("Strikethrough")), "{found:?}");
        assert!(found.first().is_some_and(|tag| tag == &format!("{:?}", Tag::Paragraph)));
    }
}
