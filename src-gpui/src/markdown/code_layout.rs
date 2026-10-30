//! 代码卡片的局部布局。只拥有当前窗口内的行布局，滚动不重新解析正文。
use super::*;
use std::cell::Cell;

pub struct CodeViewport {
    pub code: BlockLayout,
    pub numbers: BlockLayout,
    pub gutter: f32,
    pub content_width: f32,
    pub scrollbar: f32,
    pub offset: Cell<(f32, f32)>,
}

impl CodeViewport {
    pub fn viewport(&self, width: f32, height: f32) -> (f32, f32) {
        let mut w = (width - self.gutter).max(1.0);
        let mut h = height;
        // 一个方向的滚动条可能使另一个方向也溢出。
        for _ in 0..2 {
            h = (height - if self.content_width > w { self.scrollbar } else { 0.0 }).max(1.0);
            w = (width - self.gutter - if self.code.height > h { self.scrollbar } else { 0.0 }).max(1.0);
        }
        (w, h)
    }

    pub fn clamp_offset(&self, width: f32, height: f32, offset: (f32, f32)) -> (f32, f32) {
        let (w, h) = self.viewport(width, height);
        (offset.0.clamp(0.0, (self.content_width-w).max(0.0)), offset.1.clamp(0.0, (self.code.height-h).max(0.0)))
    }

    pub fn reveal(&self, source: usize, width: f32, height: f32) {
        let Some((x, y, line)) = self.code.caret_at(source) else { return; };
        let (w, h) = self.viewport(width, height);
        let (dx, dy) = self.offset.get();
        let dx = if x < dx+4.0 { x-4.0 } else if x > dx+w-6.0 { x-w+6.0 } else { dx };
        let dy = if y < dy { y } else if y+line > dy+h { y+line-h } else { dy };
        self.offset.set(self.clamp_offset(width, height, (dx, dy)));
    }

    pub fn caret_visible(&self, source: usize, width: f32, height: f32) -> bool {
        let Some((x, y, line)) = self.code.caret_at(source) else { return false; };
        let (w, h) = self.viewport(width, height);
        let (dx, dy) = self.offset.get();
        x >= dx && x < dx+w && y >= dy && y+line <= dy+h+0.5
    }
}

