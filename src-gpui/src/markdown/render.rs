//! 块级语义渲染：pulldown-cmark 事件 → 与主题无关的显示列表。
//!
//! 显示列表只含相对块左上角的几何、文本布局和颜色角色，因此可按（块文本、行宽、缩放、断行模式、
//! 图片尺寸）缓存，切换主题不需要重排。文本项附带“渲染文本区间 → 块内源码偏移”的映射，
//! 用于点击定位、选区与搜索高亮。

use super::highlight;
use crate::embeds::{EmbedKind, EmbedState};
use crate::layout::{ColorRole, InlineAsset, InlineObject, InlineStyle, OBJECT_CHAR, ParaSpec, TextLayout, Wrap, layout_text};
use crate::theme::{Family, Metrics};
use crate::nomo_mark::parse::ParsedWindow;
use crate::nomo_mark::image_properties::ImageWidth;
use gpui::Window;
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[path = "table_layout.rs"]
mod table_layout;
#[path = "code_layout.rs"]
pub(crate) mod code_layout;

/// 与表格工具条共用的布局留白；始终占位，激活表格不改变正文高度。
pub(crate) const TABLE_TOOLBAR_SPACE: f32 = 42.0;

/// 渲染文本区间到块内源码偏移的映射段；`exact` 为真时逐字节对应，否则整段对应到 `source`。
#[derive(Clone, Debug)]
pub struct MapSeg {
    pub text: Range<usize>,
    pub source: usize,
    pub source_end: usize,
    pub exact: bool,
}
impl MapSeg {
    fn mapping(&self)->crate::nomo_mark::mapping::SourceSegment{
        crate::nomo_mark::mapping::SourceSegment{display:self.text.clone(),source:self.source..self.source_end,exact:self.exact}
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paint {
    Border,
    CodeBg,
    TocCard,
    CodeBorder,
    QuoteBorder,
    TableBorder,
    TableHeader,
    Surface,
    Role(ColorRole),
    CalloutBorder(ColorRole),
}

pub enum DisplayItem {
    CodeBody { x: f32, y: f32, w: f32, h: f32, source: Range<usize>, body: Box<code_layout::CodeViewport> },
    CodeHeader { x: f32, y: f32, w: f32, h: f32, source: usize, text: Rc<str>, expanded: Option<bool> },
    /// 代码语言标签的真实命中区域；文字仍由共享显示列表绘制。
    CodeLanguage { x: f32, y: f32, w: f32, h: f32, source: usize },
    BlockControls { x: f32, y: f32, w: f32, h: f32, source: Range<usize>, kind: &'static str },
    TocLink { x: f32, y: f32, w: f32, h: f32, source: usize, target: String },
    CalloutHeader { x: f32, y: f32, size: f32, button_offset: f32, source: usize, kind: BlockQuoteKind, role: ColorRole },
    /// 当前窗口内单元格的真实边界，供控件命中和矩形选区使用；不绘制正文。
    TableCell { x: f32, y: f32, w: f32, h: f32, table: usize, row: usize, column: usize, rowspan: usize, colspan: usize, source: Range<usize> },
    /// 宽表格只在自己的矩形内横向滚动，正文与宿主纵向调度保持原位。
    Horizontal {x:f32,y:f32,w:f32,h:f32,content_width:f32,source:Range<usize>,offset:std::cell::Cell<f32>,layout:Box<BlockLayout>},
    Text { x: f32, y: f32, layout: Rc<TextLayout>, map: Vec<MapSeg> },
    Rect { x: f32, y: f32, w: f32, h: f32, paint: Paint, radius: f32, border: Option<Paint> },
    Image { x: f32, y: f32, w: f32, h: f32, path: PathBuf, source:Range<usize> },
    Checkbox { x: f32, y: f32, size: f32, checked: bool, source: usize },
    /// 代码块右上角的复制按钮；`text` 为代码正文。
    CopyButton { x: f32, y: f32, size: f32, text: Rc<str> },
    Disclosure { x:f32,y:f32,size:f32,source:usize,open:bool },
    /// 已渲染的公式或 Mermaid 图（位图在 `EmbedStore` 中，以 `key` 查找）；`source` 为块内源码偏移。
    Embed { x: f32, y: f32, w: f32, h: f32, key: u64, source: usize },
}

pub struct BlockLayout {
    pub height: f32,
    pub items: Vec<DisplayItem>,
}

impl BlockLayout {
    pub fn math_input_rect(&self) -> Option<(f32, f32, f32, f32)> {
        for item in &self.items {
            match item {
                DisplayItem::Text { x, y, layout, .. } => {
                    for (key, ox, oy, w, h) in layout.object_rects_clipped(f32::NEG_INFINITY..f32::INFINITY) {
                        if matches!(key, InlineAsset::MathInput) { return Some((x + ox, y + oy, w, h)); }
                    }
                }
                DisplayItem::Horizontal { x, y, w, offset, layout, .. } => {
                    if let Some((ix, iy, iw, ih)) = layout.math_input_rect() {
                        let left = (ix - offset.get()).max(0.0); let right = (ix + iw - offset.get()).min(*w);
                        if right > left { return Some((x + left, y + iy, right - left, ih)); }
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// 标签命中区域来自同一份行内排版，包含宽表格的滚动裁切。
    pub fn comment_rects(&self) -> Vec<(Range<usize>, f32, f32, f32, f32, f32, Range<f32>)> {
        let mut rects = Vec::new();
        for item in &self.items {
            match item {
                DisplayItem::Text { x, y, layout, map } => {
                    for (at, key, ox, oy, w, h) in layout.object_rects_with_offsets(f32::NEG_INFINITY..f32::INFINITY) {
                        if let InlineAsset::Comment { font_size, .. } = key {
                            if let Some(segment) = map.iter().find(|segment| segment.text.contains(&at)) {
                                rects.push((segment.source..segment.source_end, x + ox, y + oy, w, h, f32::from_bits(*font_size), x + ox..x + ox + w));
                            }
                        }
                    }
                }
                DisplayItem::Horizontal { x, y, w, offset, layout, .. } => {
                    for (source, ix, iy, iw, ih, em, clip) in layout.comment_rects() {
                        let left = ix - offset.get();
                        let clip = (x + clip.start - offset.get()).max(*x)..(x + clip.end - offset.get()).min(x + w);
                        if clip.start < clip.end { rects.push((source, x + left, y + iy, iw, ih, em, clip)); }
                    }
                }
                _ => {}
            }
        }
        rects
    }
    pub fn selected_cell_rects(&self, ranges: &[Range<usize>]) -> Vec<(f32, f32, f32, f32)> {
        let mut rects = Vec::new();
        for item in &self.items {
            match item {
                DisplayItem::TableCell { x, y, w, h, source, .. } if ranges.iter().any(|r| r.start <= source.start && source.end <= r.end) =>
                    rects.push((*x, *y, *w, *h)),
                DisplayItem::Horizontal { x, y, w, offset, layout, .. } => {
                    for (ix, iy, iw, ih) in layout.selected_cell_rects(ranges) {
                        let left = (ix - offset.get()).max(0.0);
                        let right = (ix + iw - offset.get()).min(*w);
                        if right > left { rects.push((x + left, y + iy, right - left, ih)); }
                    }
                }
                _ => {}
            }
        }
        rects
    }
    pub fn table_cells(&self, table: usize) -> Vec<(usize, usize, usize, usize, f32, f32, f32, f32, Range<usize>)> {
        let mut cells = Vec::new();
        for item in &self.items {
            match item {
                DisplayItem::TableCell { x, y, w, h, table: id, row, column, rowspan, colspan, source } if *id == table =>
                    cells.push((*row, *column, *rowspan, *colspan, *x, *y, *w, *h, source.clone())),
                DisplayItem::Horizontal { x, y, w, offset, layout, .. } => {
                    for (r, c, rs, cs, ix, iy, iw, ih, source) in layout.table_cells(table) {
                        let left = (ix - offset.get()).max(0.0);
                        let right = (ix + iw - offset.get()).min(*w);
                        if right > left { cells.push((r, c, rs, cs, x + left, y + iy, right - left, ih, source)); }
                    }
                }
                _ => {}
            }
        }
        cells
    }
    pub fn object_source_at(&self, x: f32, y: f32) -> Option<usize> {
        self.items.iter().find_map(|item| match item {
            DisplayItem::Horizontal { x: ix, y: iy, w, h, offset, layout, .. }
                if x >= *ix && x <= ix + w && y >= *iy && y < iy + h - 8.0 =>
                layout.object_source_at(x - ix + offset.get(), y - iy),
            DisplayItem::Image { x: ix, y: iy, w, h, source, .. }
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h => Some(source.start),
            DisplayItem::BlockControls { x: ix, y: iy, w, h, source, .. }
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h => Some(source.start),
            DisplayItem::Embed { x: ix, y: iy, w, h, source, .. }
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h => Some(*source),
            DisplayItem::Text { x: ix, y: iy, layout, map } => {
                let at = layout.object_at_point(x - ix, y - iy)?;
                map.iter().find(|segment| segment.text.contains(&at)).map(|segment| segment.source)
            }
            _ => None,
        })
    }

    pub fn horizontal_caret(&self, at: usize, unit: crate::nomo_mark::navigation::CursorUnit, forward: bool) -> Option<usize> {
        let mut best = None;
        let mut accept = |next| {
            if (forward && next > at && best.is_none_or(|old| next < old)) || (!forward && next < at && best.is_none_or(|old| next > old)) { best = Some(next); }
        };
        for item in &self.items {
            match item {
                DisplayItem::CodeBody { body, .. } => { if let Some(next) = body.code.horizontal_caret(at, unit, forward) { accept(next); } }
                DisplayItem::Horizontal { layout, .. } => { if let Some(next) = layout.horizontal_caret(at, unit, forward) { accept(next); } }
                DisplayItem::Image { source, .. } => accept(if forward { if at < source.start { source.start } else { source.end } } else if at > source.end { source.end } else { source.start }),
                DisplayItem::Embed { source, .. } => accept(*source),
                DisplayItem::Text { layout, map, .. } => {
                    let Some(first) = map.iter().map(|segment| segment.source).min() else { continue; };
                    let last = map.iter().map(|segment| segment.source_end).max().unwrap_or(first);
                    if at < first { if forward { accept(first); } continue; }
                    if at > last { if !forward { accept(last); } continue; }
                    let Some(segment) = map.iter().min_by_key(|segment| at.abs_diff(at.clamp(segment.source, segment.source_end))) else { continue; };
                    let offset = segment.mapping().to_display(at, at > segment.source && !segment.exact);
                    let next = crate::nomo_mark::navigation::boundary(&layout.text, offset, unit, forward);
                    if next != offset { if let Some(next) = map_to_source(map, next) { accept(next); } }
                }
                _ => {}
            }
        }
        best
    }

    pub fn line_boundary(&self, at: usize, end: bool) -> Option<usize> {
        let mut best = None;
        let mut distance = usize::MAX;
        for item in &self.items {
            match item {
                DisplayItem::CodeBody { source, body, .. } if source.contains(&at) => return body.code.line_boundary(at, end),
                DisplayItem::Horizontal { source, layout, .. } if source.start <= at && at <= source.end => return layout.line_boundary(at, end),
                DisplayItem::Image { source, .. } if source.start <= at && at <= source.end => return Some(if end { source.end } else { source.start }),
                DisplayItem::Text { layout, map, .. } => {
                    for segment in map {
                        let delta = at.abs_diff(at.clamp(segment.source, segment.source_end));
                        if delta >= distance { continue; }
                        let offset = segment.mapping().to_display(at, at > segment.source && !segment.exact);
                        let row = layout.line_for_offset(offset);
                        let line = &layout.lines[row];
                        if let Some(next) = map_to_source(map, if end { line.end } else { line.start }) { best = Some(next); distance = delta; }
                    }
                }
                _ => {}
            }
        }
        best
    }
    pub fn remember_text(&self, reuse: &mut crate::layout::TextLayoutReuse) {
        for item in &self.items {
            match item {
                DisplayItem::CodeBody { body, .. } => { body.code.remember_text(reuse); body.numbers.remember_text(reuse); }
                DisplayItem::Text { layout, .. } => reuse.remember(layout), DisplayItem::Horizontal { layout, .. } => layout.remember_text(reuse), _ => {}
            }
        }
    }
    pub fn estimated_bytes(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                DisplayItem::Text { layout, map, .. } => layout.estimated_bytes() + map.capacity() * std::mem::size_of::<MapSeg>(),
                DisplayItem::CodeBody { body, .. } => body.code.estimated_bytes() + body.numbers.estimated_bytes() + std::mem::size_of_val(&**body),
                DisplayItem::CodeHeader { text, .. } | DisplayItem::CopyButton { text, .. } => text.len() + 64,
                DisplayItem::TocLink { target, .. } => target.capacity() + 64,
                DisplayItem::Embed { .. } => 64,
                DisplayItem::Horizontal{layout,..}=>layout.estimated_bytes(),
                _ => 64,
            })
            .sum::<usize>() + self.items.capacity()*std::mem::size_of::<DisplayItem>()
    }

    /// 先匹配纵向最近的文本项，再比较横向距离，避免点击短行右侧时跳到其他段落。
    pub fn source_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<((f32, f32), usize)> = None;
        for item in &self.items {
            if let DisplayItem::CodeBody { x: ix, y: iy, w, h, body, .. } = item {
                let vertical = if y < *iy { iy-y } else if y > iy+h { y-iy-h } else { 0.0 };
                let horizontal = if x < *ix { ix-x } else if x > ix+w { x-ix-w } else { 0.0 };
                let distance = (vertical, horizontal);
                let (width, height) = body.viewport(*w, *h);
                let (sx, sy) = body.offset.get();
                if !best.is_some_and(|(current, _)| current <= distance) {
                    if let Some(source) = body.code.source_at((x-ix-body.gutter).clamp(0.0, width)+sx, (y-iy).clamp(0.0, height)+sy) { best = Some((distance, source)); }
                }
                continue;
            }
            if let DisplayItem::Horizontal{x:ix,y:iy,w,h,offset,layout,..}=item{
                let vertical=if y<*iy{iy-y}else if y>iy+h{y-iy-h}else{0.0};
                let horizontal=if x<*ix{ix-x}else if x>ix+w{x-ix-w}else{0.0};
                let distance=(vertical,horizontal);
                if !best.is_some_and(|(current,_)|current<=distance){if let Some(source)=layout.source_at((x-ix).clamp(0.0,*w)+offset.get(),(y-iy).clamp(0.0,*h)){best=Some((distance,source));}}
                continue;
            }
            if let DisplayItem::Image{x:ix,y:iy,w,h,source,..}=item{
                if x>=*ix&&x<=ix+w&&y>=*iy&&y<=iy+h{return Some(source.start);}continue;
            }
            if let DisplayItem::Embed { x: ix, y: iy, w, h, source, .. } = item {
                if x >= *ix && x <= ix + w && y >= *iy && y <= iy + h {
                    return Some(*source);
                }
                continue;
            }
            let DisplayItem::Text { x: ix, y: iy, layout, map } = item else { continue };
            let height = layout.height();
            let vertical = if y < *iy { iy - y } else if y > iy + height { y - iy - height } else { 0.0 };
            let right = ix + layout.max_line_width();
            let horizontal = if x < *ix { ix-x } else if x > right { x-right } else { 0.0 };
            let distance = (vertical, horizontal);
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
        self.source_rects_clipped(range, f32::NEG_INFINITY..f32::INFINITY)
    }
    pub fn source_rects_clipped(&self, range: Range<usize>, clip: Range<f32>) -> Vec<(f32, f32, f32, f32)> {
        let mut rects = Vec::new();
        if range.is_empty() { return rects; }
        for item in &self.items {
            if let DisplayItem::CodeBody { x, y, w, h, body, .. } = item {
                let (width, height) = body.viewport(*w, *h);
                if y + height < clip.start || *y > clip.end { continue; }
                let (dx, dy) = body.offset.get();
                for (sx, sy, sw, sh) in body.code.source_rects_clipped(range.clone(), (clip.start-y+dy).max(dy)..(clip.end-y+dy).min(dy+height)) {
                    let left = (sx-dx).max(0.0); let right = (sx+sw-dx).min(width);
                    let top = (sy-dy).max(0.0); let bottom = (sy+sh-dy).min(height);
                    if right > left && bottom > top { rects.push((x+body.gutter+left, y+top, right-left, bottom-top)); }
                }
                continue;
            }
            if let DisplayItem::Horizontal{x,y,w,h,offset,layout,..}=item{
                if y + h < clip.start || *y > clip.end { continue; }
                for (sx,sy,sw,sh) in layout.source_rects_clipped(range.clone(),(clip.start-y)..(clip.end-y)) {let left=(sx-offset.get()).max(0.0);let right=(sx+sw-offset.get()).min(*w);if right>left{rects.push((x+left,y+sy,right-left,sh));}}
                continue;
            }
            if let DisplayItem::Image{x,y,w,h,source,..}=item{
                if y + h < clip.start || *y > clip.end { continue; }
                if range.start<source.end&&source.start<range.end{rects.push((*x,*y,*w,*h));}continue;
            }
            if let DisplayItem::BlockControls { x, y, w, h, source, kind: "rule", .. } = item {
                if y + h >= clip.start && *y <= clip.end && range.start < source.end && source.start < range.end {
                    rects.push((*x, *y, *w, *h));
                }
                continue;
            }
            let DisplayItem::Text { x, y, layout, map } = item else { continue };
            if y + layout.height() < clip.start || *y > clip.end { continue; }
            let rows = (((clip.start-y)/layout.line_height).floor().max(0.0) as usize)..(((clip.end-y)/layout.line_height).ceil().max(0.0) as usize).saturating_add(1);
            for seg in map {
                let seg_end = seg.source_end;
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
                for (line, x0, x1) in layout.range_rects_in(text_range,rows.clone()) {
                    rects.push((x + x0, y + line as f32 * layout.line_height, x1 - x0, layout.line_height));
                }
            }
        }
        rects
    }

    /// 源码位置对应的精确光标几何；隐藏标记贴靠最近的可见边界。
    pub fn caret_at(&self, source: usize) -> Option<(f32,f32,f32)> {
        let mut best = None;
        let mut distance = usize::MAX;
        for item in &self.items {
            if let DisplayItem::CodeBody { x, y, source: range, body, .. } = item {
                if range.contains(&source) {
                    if let Some((sx, sy, h)) = body.code.caret_at(source) {
                        let (dx, dy) = body.offset.get();
                        return Some((x+body.gutter+sx-dx, y+sy-dy, h));
                    }
                }
                continue;
            }
            if let DisplayItem::Horizontal{x,y,source:range,offset,layout,..}=item{
                if range.start<=source&&source<=range.end{if let Some((sx,sy,h))=layout.caret_at(source){return Some((x+sx-offset.get(),y+sy,h));}}continue;
            }
            if let DisplayItem::Image{x,y,w,h,source:range,..}=item{
                if range.start<=source&&source<=range.end{return Some((if source==range.start{*x}else{x+w},*y,*h));}continue;
            }
            let DisplayItem::Text { x,y,layout,map } = item else { continue };
            for segment in map {
                let at = source.clamp(segment.source,segment.source_end);
                let delta = source.abs_diff(at);
                if delta >= distance { continue; }
                let byte = segment.mapping().to_display(at,at>segment.source&&!segment.exact);
                let (row,cx) = layout.caret(byte);
                best = Some((x+cx,y+row as f32*layout.line_height,layout.line_height));
                distance = delta;
            }
        }
        best
    }
}

fn map_to_source(map: &[MapSeg], offset: usize) -> Option<usize> {
    let seg = map
        .iter()
        .find(|seg| offset >= seg.text.start && offset <= seg.text.end)
        .or_else(|| map.iter().rev().find(|seg| seg.text.end <= offset))
        .or(map.first())?;
    Some(seg.mapping().to_source(offset,false))
}

/// 渲染所需的外部环境：图片尺寸查询（未就绪返回 None 并由调用方排队解码）。
pub struct RenderEnv<'a> {
    pub metrics: Metrics,
    pub wrap: Wrap,
    pub base_dir: Option<&'a Path>,
    pub image_size: &'a dyn Fn(&Path) -> Option<(u32, u32)>,
    pub image_failure: &'a dyn Fn(&Path) -> Option<&'static str>,
    pub text_layouts: Option<&'a std::cell::RefCell<crate::layout::TextLayoutReuse>>,
    /// 宽内容（表格）可用的最大宽度：从正文左缘到编辑区右缘，允许超出正文宽度。
    pub wide_width: f32,
    /// 设置“代码块行号”。
    pub code_line_numbers: bool,
    /// 设置“渲染行内代码”：关闭时行内代码按源码（含反引号）显示。
    pub inline_code: bool,
    pub block_open: Option<&'a std::collections::BTreeMap<u64, bool>>,
    pub source_base: u64,
    pub active_code: Option<usize>,
    pub comment_edit: Option<(Range<usize>, &'a str)>,
    pub math_edit: Option<(Range<usize>, &'a str)>,
    pub object_edit: Option<(Range<usize>, f32)>,
    pub empty_comment: &'a str,
    /// 查询公式或图表的渲染状态（返回缓存键与状态）；未请求过的条目由调用方登记后台渲染，此处视为渲染中。
    pub embed: &'a dyn Fn(EmbedKind, &str) -> (u64, EmbedState),
}

impl RenderEnv<'_> {
    fn layout(&self, spec: &ParaSpec, width: f32, window: &Window) -> Rc<TextLayout> {
        let resolved=spec.objects.iter().any(|object|object.width_percent.is_some()).then(||{
            let mut resolved=spec.clone();
            for object in &mut resolved.objects {if let Some(percent)=object.width_percent.take(){
                let next=width*percent/100.0;let ratio=next/object.width.max(f32::EPSILON);
                object.width=next;object.ascent*=ratio;object.descent*=ratio;
            }}
            resolved
        });
        let spec=resolved.as_ref().unwrap_or(spec);
        self.text_layouts.map_or_else(|| Rc::new(layout_text(spec, width, window)), |reuse| reuse.borrow_mut().layout(spec, width, window))
    }
}

struct ParaBuilder {
    text: String,
    spans: Vec<(Range<usize>, InlineStyle)>,
    map: Vec<MapSeg>,
    objects: Vec<InlineObject>,
    table_images: Vec<table_layout::ImageMetric>,
}

impl ParaBuilder {
    fn new() -> Self {
        Self { text: String::new(), spans: Vec::new(), map: Vec::new(), objects: Vec::new(), table_images: Vec::new() }
    }

    /// 追加一个行内对象（占位字符），整体映射到 `source`。
    fn push_object(&mut self, object: InlineObject, style: InlineStyle, source: usize) {
        let at = self.text.len();
        self.push(&OBJECT_CHAR.to_string(), style, Some((source, false)));
        self.objects.push(InlineObject { at, ..object });
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.map.is_empty()
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
            self.map.push(MapSeg { text: start..end, source, source_end: source + if exact { text.len() } else { 1 }, exact });
        }
    }
}

#[derive(Clone, Default)]
struct InlineState {
    bold: u8,
    code: u8,
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
        if self.code > 0 { style.mono = true; style.code_bg = true; style.color = ColorRole::InlineCode; }
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
            "a" => &mut self.link,
            "code" => &mut self.code,
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

/// 属性解析与结构模型共用同一套带边界、引号及实体处理的规则。
fn html_attr(html: &str, name: &str) -> Option<String> {
    let start = html.len() - html.trim_start().len();
    let tag = crate::nomo_mark::nomo_syntax::html_tag(html, start)?;
    tag.attrs.get(name).map(|value| html_escape::decode_html_entities(value).into_owned())
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
fn parse_image_attrs(text: &str) -> Option<(Option<String>, Option<ImageWidth>)> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut align = None;
    let mut width = None;
    for part in inner.split_whitespace() {
        match part.split_once('=')? {
            ("align", value) => align = Some(value.trim_matches('"').to_string()),
            ("width", value) => width = ImageWidth::parse(value.trim_matches('"')),
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
    Quote { start_y: f32, x: f32, width: f32, role: Option<ColorRole> },
    Footnote { indent: f32, min_bottom: f32 },
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
    controlled: bool,
    details:Vec<usize>,
    details_frames: Vec<(usize, f32)>,
    closed_details:std::collections::HashSet<usize>,
}

struct PendingImage {
    url: String,
    alt: String,
    align: Option<String>,
    width: Option<ImageWidth>,
    source:Range<usize>,
}

/// 布局一个顶层块。`source` 为块的完整源码（含块内换行）。
pub fn layout_block(source: &str, width: f32, env: &RenderEnv, window: &Window) -> BlockLayout {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS |
        Options::ENABLE_FOOTNOTES | Options::ENABLE_MATH | Options::ENABLE_GFM |
        Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT;
    layout_events(source, width, env, window, Parser::new_ext(source, options).into_offset_iter(), &[],None,&[],&[],&[])
}

/// 控件入口：解析已在适配层完成，此处不创建解析器或读取正文。
pub fn layout_parsed(parsed: &ParsedWindow, revealed: &[Range<usize>], details_open:&std::collections::BTreeMap<u64,bool>,width: f32, env: &RenderEnv, window: &Window) -> BlockLayout {
    let closed:Vec<_>=parsed.details.iter().filter(|block|!details_open.get(&(parsed.identity.from_byte+block.source.start as u64)).copied().unwrap_or(block.default_open)).collect();
    let starts:Vec<_>=closed.iter().map(|block|block.source.start).collect();
    let events=parsed.with_blank_paragraphs(parsed.display_events(revealed)).into_iter().filter(|(_,range)|!closed.iter().any(|block|block.body.start<=range.start&&range.end<=block.body.end));
    layout_events(&parsed.source, width, env, window, events, revealed,Some(&parsed.context),&starts,&parsed.html_tables,&parsed.html_fragments)
}

fn layout_events<'a>(source: &str, width: f32, env: &RenderEnv, window: &Window,
    events: impl Iterator<Item=(Event<'a>, Range<usize>)>, revealed: &[Range<usize>],context:Option<&crate::nomo_mark::protocol::ParseContext>,closed_details:&[usize],html_tables:&[crate::nomo_mark::html_table::HtmlTable],html_fragments:&[crate::nomo_mark::html_table::HtmlFragment]) -> BlockLayout {
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
        controlled: context.is_some(),
        details:Vec::new(),details_frames:Vec::new(),closed_details:closed_details.iter().copied().collect(),
    };
    if let Some(context)=context{
        use crate::nomo_mark::protocol::AncestorContainer;
        for ancestor in &context.ancestors{match ancestor{
            AncestorContainer::Quote=>builder.begin_quote(None, 0),
            AncestorContainer::ListItem{number}=>{
                builder.indent+=metrics.list_indent();builder.containers.push(Container::Item{marker_pending:false,number:*number,depth:0,task:None});
            },
            AncestorContainer::Details{..}=>builder.indent+=16.0*metrics.zoom,
        }}
    }
    if context.is_none()&&source.starts_with("---") && (source.ends_with("---") || source.ends_with("...")) && source.lines().count() >= 2 {
        builder.front_matter(0..source.len());
        return builder.finish(context.is_some_and(|context| context.has_following_block));
    }
    let mut events = events.peekable();
    while let Some((event, range)) = events.next() {
        if let Some((edited, height)) = env.object_edit.as_ref().filter(|(edited, _)| range.start == edited.start && range.end >= edited.end) {
            builder.flush_paragraph(); builder.apply_gap();
            builder.items.push(DisplayItem::BlockControls { x: builder.indent, y: builder.y, w: builder.width - builder.indent,
                h: *height, source: edited.clone(), kind: "object-edit" });
            builder.y += height; builder.gap(metrics.paragraph_gap());
            while events.peek().is_some_and(|(_, span)| span.start < edited.end) { events.next(); }
            continue;
        }
        if builder.controlled && matches!(&event, Event::Html(html) if html.trim_start().starts_with("<!-- toc -->")) {
            if let Some(end) = source[range.start..].find("<!-- /toc -->") {
                let end = range.start + end + "<!-- /toc -->".len();
                builder.toc(range.start..end);
                while events.peek().is_some_and(|(_, range)| range.start < end) { events.next(); }
                builder.in_html_block = false;
                continue;
            }
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let level = level as u8;
                    if builder.controlled&&!builder.items.is_empty(){builder.gap(metrics.heading(level).1);}
                    let (size, _, _) = metrics.heading(level);
                    builder.heading = Some(level);
                    builder.font_size = size;
                    builder.line_height = size * 1.35;
                    builder.base = InlineStyle { bold: true, color: ColorRole::Heading, ..builder.base_for_container() };
                    if builder.controlled && source[range.clone()].trim_matches(['#',' ','\t','\r','\n']).is_empty() {
                        let at=range.start+source[range.clone()].trim_end_matches(['\r','\n']).len();
                        builder.empty_anchor(at);
                    }
                }
                Tag::Paragraph => { builder.begin_paragraph(); if builder.controlled && range.is_empty() { builder.empty_anchor(range.start); } },
                Tag::BlockQuote(kind) => builder.begin_quote(kind, range.start),
                Tag::CodeBlock(kind) => {
                    let lang = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_lowercase(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let mut text = String::new();
                    let mut text_start = None;
                    let mut text_map=Vec::new();
                    while let Some((event, inner)) = events.next() {
                        match event {
                            Event::Text(chunk) => {
                                text_start.get_or_insert(inner.start);
                                for segment in crate::nomo_mark::mapping::text_segments(&chunk,&source[inner.clone()],inner.start) {
                                    text_map.push(MapSeg{text:text.len()+segment.display.start..text.len()+segment.display.end,source:segment.source.start,source_end:segment.source.end,exact:segment.exact});
                                }
                                text.push_str(&chunk);
                            }
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    let mut empty_start = source[range.clone()].find('\n').map_or(range.start, |end| range.start + end + 1);
                    if text.is_empty() {
                        let tail = &source[empty_start..range.end];
                        empty_start += tail.len() - tail.trim_start_matches([' ', '\t', '>']).len();
                    }
                    builder.code_block(&lang, &text, text_start.unwrap_or(empty_start),&text_map, range.start);
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
                    let raw=source[range.clone()].trim_end_matches(['\r','\n']);
                    if builder.controlled && crate::nomo_mark::commands::structural_prefix_len(raw)==Some(raw.len()) {
                        builder.empty_anchor(range.start+raw.len());
                    }
                }
                Tag::FootnoteDefinition(label) => {
                    builder.flush_paragraph();
                    builder.gap(metrics.paragraph_gap() * 0.5);
                    builder.apply_gap();
                    let zoom = metrics.zoom;
                    builder.items.push(DisplayItem::Rect { x: builder.indent, y: builder.y, w: builder.width - builder.indent, h: 1.0,
                        paint: Paint::Border, radius: 0.0, border: None });
                    builder.y += 10.0 * zoom;
                    let spec = ParaSpec::plain(label.to_string(), InlineStyle { color: ColorRole::Link, bold: true, ..Default::default() },
                        Family::Text, metrics.body_size() * 0.75, metrics.body_line_height() * 0.9, Wrap::Greedy);
                    let badge = env.layout(&spec, (builder.width * 0.2).max(24.0 * zoom), window);
                    let w = (badge.max_line_width() + 8.0 * zoom).max(24.0 * zoom); let h = badge.height();
                    builder.items.push(DisplayItem::Rect { x: builder.indent, y: builder.y, w, h, paint: Paint::Surface, radius: 4.0 * zoom, border: Some(Paint::Border) });
                    builder.items.push(DisplayItem::Text { x: builder.indent + 4.0 * zoom, y: builder.y, layout: badge,
                        map: vec![MapSeg { text: 0..label.len(), source: range.start, source_end: range.start, exact: false }] });
                    builder.items.push(DisplayItem::TocLink { x: builder.indent, y: builder.y, w, h, source: range.start, target: format!("#fnref-{}", label.replace('%', "%25")) });
                    builder.containers.push(Container::Footnote { indent: w + 10.0 * zoom, min_bottom: builder.y + h });
                    builder.indent += w + 10.0 * zoom;
                    builder.font_size = metrics.body_size() * 0.9;
                    builder.line_height = metrics.body_line_height() * 0.9;
                }
                Tag::Table(alignments) => {
                    builder.flush_paragraph();
                    let table = collect_table(&mut events, alignments, source,range,env,context,window);
                    builder.table(table, revealed);
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
                    if !context.is_some_and(|context| context.reference_target_pending(&range)) {
                        builder.images.push(PendingImage { url: dest_url.to_string(), alt: alt.clone(), align: None, width: None, source:range.clone() });
                    }
                    let style = InlineStyle { color: ColorRole::Link, ..Default::default() };
                    builder.para.push(&format!("［图片：{}］", if alt.is_empty() { dest_url.as_ref() } else { alt.as_str() }), style, Some((range.start, false)));
                }
                Tag::HtmlBlock => {
                    let raw = &source[range.clone()];
                    if builder.controlled && ((range.start == 0 && env.source_base == 0 && crate::nomo_mark::nomo_syntax::front_matter_end(raw).is_some())
                        || (raw.trim().starts_with("<!--") && raw.trim().ends_with("-->") && raw.contains('\n') && !raw.trim_start().starts_with("<!-- toc -->"))) {
                        builder.html_line(raw, range.start, None);
                        while let Some((event, _)) = events.next() { if matches!(event, Event::End(TagEnd::HtmlBlock)) { break; } }
                        continue;
                    }
                    let fragments: Vec<_> = html_fragments.iter().filter(|fragment| range.start <= fragment.source.start && fragment.source.end <= range.end).collect();
                    let mut previous = range.start;
                    let complete = !fragments.is_empty() && fragments.iter().all(|fragment| {
                        let adjacent = previous <= fragment.source.start && source[previous..fragment.source.start].trim().is_empty();
                        previous = fragment.source.end; adjacent
                    }) && source[previous..range.end].trim().is_empty();
                    if complete {
                        for fragment in fragments { builder.html_fragment(fragment); }
                        while let Some((event, _)) = events.next() { if matches!(event, Event::End(TagEnd::HtmlBlock)) { break; } }
                        continue;
                    }
                    if let Some(table)=html_tables.iter().find(|table|range.start<=table.source.start&&table.source.end<=range.end&&source[range.start..table.source.start].trim().is_empty()) {
                        builder.html_table(table,revealed);
                        while let Some((event,_))=events.next(){if matches!(event,Event::End(TagEnd::HtmlBlock)){break;}}
                        continue;
                    }
                    builder.begin_paragraph();
                    builder.in_html_block = true;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(level) => {
                    builder.flush_paragraph();
                    if builder.controlled {builder.gap(metrics.heading(level as u8).2);}
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
                    if let Some(Container::Footnote { indent, min_bottom }) = builder.containers.pop() { builder.indent -= indent; builder.y = builder.y.max(min_bottom); }
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
                if builder.controlled && text.is_empty() && range.is_empty() { builder.empty_anchor(range.start); }
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
                let mut style = builder.inline.style(builder.base);
                if revealed.contains(&range) { style.color = ColorRole::Muted; }
                builder.para.push(&text, style, Some((range.start, exact)));
                if let Some(map) = builder.para.map.last_mut() { map.source_end = range.end; }
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
                if !push_math_input(&mut builder.para, &range, builder.font_size, builder.width - builder.indent, env) {
                    builder.inline_math(&tex, range.start + 1);
                    if let Some(map) = builder.para.map.last_mut() { map.source_end = range.end; }
                }
            }
            Event::DisplayMath(tex) => builder.display_math(&tex, range.start + 2),
            Event::InlineHtml(html) if builder.controlled && crate::nomo_mark::comments::content(&html).is_some() => {
                builder.image_only = false;
                let content = crate::nomo_mark::comments::content(&html).unwrap();
                push_comment(&mut builder.para, &content, range, builder.font_size, env, window);
            }
            Event::InlineHtml(html) => {
                if let Some(run) = builder.controlled.then(|| crate::nomo_mark::comments::span_at(source, range.start)).flatten() {
                    if let Some(crate::nomo_mark::html_table::InlineAtom::Comment(content)) = run.atom {
                        builder.image_only = false;
                        push_comment(&mut builder.para, &content, run.source.clone(), builder.font_size, env, window);
                        while events.peek().is_some_and(|(event, range)| !matches!(event, Event::Start(_) | Event::End(_))
                            && run.source.start <= range.start && range.end <= run.source.end) { events.next(); }
                        continue;
                    }
                }
                match builder.inline.apply_html(&html) {
                Some(true) => {}
                Some(false) => builder.line_break(range),
                None => {
                    builder.image_only = false;
                    let style = InlineStyle { mono: true, color: ColorRole::Muted, ..Default::default() };
                    builder.para.push(&html, style, Some((range.start, html.len() == range.len())));
                }
                }
            },
            Event::Html(html) => builder.html_line(&html, range.start, html_fragments.iter().find(|fragment| fragment.source.start == range.start)),
            Event::FootnoteReference(label) => {
                builder.image_only = false;
                let style = InlineStyle { color: ColorRole::Link, shift: 1, footnote: true, ..builder.base };
                builder.para.push(&label, style, Some((range.start, false)));
                if let Some(map) = builder.para.map.last_mut() { map.source_end = range.end; }
            }
            Event::SoftBreak => {
                if env.wrap != Wrap::KnuthPlass {
                    builder.line_break(range);
                    continue;
                }
                let left = builder.para.text.chars().last();
                let right = source[range.end..].chars().find(|ch| !ch.is_whitespace());
                if !left.is_some_and(is_cjk) && !right.is_some_and(is_cjk) {
                    builder.para.push(" ", builder.inline.style(builder.base), Some((range.start, false)));
                }
            }
            Event::HardBreak => builder.line_break(range),
            Event::Rule => builder.rule(range),
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item { task, .. }) = builder.containers.last_mut() {
                    let at = source[range.clone()].find('[').map_or(range.start, |offset| range.start + offset);
                    *task = Some((checked, at));
                }
            }
        }
    }
    builder.flush_paragraph();
    if let Some(context)=context{
        use crate::nomo_mark::protocol::AncestorContainer;
        for ancestor in context.ancestors.iter().rev(){match ancestor{
            AncestorContainer::Quote=>builder.end_quote(),
            AncestorContainer::ListItem{..}=>{builder.containers.pop();builder.indent-=metrics.list_indent();},
            AncestorContainer::Details{..}=>builder.indent-=16.0*metrics.zoom,
        }}
    }
    builder.finish(context.is_some_and(|context| context.has_following_block))
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

struct TableData {
    source:Range<usize>,
    alignments: Vec<Alignment>,
    rows: Vec<(bool, Vec<ParaBuilder>)>,
}

fn push_comment(para: &mut ParaBuilder, content: &str, range: Range<usize>, font_size: f32, env: &RenderEnv, window: &Window) {
    let edit = env.comment_edit.as_ref().filter(|(target, _)| *target == range);
    let content = edit.map_or(content, |(_, text)| *text);
    let object = crate::nomo_mark::comments::object(content, env.empty_comment, edit.is_some(), font_size, env.metrics.zoom, window);
    para.push_object(object, InlineStyle::default(), range.start);
    if let Some(map) = para.map.last_mut() { map.source_end = range.end; }
}

fn push_math_input(para: &mut ParaBuilder, range: &Range<usize>, em: f32, available: f32, env: &RenderEnv) -> bool {
    let Some((_, text)) = env.math_edit.as_ref().filter(|(target, _)| target == range) else { return false; };
    let width = (text.chars().take(64).count() as f32 * em * 0.65 + 20.0 * env.metrics.zoom)
        .max(72.0 * env.metrics.zoom).min(available.max(32.0)).min(480.0 * env.metrics.zoom);
    para.push_object(InlineObject { at: 0, width, width_percent: None, ascent: em * 1.2, descent: em * 0.35, key: InlineAsset::MathInput }, InlineStyle::default(), range.start);
    if let Some(map) = para.map.last_mut() { map.source_end = range.end; }
    true
}

fn push_table_atom(para: &mut ParaBuilder, atom: &crate::nomo_mark::html_table::InlineAtom, range: Range<usize>, mut style: InlineStyle, env: &RenderEnv, window: &Window) {
    use crate::nomo_mark::html_table::InlineAtom;
    if matches!(atom, InlineAtom::Math(_)) && push_math_input(para, &range, 14.0 * env.metrics.zoom, 320.0 * env.metrics.zoom, env) { return; }
    if let InlineAtom::Comment(content) = atom {
        push_comment(para, content, range, 14.0 * env.metrics.zoom, env, window);
        return;
    }
    let start = para.text.len();
    let mut fallback = atom.display_text();
    let object = match atom {
        InlineAtom::Math(tex) => {
            let (key, state) = (env.embed)(EmbedKind::Math { display: false }, tex);
            if let EmbedState::Ready(info) = state {
                let scale = (14.0 * 1.55 * env.metrics.zoom * 1.3 / info.height).min(1.0);
                Some(InlineObject { at: 0, width: info.width * scale, width_percent:None, ascent: (info.height - info.depth) * scale, descent: info.depth * scale, key: InlineAsset::Embed(key) })
            } else { style.color = ColorRole::Accent; None }
        }
        InlineAtom::Image { src, width, .. } => {
            let path = crate::images::resolve_source(src, env.base_dir);
            path.and_then(|path| {
                let size = (env.image_size)(&path);
                if let Some(reason) = (env.image_failure)(&path) { fallback = format!("{reason}：{fallback}"); }
                size.map(|(w, h)| (path, w, h))
            }).filter(|(_, w, h)| *w > 0 && *h > 0).map(|(path, w, h)| {
                // 百分比在最终单元格宽度确定后解析；先保留非零的固有比例。
                let display_w = match *width {Some(ImageWidth::Pixels(value))=>value*env.metrics.zoom,_=>w as f32*env.metrics.zoom};
                let width_percent=match width {Some(ImageWidth::Percent(value))=>Some(*value),_=>None};
                InlineObject { at: 0, width: display_w, width_percent, ascent: display_w * h as f32 / w as f32, descent: 0.0, key: InlineAsset::Image(path) }
            })
        }
        InlineAtom::Comment(_) => { style.color = ColorRole::Muted; style.bg = 1; None }
        InlineAtom::Footnote(id) => { style.color = ColorRole::Link; style.shift = 1; style.footnote = true; fallback = id.clone(); None }
    };
    if let Some(object) = object { para.push_object(object, style, range.start); }
    else { para.push(&fallback, style, Some((range.start, false))); }
    if let InlineAtom::Image { width, align, .. } = atom {
        para.table_images.push(table_layout::ImageMetric {
            text: start..para.text.len(),
            width: match width { Some(ImageWidth::Pixels(value)) => Some(value * env.metrics.zoom), _ => None },
            align: match align.as_deref() { Some("center") => Some(Alignment::Center), Some("right") => Some(Alignment::Right), Some("left") => Some(Alignment::Left), _ => None },
        });
    }
    if let Some(map) = para.map.last_mut() { map.source_end = range.end; }
}

fn collect_table<'a>(events: &mut impl Iterator<Item = (Event<'a>, Range<usize>)>, alignments: Vec<Alignment>, source:&str,range:Range<usize>,env:&RenderEnv,context:Option<&crate::nomo_mark::protocol::ParseContext>,window:&Window) -> TableData {
    use crate::nomo_mark::{html_table::{self, InlineAtom}, table_inline};
    let mut table = TableData { source:range,alignments, rows: Vec::new() };
    let mut in_head = false;
    let mut inline = InlineState::default();
    let mut skip = None;
    let mut cell_range: Option<Range<usize>> = None;
    while let Some((event, range)) = events.next() {
        let Some((event, range)) = table_inline::after_atom(source, event, range, &skip) else { continue; };
        match event {
            Event::Start(Tag::TableHead) => {
                in_head = true;
                table.rows.push((true, Vec::new()));
            }
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => table.rows.push((in_head, Vec::new())),
            Event::Start(Tag::TableCell) => {
                inline = InlineState::default();
                cell_range = Some(range.clone());
                if let Some((_, cells)) = table.rows.last_mut() {
                    let mut cell=ParaBuilder::new();
                    cell.map.push(MapSeg{text:0..0,source:range.start,source_end:range.start,exact:true});
                    cells.push(cell);
                }
            }
            Event::End(TagEnd::TableCell) => cell_range = None,
            Event::Start(Tag::Strong) => inline.bold += 1,
            Event::End(TagEnd::Strong) => inline.bold = inline.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => inline.italic += 1,
            Event::End(TagEnd::Emphasis) => inline.italic = inline.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => inline.strike += 1,
            Event::End(TagEnd::Strikethrough) => inline.strike = inline.strike.saturating_sub(1),
            Event::Start(Tag::Superscript) => inline.sup += 1,
            Event::End(TagEnd::Superscript) => inline.sup=inline.sup.saturating_sub(1),
            Event::Start(Tag::Subscript) => inline.sub += 1,
            Event::End(TagEnd::Subscript) => inline.sub=inline.sub.saturating_sub(1),
            Event::InlineHtml(html) => {
                if let Some(run) = html_table::atom_at(source, range.start).filter(|run| cell_range.as_ref().is_some_and(|cell| run.source.end <= cell.end)) {
                    if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                        push_table_atom(cell, run.atom.as_ref().unwrap(), run.source.clone(), inline.style(InlineStyle::default()), env, window);
                    }
                    skip = Some(run.source);
                } else {
                    match inline.apply_html(&html) {
                        Some(true) => {}
                        Some(false) => {
                            if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) { cell.push("\n", inline.style(InlineStyle::default()), Some((range.start, false))); if let Some(map)=cell.map.last_mut(){map.source_end=range.end;} }
                        }
                        None => {
                            if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) { cell.push(&html, InlineStyle { mono: true, color: ColorRole::Muted, ..inline.style(InlineStyle::default()) }, Some((range.start, true))); }
                        }
                    }
                }
            },
            Event::Start(Tag::Link { .. }) => inline.link += 1,
            Event::End(TagEnd::Link) => inline.link = inline.link.saturating_sub(1),
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let pending = context.is_some_and(|context| context.reference_target_pending(&range));
                let (image, range) = table_inline::image(source, events, &dest_url, &title, range);
                let atom = InlineAtom::Image { src: image.src, alt: image.alt, width: ImageWidth::parse(&image.width), align: Some(image.align) };
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    if pending {
                        cell.push(&atom.display_text(), inline.style(InlineStyle::default()), Some((range.start, false)));
                    } else {
                        push_table_atom(cell, &atom, range.clone(), inline.style(InlineStyle::default()), env, window);
                    }
                }
                skip = Some(range);
            }
            Event::Text(text) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    let exact = source.get(range.clone())==Some(text.as_ref());
                    cell.push(&text, inline.style(InlineStyle::default()), Some((range.start, exact)));
                    if let Some(map)=cell.map.last_mut(){map.source_end=range.end;}
                }
            }
            Event::Code(code) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    let style = InlineStyle { mono: true, code_bg: true, color: ColorRole::InlineCode, ..inline.style(InlineStyle::default()) };
                    let inner=source[range.clone()].find(code.as_ref()).map(|offset|range.start+offset);
                    cell.push(&code, style, Some((inner.unwrap_or(range.start), inner.is_some())));
                    if inner.is_none(){if let Some(map)=cell.map.last_mut(){map.source_end=range.end;}}
                }
            }
            Event::InlineMath(tex) | Event::DisplayMath(tex) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) {
                    push_table_atom(cell, &InlineAtom::Math(tex.to_string()), range, inline.style(InlineStyle::default()), env, window);
                }
            }
            Event::FootnoteReference(id) => {
                if let Some(cell) = table.rows.last_mut().and_then(|(_, cells)| cells.last_mut()) { push_table_atom(cell, &InlineAtom::Footnote(id.to_string()), range, inline.style(InlineStyle::default()), env, window); }
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

    fn apply_table_gap(&mut self) {
        if self.controlled {
            // 窗口首块也要保留工具条空间，不能依赖前一段落或另一窗口的段距。
            self.y += self.pending_gap.max(TABLE_TOOLBAR_SPACE * self.env.metrics.zoom);
            self.pending_gap = 0.0;
        } else {
            self.apply_gap();
        }
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
                self.image_with(&image.url, &image.alt, image.width, image.align.as_deref(),image.source);
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

    fn empty_anchor(&mut self, at:usize) {
        self.para.map.push(MapSeg{text:self.para.text.len()..self.para.text.len(),source:at,source_end:at,exact:true});
    }
    fn flush_paragraph(&mut self) {
        if self.para.is_empty() {
            return;
        }
        self.apply_gap();
        let para = std::mem::replace(&mut self.para, ParaBuilder::new());
        let (spec, map) = self.spec(para);
        let width = (self.width - self.indent).max(40.0);
        let layout = table_layout::layout(&spec, width, self.env, self.window);
        self.draw_marker();
        let height = layout.height();
        let free = (width - layout.max_line_width()).max(0.0);
        let offset = match self.html_align.as_deref() { Some("center") => free / 2.0, Some("right") => free, _ => 0.0 };
        self.items.push(DisplayItem::Text { x: self.indent + offset, y: self.y, layout, map });
        self.y += height;
    }

    /// 硬换行：结束当前视觉段，下一段紧贴（无段距），保持同一缩进与样式。
    fn line_break(&mut self, source: Range<usize>) {
        if self.controlled {
            self.image_only = false;
            self.empty_anchor(source.start);
        }
        self.flush_paragraph_continued();
        if self.controlled {
            // 即使尚未输入文字，也保留下一行的高度、命中和光标位置。
            self.empty_anchor(source.end);
        }
    }

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
                let layout = self.env.layout(&spec, 100.0, self.window);
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
        let layout = self.env.layout(&spec, 200.0, self.window);
        let width = layout.max_line_width();
        let x = indent - width - 8.0 * metrics.zoom;
        self.items.push(DisplayItem::Text { x, y, layout, map: Vec::new() });
    }

    fn begin_quote(&mut self, kind: Option<BlockQuoteKind>, source: usize) {
        self.flush_paragraph();
        let zoom = self.env.metrics.zoom;
        self.gap(18.0 * zoom);
        self.apply_gap();
        let role = kind.map(|kind| match kind {
            BlockQuoteKind::Note => ColorRole::Note,
            BlockQuoteKind::Tip => ColorRole::Tip,
            BlockQuoteKind::Important => ColorRole::Important,
            BlockQuoteKind::Warning => ColorRole::Warning,
            BlockQuoteKind::Caution => ColorRole::Caution,
        });
        let x = self.indent;
        self.containers.push(Container::Quote { start_y: self.y, x, width: self.width, role });
        self.indent += if kind.is_some() { 12.0 * zoom } else { 16.0 * zoom };
        self.y += 8.0 * zoom;
        if let (Some(kind), Some(role)) = (kind, role) {
            let title = match kind {
                BlockQuoteKind::Note => "提醒",
                BlockQuoteKind::Tip => "建议",
                BlockQuoteKind::Important => "重要",
                BlockQuoteKind::Warning => "警告",
                BlockQuoteKind::Caution => "风险",
            };
            let header_height = (14.0 * self.env.metrics.line_height).max(24.0) * zoom;
            let spec = ParaSpec::plain(title.into(), InlineStyle { bold: true, color: role, ..Default::default() }, Family::Text, 14.0 * zoom, header_height, Wrap::Greedy);
            let layout = self.env.layout(&spec, self.width, self.window);
            let height = layout.height();
            let button_offset = 28.0 * zoom + layout.max_line_width();
            self.items.push(DisplayItem::CalloutHeader { x: self.indent, y: self.y + (height - 16.0 * zoom) / 2.0, size: 16.0 * zoom, button_offset, source, kind, role });
            self.items.push(DisplayItem::Text { x: self.indent + 22.0 * zoom, y: self.y, layout, map: Vec::new() });
            // 原版 header 下内边距 7px，正文上内边距 11px，正文右侧保留 16px。
            self.y += height + 18.0 * zoom;
            self.width -= 16.0 * zoom;
        }
        self.pending_gap = 0.0;
    }

    fn end_quote(&mut self) {
        self.flush_paragraph();
        self.pending_gap = 0.0;
        if let Some(Container::Quote { start_y, x, width, role }) = self.containers.pop() {
            self.y += if role.is_some() { 13.0 * self.env.metrics.zoom } else { 8.0 * self.env.metrics.zoom };
            let paint = role.map_or(Paint::QuoteBorder, Paint::CalloutBorder);
            let inset = 8.0 * self.env.metrics.zoom;
            self.items.push(DisplayItem::Rect { x, y: start_y + inset, w: 4.0 * self.env.metrics.zoom, h: (self.y - start_y - inset * 2.0).max(0.0), paint, radius: 2.0 * self.env.metrics.zoom, border: None });
            self.indent = x;
            self.width = width;
        }
        self.base = self.base_for_container();
        self.gap(18.0 * self.env.metrics.zoom);
    }

    fn code_block(&mut self, lang: &str, text: &str, text_start: usize, text_map:&[MapSeg], block_start: usize) {
        self.flush_paragraph();
        if self.controlled { self.gap(18.0 * self.env.metrics.zoom); }
        self.apply_gap();
        let mut failure = None;
        if lang == "mermaid" {
            match self.mermaid(text, text_start, block_start) {
                Ok(()) => {
                    self.gap(self.env.metrics.paragraph_gap());
                    return;
                }
                Err(message) => failure = Some(message),
            }
        }
        if self.controlled {
            self.code_card(lang, text, text_start, text_map, block_start, failure.as_deref());
            return;
        }
        let metrics = self.env.metrics;
        let x = self.indent;
        let width = self.width - self.indent;
        let pad = 14.0 * metrics.zoom;
        let top = self.y;
        let header = 26.0 * metrics.zoom;
        let label = match lang {
            "" => "code".to_string(),
            "mermaid" => format!("Mermaid · {}", failure.as_deref().unwrap_or("渲染失败")),
            other => crate::nomo_mark::code_blocks::language_label(other).to_string(),
        };
        let label_spec = ParaSpec::plain(label, InlineStyle { bold: true, color: ColorRole::Muted, ..Default::default() }, Family::Text, 12.0 * metrics.zoom, header, Wrap::Greedy);
        let label_layout = self.env.layout(&label_spec, width, self.window);
        let label_width = label_layout.max_line_width();
        let mut y = top + header;
        let mut lines = Vec::new();
        let mut offset = 0;
        let body = text.strip_suffix('\n').unwrap_or(text);
        // 行号槽：宽度按最大行号位数估算，行号右对齐、弱化显示。
        let line_count = body.split('\n').count();
        let expanded = self.env.active_code == Some(block_start)
            || self.env.block_open.and_then(|open| open.get(&(self.env.source_base + block_start as u64))).copied().unwrap_or(false);
        let gutter = if self.env.code_line_numbers { (line_count.to_string().len() as f32 * 0.62 + 1.2) * metrics.code_size() } else { 0.0 };
        let highlighted = highlight::code_block(body, lang);
        for (number, line) in body.split('\n').enumerate() {
            if number >= 24 && !expanded && self.env.block_open.is_some() { break; }
            if gutter > 0.0 {
                let label = ParaSpec::plain((number + 1).to_string(), InlineStyle { color: ColorRole::Marker, ..Default::default() }, Family::Mono, metrics.code_size(), metrics.code_line_height(), Wrap::Greedy);
                let label = self.env.layout(&label, gutter, self.window);
                let label_width = label.max_line_width();
                lines.push(DisplayItem::Text { x: x + pad + gutter - label_width - 0.8 * metrics.code_size(), y, layout: label, map: Vec::new() });
            }
            let spec = ParaSpec {
                text: line.to_string(),
                spans: highlighted[number].clone(),
                base: InlineStyle { color: ColorRole::CodeFg, ..Default::default() },
                family: Family::Mono,
                font_size: metrics.code_size(),
                line_height: metrics.code_line_height(),
                wrap: Wrap::Greedy,
                objects: Vec::new(),
            };
            let layout = self.env.layout(&spec, (width - pad * 2.0 - gutter).max(40.0), self.window);
            let height = layout.height();
            let mut map:Vec<_> = text_map.iter().filter_map(|segment| {
                let from=segment.text.start.max(offset);let to=segment.text.end.min(offset+line.len());
                if from>to || (from==to&&!line.is_empty()){return None;}
                Some(MapSeg{text:from-offset..to-offset,source:segment.mapping().to_source(from,false),source_end:segment.mapping().to_source(to,true),exact:segment.exact})
            }).collect();
            if map.is_empty() {let at=text_map.iter().find(|seg|seg.text.start<=offset&&offset<=seg.text.end).map_or(text_start,|seg|seg.mapping().to_source(offset,false));map.push(MapSeg{text:0..0,source:at,source_end:at,exact:true});}
            lines.push(DisplayItem::Text { x: x + pad + gutter, y, layout, map });
            y += height;
            offset += line.len() + 1;
        }
        if line_count > 24 && self.env.block_open.is_some() {
            self.items.push(DisplayItem::Disclosure { x: x + pad, y, size: 16.0 * metrics.zoom, source: block_start, open: expanded });
            let label = if expanded { "收起代码".to_string() } else { format!("展开其余 {} 行", line_count - 24) };
            let spec = ParaSpec::plain(label, InlineStyle { color: ColorRole::Muted, ..Default::default() }, Family::Text, 12.0 * metrics.zoom, 22.0 * metrics.zoom, Wrap::Greedy);
            let label = self.env.layout(&spec, width - pad * 3.0, self.window);
            self.items.push(DisplayItem::Text { x: x + pad + 20.0 * metrics.zoom, y, layout: label, map: Vec::new() });
            y += 24.0 * metrics.zoom;
        }
        y += pad * 0.8;
        self.items.push(DisplayItem::Rect { x, y: top, w: width, h: y - top, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
        self.items.push(DisplayItem::Text { x: x + pad, y: top, layout: label_layout, map: Vec::new() });
        if lang != "mermaid" {
            self.items.push(DisplayItem::CodeLanguage { x: x + pad - 4.0 * metrics.zoom, y: top,
                w: (label_width + 22.0 * metrics.zoom).min((width - pad * 2.0 - 28.0 * metrics.zoom).max(16.0)),
                h: header, source: block_start });
        }
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
                let object = InlineObject { at: 0, width: info.width * scale, width_percent:None, ascent: (info.height - info.depth) * scale, descent: info.depth * scale, key: InlineAsset::Embed(key) };
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
        if tex.trim().is_empty() {
            // 空公式保留可点击的输入落点，不向排版引擎请求空位图。
            let note = Rc::new(self.note_layout("输入公式…", ColorRole::Muted, width));
            let h = note.height();
            self.items.push(DisplayItem::Text { x: self.indent + (width - note.max_line_width()) / 2.0,
                y: self.y + 8.0 * metrics.zoom, layout: note,
                map: vec![MapSeg { text: 0.."输入公式…".len(), source, source_end: source + tex.len(), exact: false }] });
            self.y += h + 16.0 * metrics.zoom;
            self.gap(metrics.paragraph_gap());
            return;
        }
        let (key, state) = (self.env.embed)(EmbedKind::Math { display: true }, tex);
        if let EmbedState::Ready(info) = &state {
            let pad = 8.0 * metrics.zoom;
            let top = self.y;
            let first = self.items.len();
            let (x, w, h) = self.fit_wide(info.width, info.height);
            self.items.push(DisplayItem::Embed { x, y: self.y + pad, w, h, key, source });
            self.y += h + pad * 2.0;
            self.wrap_horizontal(first, source..source + tex.len(), top, width, w);
            self.gap(metrics.paragraph_gap());
            return;
        }
        let spec = ParaSpec::plain(tex.trim().to_string(), InlineStyle { mono: true, italic: true, color: ColorRole::Accent, ..Default::default() }, Family::Text, metrics.body_size() * 1.05, metrics.body_line_height(), Wrap::Greedy);
        let layout = self.env.layout(&spec, width - 32.0, self.window);
        let text_width = layout.max_line_width();
        let note = match &state {
            EmbedState::Failed(message) => Some(Rc::new(self.note_layout(message, ColorRole::Caution, width - 32.0))),
            _ => None,
        };
        let note_height = note.as_ref().map_or(0.0, |note| note.height());
        let height = layout.height() + note_height + 20.0 * metrics.zoom;
        self.items.push(DisplayItem::Rect { x: self.indent, y: self.y, w: width, h: height, paint: Paint::CodeBg, radius: 8.0, border: None });
        let map = vec![MapSeg { text: 0..spec.text.len(), source, source_end: source + 1, exact: false }];
        self.items.push(DisplayItem::Text { x: self.indent + (width - text_width) / 2.0, y: self.y + 10.0 * metrics.zoom, layout: layout.clone(), map });
        if let Some(note) = note {
            self.items.push(DisplayItem::Text { x: self.indent + 16.0, y: self.y + 10.0 * metrics.zoom + layout.height(), layout: note, map: Vec::new() });
        }
        self.y += height;
        self.gap(metrics.paragraph_gap());
    }

    /// 宽内容保持可读尺寸；超出正文的部分由同一节点的横向滚动容器承载。
    /// 返回 (x, 宽, 高)。
    fn fit_wide(&self, width: f32, height: f32) -> (f32, f32, f32) {
        let available = self.width - self.indent;
        if width <= available {
            return (self.indent + (available - width) / 2.0, width, height);
        }
        (self.indent, width, height)
    }

    /// 小号提示文字（渲染失败原因等）。
    fn note_layout(&self, text: &str, color: ColorRole, width: f32) -> TextLayout {
        let metrics = self.env.metrics;
        let spec = ParaSpec::plain(text.to_string(), InlineStyle { color, ..Default::default() }, Family::Text, metrics.body_size() * 0.8, metrics.body_line_height() * 0.8, Wrap::Greedy);
        layout_text(&spec, width.max(40.0), self.window)
    }

    /// Mermaid 代码块：就绪时居中显示，过宽时在本节点内滚动；渲染中显示占位框。
    /// 返回 Ok 表示已按图表处理；Err 携带失败原因，调用方退回代码块并在标签中显示原因。
    fn mermaid(&mut self, text: &str, source: usize, block_start: usize) -> Result<(), String> {
        let metrics = self.env.metrics;
        let width = self.width - self.indent;
        let (key, state) = (self.env.embed)(EmbedKind::Mermaid, text);
        if self.controlled && !matches!(&state, EmbedState::Failed(_)) {
            // 工具栏属于节点容器，不随位图居中或横向滚动移动，也不遮住图表内容。
            let height = 40.0 * metrics.zoom;
            self.items.push(DisplayItem::BlockControls { x: self.indent, y: self.y, w: width, h: height,
                source: block_start..source + text.len(), kind: "diagram-toolbar" });
            self.y += height;
        }
        match state {
            EmbedState::Ready(info) => {
                let pad = 8.0 * metrics.zoom;
                let top = self.y;
                let first = self.items.len();
                let (x, w, h) = self.fit_wide(info.width, info.height);
                self.items.push(DisplayItem::Embed { x, y: self.y + pad, w, h, key, source });
                self.y += h + pad * 2.0;
                self.wrap_horizontal(first, source..source + text.len(), top, width, w);
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

    fn rule(&mut self, source: Range<usize>) {
        self.flush_paragraph();
        self.apply_gap();
        let zoom = self.env.metrics.zoom;
        self.items.push(DisplayItem::Rect { x: self.indent, y: self.y + 12.0 * zoom, w: self.width - self.indent, h: 1.0, paint: Paint::Border, radius: 0.0, border: None });
        self.items.push(DisplayItem::BlockControls { x: self.indent, y: self.y, w: self.width - self.indent, h: 25.0 * zoom, source, kind: "rule" });
        self.y += 25.0 * zoom;
        self.gap(self.env.metrics.paragraph_gap());
    }

    /// HTML 块的一行：折叠块标签只保留标题行，`<img>` 渲染为图片，其余按原样弱化显示。
    fn html_line(&mut self, html: &str, source: usize, fragment: Option<&crate::nomo_mark::html_table::HtmlFragment>) {
        let line = html.trim_end_matches('\n');
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if self.controlled&&source==0&&crate::nomo_mark::nomo_syntax::front_matter_end(html).is_some(){
            self.front_matter(source..source+html.len());self.gap(self.env.metrics.paragraph_gap());return;
        }
        if lower == "<details>" || lower.starts_with("<details ") || lower == "</details>" {
            self.flush_paragraph();
            if self.controlled{
                let em = self.env.metrics.body_size();
                if lower.starts_with("</") {
                    if self.details.pop().is_some() {
                        self.indent -= em * 0.8 + 18.0 * self.env.metrics.zoom;
                        self.width += em * 0.8;
                        self.y += em * 0.5;
                        if let Some((item, top)) = self.details_frames.pop() {
                            if let DisplayItem::Rect { h, .. } = &mut self.items[item] { *h = self.y - top; }
                        }
                        self.gap(em);
                    }
                } else {
                    self.apply_gap();
                    let top = self.y;
                    self.details_frames.push((self.items.len(), top));
                    self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: self.width - self.indent, h: 0.0,
                        paint: Paint::Surface, radius: 6.0 * self.env.metrics.zoom, border: Some(Paint::Border) });
                    self.details.push(source);
                    self.indent += em * 0.8 + 18.0 * self.env.metrics.zoom;
                    self.width -= em * 0.8;
                    self.y += em * 0.5;
                }
            }
            return;
        }
        // 目录标记由 Web 版维护，语义视图只显示其间的链接列表。
        if lower == "<!-- toc -->" || lower == "<!-- /toc -->" {
            return;
        }
        if self.controlled && trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            if !trimmed.contains('\n') {
                if let Some(content) = crate::nomo_mark::comments::content(trimmed) {
                    let start = source + html.len() - html.trim_start().len();
                    push_comment(&mut self.para, &content, start..start + trimmed.len(), self.font_size, self.env, self.window);
                    return;
                }
            }
            let content = trimmed.trim_start_matches("<!--").trim_end_matches("-->").trim();
            self.block_comment(content, source..source + html.len());
            return;
        }
        if lower.starts_with("<p") && lower.contains("align=") && !lower.contains("<img") {
            self.flush_paragraph();
            self.html_align = html_attr(trimmed, "align");
            self.html_inline(html, source, self.base_for_container());
            self.flush_paragraph();
            if lower.contains("</p>") { self.html_align = None; }
            return;
        }
        if lower == "</p>" {
            self.html_align = None;
            return;
        }
        if lower.starts_with("<summary") {
            self.flush_paragraph();
            let summary_start = self.items.len();
            let style = InlineStyle { bold: true, color: ColorRole::Heading, ..self.base_for_container() };
            if self.controlled {
                self.apply_gap();
                if let Some(opening)=self.details.last().copied(){self.items.push(DisplayItem::Disclosure{x:self.indent-18.0*self.env.metrics.zoom,y:self.y,size:16.0*self.env.metrics.zoom,source:opening,open:!self.closed_details.contains(&opening)});}
                if let Some(fragment) = fragment {
                    let base = self.base;
                    self.base = style;
                    self.html_fragment(fragment);
                    self.base = base;
                } else {
                    self.html_inline(html,source,style);
                    if self.para.is_empty() {
                        if let Some(tag) = crate::nomo_mark::nomo_syntax::html_tag(html, 0) { self.empty_anchor(source + tag.end); }
                    }
                }
            }else{
                let title = strip_tags(trimmed);
                self.para.push(&format!("▾ {}", title.trim()), style, Some((source, false)));
            }
            self.flush_paragraph();
            if let Some((DisplayItem::Disclosure { y, size, .. }, title_items)) = self.items[summary_start..].split_first_mut() {
                // 箭头跟随标题首行的实际行高，换行标题不按整个段落高度居中。
                if let Some((line_y, line_height)) = title_items.iter().find_map(|item| match item {
                    DisplayItem::Text { y, layout, .. } => Some((*y, layout.line_height)),
                    _ => None,
                }) {
                    *y = line_y + (line_height - *size) * 0.5;
                }
            }
            self.gap(self.env.metrics.paragraph_gap() * 0.5);
            return;
        }
        if let Some(at) = lower.find("<img") {
            if let Some(src) = html_attr(&trimmed[at..], "src") {
                self.flush_paragraph();
                let alt = html_attr(&trimmed[at..], "alt").unwrap_or_default();
                let width = html_attr(&trimmed[at..], "width").and_then(|value| ImageWidth::parse(&value));
                let align = html_attr(trimmed, "align").or_else(|| self.html_align.clone());
                self.image_with(&src, &alt, width, align.as_deref(),source..source+html.len());
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
    fn image_with(&mut self, url: &str, alt: &str, max_width: Option<ImageWidth>, align: Option<&str>, source:Range<usize>) {
        self.apply_gap();
        let metrics = self.env.metrics;
        let available = max_width.map_or(self.width - self.indent, |width| width.resolve(self.width-self.indent,metrics.zoom).min(self.width - self.indent));
        let path = crate::images::resolve_source(url, self.env.base_dir);
        let size = path.as_deref().and_then(|path| (self.env.image_size)(path));
        match (path, size) {
            (Some(path), Some((w, h))) if w > 0 && h > 0 => {
                // Store 返回原始像素；缩略图解码倍率不改变文档中的 CSS 尺寸。
                let natural = w as f32 * metrics.zoom;
                let display_w = if max_width.is_some() { available } else { natural.min(available) };
                let display_h = display_w * h as f32 / w as f32;
                let free = (self.width - self.indent - display_w).max(0.0);
                let offset = match align {
                    Some("center") => free / 2.0,
                    Some("right") => free,
                    _ => 0.0,
                };
                self.items.push(DisplayItem::Image { x: self.indent + offset, y: self.y, w: display_w, h: display_h, path,source });
                self.y += display_h;
            }
            (path, _) => {
                let note = match path {
                    Some(path) => format!("{}：{alt}", (self.env.image_failure)(&path).unwrap_or("图片未就绪")),
                    None => format!("远程图片未加载：{url}"),
                };
                let h = 64.0 * metrics.zoom;
                self.items.push(DisplayItem::Rect { x: self.indent, y: self.y, w: available, h, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
                let spec = ParaSpec::plain(note, InlineStyle { color: ColorRole::Muted, ..Default::default() }, Family::Text, metrics.body_size() * 0.85, h, Wrap::Greedy);
                let layout = self.env.layout(&spec, available - 24.0, self.window);
                let length=layout.text.len();
                self.items.push(DisplayItem::Text { x: self.indent + 12.0, y: self.y, layout,
                    map:vec![MapSeg{text:0..length,source:source.start,source_end:source.end,exact:false}] });
                self.y += h;
            }
        }
        self.gap(metrics.paragraph_gap());
    }

    fn html_inline(&mut self,html:&str,source:usize,base:InlineStyle){
        let mut at=0;let mut inline=InlineState::default();
        while at<html.len(){
            if let Some(tag)=crate::nomo_mark::nomo_syntax::html_tag(html,at){
                if inline.apply_html(&html[at..tag.end]) == Some(false) { self.line_break(source + at..source + tag.end); }
                at=tag.end;continue;
            }
            let end=html[at..].find('<').filter(|p|*p>0).map_or_else(||html.len(),|p|at+p);
            let text=html_escape::decode_html_entities(&html[at..end]);
            self.para.push(&text,inline.style(base),Some((source+at,text.as_ref()==&html[at..end])));
            if let Some(map)=self.para.map.last_mut(){map.source_end=source+end;}
            at=end;
        }
    }

    fn html_fragment(&mut self, fragment: &crate::nomo_mark::html_table::HtmlFragment) {
        use crate::nomo_mark::parse::InlineFormat;
        self.flush_paragraph(); self.apply_gap();
        let top = self.y;
        let index = self.items.len();
        let pad = 12.0 * self.env.metrics.zoom;
        if fragment.card {
            self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: self.width - self.indent, h: 0.0,
                paint: Paint::Surface, radius: 6.0 * self.env.metrics.zoom, border: Some(Paint::Border) });
            self.indent += pad; self.width -= pad; self.y += pad;
            let label = self.note_layout(&fragment.label, ColorRole::Muted, self.width - self.indent);
            let h = label.height();
            self.items.push(DisplayItem::Text { x: self.indent, y: self.y, layout: Rc::new(label), map: Vec::new() });
            self.y += h + pad;
        }
        for cell in &fragment.cells {
            self.html_align = cell.align.clone();
            if cell.runs.iter().filter(|run| !run.text.trim().is_empty()).count() == 1 {
                if let Some(crate::nomo_mark::html_table::InlineAtom::Image { align: Some(align), .. }) = cell.runs.iter().find_map(|run| run.atom.as_ref()) {
                    self.html_align = Some(align.clone());
                }
            }
            if cell.runs.is_empty() { self.empty_anchor(cell.content.start); }
            for run in &cell.runs {
                if run.text == "\n" && run.atom.is_none() { self.line_break(run.source.clone()); continue; }
                let mut style = self.base;
                for format in &run.formats {
                    match format {
                        InlineFormat::Strong => style.bold = true, InlineFormat::Emphasis => style.italic = true,
                        InlineFormat::Strike => style.strike = true, InlineFormat::Underline => style.underline = true,
                        InlineFormat::Highlight => style.bg = 1, InlineFormat::Superscript => style.shift = 1,
                        InlineFormat::Subscript => style.shift = -1,
                        InlineFormat::Code => { style.mono = true; style.code_bg = true; style.color = ColorRole::InlineCode; }
                        InlineFormat::Link => { style.underline = true; style.color = ColorRole::Link; }
                    }
                }
                if let Some(atom) = &run.atom { push_table_atom(&mut self.para, atom, run.source.clone(), style, self.env, self.window); }
                else {
                    self.para.push(&run.text, style, Some((run.source.start, self.source.get(run.source.clone()) == Some(run.text.as_str()))));
                    if let Some(map) = self.para.map.last_mut() { map.source_end = run.source.end; }
                }
            }
            self.flush_paragraph();
            self.html_align = None;
            self.gap(self.env.metrics.paragraph_gap());
        }
        if fragment.card {
            self.indent -= pad; self.width += pad; self.y += pad;
            if let DisplayItem::Rect { h, .. } = &mut self.items[index] { *h = self.y - top; }
        }
    }
    fn front_matter(&mut self,range:Range<usize>) {
        if self.controlled {
            let data = crate::nomo_mark::front_matter::read(&self.source[range.clone()]);
            let title = if data.title.is_empty() { "文档属性" } else { &data.title };
            let mut rows = Vec::new();
            if !data.created.is_empty() { rows.push(format!("创建于 {}", data.created)); }
            if !data.updated.is_empty() { rows.push(format!("更新于 {}", data.updated)); }
            if data.extra > 0 { rows.push(format!("另有 {} 项属性", data.extra)); }
            self.metadata_card(title, &data.status, &rows.join("  ·  "), &data.tags, data.warning, range);
            return;
        }
        let metrics = self.env.metrics;
        let pad = 12.0 * metrics.zoom;
        let width = self.width;
        let top = self.y;
        let mut y = top + pad;
        let mut offset = range.start;
        let mut texts = Vec::new();
        for raw in self.source[range].trim_end_matches(['\r','\n']).split('\n') {
            let line=raw.strip_suffix('\r').unwrap_or(raw);
            let spec = ParaSpec::plain(line.to_string(), InlineStyle { color: if line.starts_with("---") || line.starts_with("...") { ColorRole::Marker } else { ColorRole::Muted }, ..Default::default() }, Family::Mono, metrics.code_size(), metrics.code_line_height(), Wrap::Greedy);
            let layout = self.env.layout(&spec, width - pad * 2.0, self.window);
            let height = layout.height();
            texts.push(DisplayItem::Text { x: pad, y, layout, map: vec![MapSeg { text: 0..line.len(), source: offset, source_end: offset + line.len(), exact: true }] });
            y += height;
            offset += raw.len() + 1;
        }
        y += pad;
        self.items.push(DisplayItem::Rect { x: 0.0, y: top, w: width, h: y - top, paint: Paint::CodeBg, radius: 8.0, border: Some(Paint::CodeBorder) });
        self.items.extend(texts);
        self.y = y;
    }

    fn block_comment(&mut self, content: &str, range: Range<usize>) {
        self.flush_paragraph(); self.apply_gap();
        let zoom = self.env.metrics.zoom;
        let top = self.y; let pad = 10.0 * zoom; let label_width = 48.0 * zoom;
        let width = self.width - self.indent;
        let text = if content.is_empty() { "填写注释…" } else { content };
        let label = Rc::new(self.note_layout("注释", ColorRole::Success, label_width));
        let spec = ParaSpec::plain(text.to_owned(), InlineStyle { color: ColorRole::Muted, ..Default::default() },
            Family::Text, self.font_size * 0.9, self.line_height, Wrap::Greedy);
        let layout = self.env.layout(&spec, (width - label_width - pad * 2.0 - 60.0 * zoom).max(32.0), self.window);
        let height = layout.height().max(label.height()) + pad * 2.0;
        self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: width, h: height, paint: Paint::CodeBg, radius: 6.0 * zoom, border: Some(Paint::Border) });
        self.items.push(DisplayItem::Text { x: self.indent + pad, y: top + pad, layout: label, map: Vec::new() });
        self.items.push(DisplayItem::Text { x: self.indent + pad + label_width, y: top + pad, layout,
            map: vec![MapSeg { text: 0..text.len(), source: range.start, source_end: range.end, exact: false }] });
        self.items.push(DisplayItem::BlockControls { x: self.indent, y: top, w: width, h: height, source: range, kind: "comment" });
        self.y += height; self.gap(self.env.metrics.paragraph_gap());
    }

    fn metadata_card(&mut self, title: &str, status: &str, metadata: &str, tags: &[String], warning: bool, range: Range<usize>) {
        self.flush_paragraph(); self.apply_gap();
        let top = self.y; let first = self.items.len(); let zoom = self.env.metrics.zoom;
        let pad = 14.0 * zoom; let width = self.width - self.indent;
        self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: width, h: 0.0, paint: Paint::CodeBg, radius: 8.0 * zoom, border: Some(Paint::Border) });
        let status_layout = (!status.is_empty()).then(|| Rc::new(self.note_layout(status, ColorRole::Accent, (width * 0.25).max(20.0))));
        let status_w = status_layout.as_ref().map_or(0.0, |layout| layout.max_line_width() + pad);
        let spec = ParaSpec::plain(format!("▤  {title}"), InlineStyle { bold: true, color: ColorRole::Heading, ..Default::default() },
            Family::Text, self.font_size, self.line_height, Wrap::Greedy);
        let heading = self.env.layout(&spec, (width - pad * 2.0 - status_w - 64.0 * zoom).max(32.0), self.window);
        let row_h = heading.height().max(status_layout.as_ref().map_or(0.0, |layout| layout.height()));
        self.items.push(DisplayItem::Text { x: self.indent + pad, y: top + pad, layout: heading, map: Vec::new() });
        if let Some(layout) = status_layout {
            let x = self.indent + width - pad - 64.0 * zoom - status_w;
            self.items.push(DisplayItem::Rect { x, y: top + pad, w: status_w, h: layout.height(), paint: Paint::Surface, radius: 4.0 * zoom, border: Some(Paint::Border) });
            self.items.push(DisplayItem::Text { x: x + pad * 0.5, y: top + pad, layout, map: Vec::new() });
        }
        self.y += pad + row_h;
        if !metadata.is_empty() {
            let layout = Rc::new(self.note_layout(metadata, ColorRole::Muted, (width - pad * 2.0).max(32.0)));
            self.items.push(DisplayItem::Text { x: self.indent + pad, y: self.y + pad * 0.5, layout: layout.clone(), map: Vec::new() });
            self.y += pad * 0.5 + layout.height();
        }
        if !tags.is_empty() {
            self.y += pad * 0.5;
            let mut x = self.indent + pad; let mut row = 0.0f32;
            for tag in tags {
                let layout = Rc::new(self.note_layout(tag, ColorRole::Muted, (width - pad * 3.0).max(20.0)));
                let w = layout.max_line_width() + pad; let h = layout.height() + 4.0 * zoom;
                if x > self.indent + pad && x + w > self.width - pad { self.y += row + 6.0 * zoom; x = self.indent + pad; row = 0.0; }
                self.items.push(DisplayItem::Rect { x, y: self.y, w, h, paint: Paint::Surface, radius: 4.0 * zoom, border: Some(Paint::Border) });
                self.items.push(DisplayItem::Text { x: x + pad * 0.5, y: self.y + 2.0 * zoom, layout, map: Vec::new() });
                x += w + 6.0 * zoom; row = row.max(h);
            }
            self.y += row;
        }
        if warning {
            let layout = Rc::new(self.note_layout("部分属性包含复杂值，请在编辑时查看原文", ColorRole::Warning, (width - pad * 2.0).max(32.0)));
            self.items.push(DisplayItem::Text { x: self.indent + pad, y: self.y + 6.0 * zoom, layout: layout.clone(), map: Vec::new() });
            self.y += layout.height() + 6.0 * zoom;
        }
        self.y += pad;
        if let DisplayItem::Rect { h, .. } = &mut self.items[first] { *h = self.y - top; }
        self.items.push(DisplayItem::BlockControls { x: self.indent, y: top, w: width, h: self.y - top, source: range, kind: "front-matter" });
        self.gap(self.env.metrics.paragraph_gap());
    }

    fn toc(&mut self, range: Range<usize>) {
        let zoom = self.env.metrics.zoom;
        self.flush_paragraph(); self.gap(20.0 * zoom); self.apply_gap();
        let top = self.y;
        let pad = 18.0 * zoom;
        let top_pad = 14.0 * zoom;
        let width = self.width - self.indent;
        let mut content = Vec::new();
        let title = ParaSpec::plain("目录".into(), InlineStyle { bold: true, color: ColorRole::Heading, ..Default::default() },
            Family::Text, 19.0 * zoom, 28.0 * zoom, Wrap::Greedy);
        let title = self.env.layout(&title, width, self.window);
        content.push(DisplayItem::Text { x: self.indent + pad, y: top + top_pad, layout: title.clone(), map: Vec::new() });
        self.y = top + top_pad + title.height() + 8.0 * zoom;
        content.push(DisplayItem::Rect { x: self.indent + pad, y: self.y, w: (width - pad * 2.0).max(0.0), h: 1.0, paint: Paint::Border, radius: 0.0, border: None });
        self.y += 10.0 * zoom;
        let mut entries = Parser::new(&self.source[range.clone()]).into_offset_iter();
        let mut number = 0;
        while let Some((event, span)) = entries.next() {
            let Event::Start(Tag::Link { dest_url, .. }) = event else { continue; };
            let mut label = String::new();
            for (event, _) in entries.by_ref() {
                match event { Event::End(TagEnd::Link) => break, Event::Text(text) | Event::Code(text) => label.push_str(&text), _ => {} }
            }
            number += 1;
            let source = range.start + span.start;
            let line = crate::nomo_mark::commands::line_start(self.source, source);
            let indent = self.source[line..source].bytes().take_while(|b| *b == b' ').count() as f32 * 8.0 * zoom;
            let x = self.indent + pad + indent.min((width - 100.0).max(0.0));
            let spec = ParaSpec::plain(if indent > 0.0 { format!("• {label}") } else { label }, InlineStyle { color: ColorRole::Link, ..Default::default() },
                Family::Text, 17.0 * zoom, 17.0 * 1.45 * zoom, Wrap::Greedy);
            let label = self.env.layout(&spec, (self.width - x - 54.0).max(20.0), self.window);
            let y = self.y;
            let h = label.height().max(30.0 * zoom);
            content.push(DisplayItem::Text { x, y, layout: label, map: vec![MapSeg { text: 0..spec.text.len(), source, source_end: range.start + span.end, exact: false }] });
            let number_spec = ParaSpec::plain(number.to_string(), InlineStyle { color: ColorRole::Muted, ..Default::default() },
                Family::Text, 13.0 * zoom, 17.0 * 1.45 * zoom, Wrap::Greedy);
            let number_layout = self.env.layout(&number_spec, 36.0 * zoom, self.window);
            content.push(DisplayItem::Text { x: self.width - pad - number_layout.max_line_width(), y, layout: number_layout, map: Vec::new() });
            content.push(DisplayItem::TocLink { x, y, w: self.width - x - pad, h, source, target: dest_url.to_string() });
            self.y += h + 8.0 * zoom;
        }
        if number == 0 {
            let spec = ParaSpec::plain("文档暂无标题".into(), InlineStyle { color: ColorRole::Muted, ..Default::default() },
                Family::Text, self.env.metrics.body_size() * 0.9, self.line_height, Wrap::Greedy);
            let label = self.env.layout(&spec, width - pad * 2.0, self.window);
            self.y += label.height();
            content.push(DisplayItem::Text { x: self.indent + pad, y: top + pad + title.height() + 8.0, layout: label, map: Vec::new() });
        }
        self.y += 16.0 * zoom;
        self.items.push(DisplayItem::Rect { x: self.indent, y: top, w: width, h: self.y - top, paint: Paint::TocCard, radius: 8.0 * zoom, border: Some(Paint::Border) });
        self.items.extend(content);
        self.items.push(DisplayItem::BlockControls { x: self.indent, y: top, w: width, h: 30.0, source: range, kind: "toc" });
        self.gap(34.0 * zoom);
    }

    fn html_table(&mut self,table:&crate::nomo_mark::html_table::HtmlTable,revealed:&[Range<usize>]) {
        use crate::nomo_mark::{parse::InlineFormat,table_widths::{self,CellMetrics}};
        self.flush_paragraph();self.apply_table_gap();let first_item=self.items.len();let top=self.y;
        let zoom=self.env.metrics.zoom;let em=14.0*zoom;let line_height=em*1.55;let pad_x=11.0*zoom;let pad_y=9.0*zoom;
        let border = zoom;
        let viewport_width = (self.width - self.indent).max(1.0);
        let mut prepared=Vec::new();let mut measures=Vec::new();
        for cell in &table.cells {
            let base=table_layout::header_style(InlineStyle::default(), cell.header);
            let font_size = if cell.header { 13.0 } else { 14.0 } * zoom;
            let mut pieces:Vec<_>=cell.runs.iter().map(|run|(run.source.clone(),run.text.clone(),run.formats.clone(),run.atom.as_ref())).collect();
            for span in &cell.spans{for marker in [&span.open,&span.close]{if revealed.contains(marker){pieces.push((marker.clone(),self.source[marker.clone()].into(),Vec::new(),None));}}}
            pieces.sort_by_key(|(range,_,_,_)|range.start);
            let mut paragraphs=Vec::new();let mut para=ParaBuilder::new();
            let mut empty_anchor=cell.content.start;
            for (range,text,formats,atom) in pieces {
                if text=="\n" {
                    if para.map.is_empty(){para.map.push(MapSeg{text:0..0,source:range.start,source_end:range.start,exact:true});}
                    paragraphs.push(para);para=ParaBuilder::new();
                    // 只为空行保留锚点，后续正文仍使用自身格式和源码映射。
                    empty_anchor=range.end;
                    continue;
                }
                let mut style=base;
                for format in formats{match format{InlineFormat::Strong=>style.bold=true,InlineFormat::Emphasis=>style.italic=true,InlineFormat::Code=>{style.mono=true;style.code_bg=true;style.color=ColorRole::InlineCode;},InlineFormat::Strike=>style.strike=true,InlineFormat::Underline=>style.underline=true,InlineFormat::Highlight=>style.bg=1,InlineFormat::Superscript=>style.shift=1,InlineFormat::Subscript=>style.shift=-1,InlineFormat::Link=>{style.underline=true;style.color=ColorRole::Link;}}}
                if let Some(atom) = atom {
                    push_table_atom(&mut para, atom, range, style, self.env, self.window);
                    continue;
                }
                if text.is_empty(){para.map.push(MapSeg{text:para.text.len()..para.text.len(),source:range.start,source_end:range.end,exact:true});}
                else{let exact=self.source.get(range.clone())==Some(text.as_str());para.push(&text,style,Some((range.start,exact)));if let Some(map)=para.map.last_mut(){map.source_end=range.end;}}
            }
            if para.map.is_empty(){para.map.push(MapSeg{text:0..0,source:empty_anchor,source_end:empty_anchor,exact:true});}paragraphs.push(para);
            let mut natural=0.0f32;let mut specs=Vec::new();
            let mut atomic = 0.0f32;
            for para in paragraphs {
                let spec = ParaSpec { text:para.text, spans:para.spans, base, family:Family::Text,
                    font_size, line_height:font_size*1.55, wrap:Wrap::Greedy, objects:para.objects };
                let measured = table_layout::measure(&spec, &para.map, revealed, &para.table_images, self.env, self.window);
                natural = natural.max(measured.0);
                atomic = atomic.max(measured.1);
                specs.push((spec, para.map));
            }
            measures.push(CellMetrics{column:cell.column,span:cell.colspan,header:cell.header,natural,atomic,chrome:pad_x*2.0+zoom,explicit:cell.widths.iter().map(|width|*width as f32*zoom).collect()});
            prepared.push(specs);
        }
        let widths=table_widths::allocate(&table_widths::constraints(table.columns,&measures,em),(viewport_width-border).max(0.0));
        let mut xs=vec![self.indent+border];for width in &widths{xs.push(xs.last().unwrap()+width);}
        let mut row_heights=vec![line_height+pad_y*2.0;table.rows];let mut laid=Vec::new();
        for (cell,paragraphs) in table.cells.iter().zip(prepared){
            let width=widths[cell.column..cell.column+cell.colspan].iter().sum::<f32>();let mut height=0.0;let mut content=Vec::new();
            let content_width = (width - pad_x * 2.0 - border).max(1.0);
            for (spec, map) in paragraphs {
                let layout = table_layout::layout(&spec, content_width, self.env, self.window);
                let image_align = (spec.text.trim() == OBJECT_CHAR.to_string()).then(|| cell.runs.iter().find_map(|run| {
                    let crate::nomo_mark::html_table::InlineAtom::Image { align, .. } = run.atom.as_ref()? else { return None; };
                    map.iter().any(|segment| segment.source == run.source.start).then(|| align.as_deref()).flatten()
                })).flatten();
                let free = (content_width - layout.max_line_width()).max(0.0);
                let offset = match image_align.or(cell.align.as_deref()) { Some("center") => free / 2.0, Some("right") => free, _ => 0.0 };
                height += layout.height();
                content.push((height - layout.height(), offset, layout, map));
            }
            if cell.rowspan==1{row_heights[cell.row]=row_heights[cell.row].max(height+pad_y*2.0);}laid.push((height,content));
        }
        for (cell,(height,_)) in table.cells.iter().zip(&laid){let available=row_heights[cell.row..cell.row+cell.rowspan].iter().sum::<f32>();if available<height+pad_y*2.0{row_heights[cell.row+cell.rowspan-1]+=height+pad_y*2.0-available;}}
        let mut ys=vec![self.y];for height in row_heights{ys.push(ys.last().unwrap()+height);}
        let table_width = xs.last().unwrap() - self.indent;
        self.items.push(DisplayItem::Rect { x:self.indent, y:top, w:table_width, h:ys.last().unwrap()-top,
            paint:Paint::Surface, radius:0.0, border:Some(Paint::TableBorder) });
        let mut texts=Vec::new();
        for (cell,(_,content)) in table.cells.iter().zip(laid){
            let x=xs[cell.column];let y=ys[cell.row];let width=xs[cell.column+cell.colspan]-x;let height=ys[cell.row+cell.rowspan]-y;
            self.items.push(DisplayItem::Rect{x,y,w:width,h:height,paint:if cell.header{Paint::TableHeader}else{Paint::Surface},radius:0.0,border:Some(Paint::TableBorder)});
            self.items.push(DisplayItem::TableCell { x, y, w: width, h: height, table: table.source.start,
                row: cell.row, column: cell.column, rowspan: cell.rowspan, colspan: cell.colspan, source: cell.content.clone() });
            for (dy,dx,layout,map) in content{texts.push(DisplayItem::Text{x:x+pad_x+dx,y:y+pad_y+dy,layout,map});}
        }
        self.items.extend(texts);self.y=*ys.last().unwrap();
        self.wrap_horizontal(first_item,table.source.clone(),top,viewport_width,table_width);
        self.gap(20.0 * zoom);
    }
    fn table(&mut self, table: TableData, revealed: &[Range<usize>]) {
        self.apply_table_gap();
        let first_item=self.items.len();
        let metrics = self.env.metrics;
        let columns = table.rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0).max(table.alignments.len());
        if columns == 0 {
            return;
        }
        // 与旧版滚动壳一致：扣除所在容器的内缩和表格外边框后分配列宽。
        let border = if self.controlled { metrics.zoom } else { 0.0 };
        let viewport_width = (self.width - self.indent).max(1.0);
        let available = if self.controlled { (viewport_width - border).max(0.0) }
            else { (self.env.wide_width - self.indent).max(viewport_width) };
        let pad_x = 11.0 * metrics.zoom;
        let pad_y = if self.controlled {9.0} else {8.0} * metrics.zoom;
        let font_size = 14.0 * metrics.zoom;
        let line_height = 14.0 * 1.55 * metrics.zoom;
        let spec_for = |cell: &ParaBuilder, header: bool| ParaSpec {
            text: cell.text.clone(),
            spans: cell.spans.iter().map(|(range, style)| (range.clone(),
                if self.controlled { table_layout::header_style(*style, header) } else { *style })).collect(),
            base: if self.controlled { table_layout::header_style(InlineStyle::default(), header) }
                else { InlineStyle { bold: header, color: if header { ColorRole::Heading } else { ColorRole::Fg }, ..Default::default() } },
            family: Family::Text,
            font_size: if self.controlled && header { 13.0 * metrics.zoom } else { font_size },
            line_height: if self.controlled && header { 13.0 * 1.55 * metrics.zoom } else { line_height },
            wrap: Wrap::Greedy,
            objects: cell.objects.clone(),
        };
        // NomoMark 使用共享语义约束；旧 Buffer 渲染入口保留原有分配方式。
        let mut natural = vec![40.0f32; columns];
        let mut cell_metrics=Vec::new();
        for (header, cells) in &table.rows {
            for (column, cell) in cells.iter().enumerate() {
                let spec = spec_for(cell, *header);
                if self.controlled {
                    let (natural, atomic) = table_layout::measure(&spec, &cell.map, revealed, &cell.table_images, self.env, self.window);
                    cell_metrics.push(crate::nomo_mark::table_widths::CellMetrics {
                        column, span:1, header:*header, natural, atomic, chrome:pad_x*2.0+border, explicit:Vec::new() });
                } else {
                    let layout = self.env.layout(&spec, 10_000.0, self.window);
                    natural[column] = natural[column].max(layout.max_line_width() + pad_x * 2.0 + 2.0);
                }
            }
        }
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = if self.controlled {
            let constraints=crate::nomo_mark::table_widths::constraints(columns,&cell_metrics,font_size);
            crate::nomo_mark::table_widths::allocate(&constraints,available)
        } else if total <= available {
            natural
        } else {
            let floors: Vec<f32> = natural.iter().map(|width| width.min(120.0 * metrics.zoom)).collect();
            let floor_total: f32 = floors.iter().sum();
            let scale = (available - floor_total).max(0.0) / (total - floor_total).max(1.0);
            natural.iter().zip(&floors).map(|(width, floor)| floor + (width - floor) * scale).collect()
        };
        let table_width: f32 = widths.iter().sum::<f32>() + border;
        let top = self.y;
        let mut y = top;
        let mut cells_out = Vec::new();
        let mut row_bounds = Vec::new();
        for (row, (header, cells)) in table.rows.iter().enumerate() {
            let mut row_height = if self.controlled && *header { 13.0 * 1.55 * metrics.zoom } else { line_height };
            let mut laid = Vec::new();
            let mut x = self.indent + border;
            for column in 0..columns {
                let width = widths[column];
                if let Some(cell) = cells.get(column) {
                    let content_width = (width - pad_x * 2.0 - border).max(1.0);
                    let layout = if self.controlled { table_layout::layout(&spec_for(cell, *header), content_width, self.env, self.window) }
                        else { self.env.layout(&spec_for(cell, *header), content_width.max(20.0), self.window) };
                    row_height = row_height.max(layout.height());
                    let text_width = layout.max_line_width();
                    let image_align = (cell.text.trim() == OBJECT_CHAR.to_string()).then(|| cell.table_images.first().and_then(|image| image.align)).flatten();
                    let offset = match image_align.as_ref().or_else(|| table.alignments.get(column)) {
                        Some(Alignment::Center) => (content_width - text_width).max(0.0) / 2.0,
                        Some(Alignment::Right) => (content_width - text_width).max(0.0),
                        _ => 0.0,
                    };
                    laid.push((x + pad_x + offset, layout, cell.map.clone()));
                }
                x += width;
            }
            let height = row_height + pad_y * 2.0;
            row_bounds.push((*header, y, height));
            if self.controlled {
                let mut x = self.indent + border;
                for (column, cell) in cells.iter().enumerate() {
                    if let (Some(start), Some(end)) = (cell.map.iter().map(|m| m.source).min(), cell.map.iter().map(|m| m.source_end).max()) {
                        cells_out.push(DisplayItem::TableCell { x, y, w: widths[column], h: height, table: table.source.start,
                            row, column, rowspan: 1, colspan: 1, source: start..end });
                    }
                    x += widths[column];
                }
            }
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
        let mut x = self.indent + border;
        for width in &widths[..columns - 1] {
            x += width;
            self.items.push(DisplayItem::Rect { x, y: top, w: 1.0, h: y - top, paint: Paint::TableBorder, radius: 0.0, border: None });
        }
        self.items.extend(cells_out);
        self.y = y;
        if self.controlled {self.wrap_horizontal(first_item,table.source,top,viewport_width,table_width);}
        self.gap(if self.controlled { 20.0 * metrics.zoom } else { metrics.paragraph_gap() });
    }

    fn finish(mut self, has_following_block: bool) -> BlockLayout {
        self.flush_paragraph();
        // 跨窗口仍保留最后一个块的段距，但不把分隔行渲染成额外空段落。
        if has_following_block { self.apply_gap(); }
        BlockLayout { height: self.y.max(1.0), items: self.items }
    }
    fn wrap_horizontal(&mut self,first:usize,source:Range<usize>,top:f32,width:f32,content_width:f32){
        if content_width<=width+0.5{return;}
        let mut items=self.items.split_off(first);
        for item in &mut items{match item{
            DisplayItem::CodeBody{x,y,..}|DisplayItem::CodeHeader{x,y,..}|DisplayItem::CodeLanguage{x,y,..}|DisplayItem::Text{x,y,..}|DisplayItem::Rect{x,y,..}|DisplayItem::Image{x,y,..}|DisplayItem::Checkbox{x,y,..}|DisplayItem::CopyButton{x,y,..}|DisplayItem::Disclosure{x,y,..}|DisplayItem::Embed{x,y,..}|DisplayItem::Horizontal{x,y,..}|DisplayItem::TableCell{x,y,..}|DisplayItem::CalloutHeader{x,y,..}|DisplayItem::BlockControls{x,y,..}|DisplayItem::TocLink{x,y,..}=>{*x-=self.indent;*y-=top;}
        }}
        let height=self.y-top;self.y+=8.0;
        self.items.push(DisplayItem::Horizontal{x:self.indent,y:top,w:width,h:height+8.0,content_width,source,offset:std::cell::Cell::new(0.0),layout:Box::new(BlockLayout{height,items})});
    }
}

pub(crate) fn percent_decode(url: &str) -> String {
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