impl Builder<'_> {
    pub(super) fn code_card(&mut self, lang: &str, text: &str, text_start: usize, text_map: &[MapSeg], block_start: usize, failure: Option<&str>) {
        crate::theme::prepare_code_font(self.window);
        let zoom = self.env.metrics.zoom;
        let (x, top, width) = (self.indent, self.y, (self.width-self.indent).max(1.0));
        let border = zoom;
        let header = 38.0 * zoom;
        let font_size = 13.0 * zoom;
        let line_height = 18.0 * zoom;
        let pad = 12.0 * zoom;
        let body = text.strip_suffix('\n').unwrap_or(text);
        let count = body.split('\n').count();
        let expanded = self.env.block_open.and_then(|open| open.get(&(self.env.source_base+block_start as u64))).copied().unwrap_or(false);
        let number_spec = |label: String| ParaSpec::plain(label, InlineStyle { color: ColorRole::CodeLineNumber, ..Default::default() }, Family::Code, font_size, line_height, Wrap::None);
        let gutter = if self.env.code_line_numbers {
            self.env.layout(&number_spec(count.to_string()), 1_000_000.0, self.window).max_line_width()+14.0*zoom+border
        } else { 0.0 };
        let mut code = Vec::new();
        let mut numbers = Vec::new();
        let mut offset = 0;
        let mut y = pad;
        let mut content_width = 0.0f32;
        let highlighted = highlight::code_block(body, lang);
        for (number, line) in body.split('\n').enumerate() {
            let spec = ParaSpec {
                text: line.to_string(), spans: highlighted[number].clone(),
                base: InlineStyle { color: ColorRole::CodeFg, ..Default::default() },
                family: Family::Code, font_size, line_height, wrap: Wrap::None, objects: Vec::new(),
            };
            let layout = self.env.layout(&spec, 1_000_000.0, self.window);
            let height = layout.height();
            if gutter > 0.0 {
                let label = self.env.layout(&number_spec((number+1).to_string()), gutter, self.window);
                numbers.push(DisplayItem::Text { x: gutter-border-8.0*zoom-label.max_line_width(), y, layout: label, map: Vec::new() });
            }
            let mut map: Vec<_> = text_map.iter().filter_map(|segment| {
                let from = segment.text.start.max(offset); let to = segment.text.end.min(offset+line.len());
                if from > to || (from == to && !line.is_empty()) { return None; }
                Some(MapSeg { text: from-offset..to-offset, source: segment.mapping().to_source(from, false), source_end: segment.mapping().to_source(to, true), exact: segment.exact })
            }).collect();
            if map.is_empty() {
                let at = text_map.iter().find(|seg| seg.text.start <= offset && offset <= seg.text.end).map_or(text_start, |seg| seg.mapping().to_source(offset, false));
                map.push(MapSeg { text: 0..0, source: at, source_end: at, exact: true });
            }
            content_width = content_width.max(pad+layout.max_line_width()+16.0*zoom);
            code.push(DisplayItem::Text { x: pad, y, layout, map });
            y += height;
            offset += line.len()+1;
        }
        let content_height = (y-pad).max(3.0*line_height)+pad*2.0;
        let limit = (24.0*line_height+pad*2.0).min(f32::from(self.window.viewport_size().height)*0.6).max(3.0*line_height+pad*2.0);
        let body_height = if expanded { content_height } else { content_height.min(limit) };
        let body_width = (width-2.0*border).max(1.0);
        let viewport = CodeViewport {
            code: BlockLayout { height: content_height, items: code },
            numbers: BlockLayout { height: content_height, items: numbers },
            gutter, content_width, scrollbar: 13.0*zoom, offset: Cell::new((0.0, 0.0)),
        };
        // 展开时只取消纵向限制，横向滚动条仍占自己的空间。
        let body_height = body_height + if content_width > body_width-gutter { viewport.scrollbar } else { 0.0 };
        self.items.push(DisplayItem::Rect { x, y: top, w: width, h: header+body_height+2.0*border, paint: Paint::CodeBg, radius: 8.0*zoom, border: Some(Paint::CodeBorder) });
        self.items.push(DisplayItem::Rect { x: x+border, y: top+header, w: body_width, h: border, paint: Paint::CodeBorder, radius: 0.0, border: None });
        let source_end = text_map.iter().map(|seg| seg.source_end).max().unwrap_or(text_start).max(text_start);
        self.items.push(DisplayItem::CodeBody { x: x+border, y: top+header+border, w: body_width, h: body_height, source: block_start..source_end.saturating_add(1), body: Box::new(viewport) });
        let label = if lang == "mermaid" { format!("Mermaid · {}", failure.unwrap_or("渲染失败")) }
            else { crate::nomo_mark::code_blocks::language_label(lang).to_string() };
        let limit = (width-14.0*zoom-10.0*zoom-if count > 24 { 76.0*zoom } else { 38.0*zoom }).max(1.0);
        let mut label: String = label.chars().take(64).collect();
        let label_layout = loop {
            let spec = ParaSpec::plain(label.clone(), InlineStyle { bold: true, color: ColorRole::CodeLabel, ..Default::default() }, Family::Code, 12.0*zoom, header-border, Wrap::None);
            let layout = self.env.layout(&spec, limit, self.window);
            if layout.max_line_width() <= limit || label.chars().count() <= 1 { break layout; }
            label.pop(); label.pop(); label.push('…');
        };
        let label_width = label_layout.max_line_width();
        self.items.push(DisplayItem::Text { x: x+border+14.0*zoom, y: top+border, layout: label_layout, map: Vec::new() });
        if lang != "mermaid" { self.items.push(DisplayItem::CodeLanguage { x: x+border+14.0*zoom, y: top+border, w: label_width.max(12.0*zoom), h: header-border, source: block_start }); }
        self.items.push(DisplayItem::CodeHeader { x: x+width-border-10.0*zoom-32.0*zoom, y: top+border+(header-border-28.0*zoom)/2.0, w: 32.0*zoom, h: 28.0*zoom, source: block_start, text: Rc::from(body), expanded: (count > 24).then_some(expanded) });
        self.y = top+header+body_height+2.0*border;
        self.gap(18.0*zoom);
    }
}
