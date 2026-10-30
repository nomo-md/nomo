//! A4 原生分页；正文以嵌入字体输出，增强图形按有界位图输出。
use super::{Appearance, ExportReport, Job, media::{self, Asset}, model::*, pdf_fonts::Fonts};
use krilla::{Document, color::rgb, geom::{Point, Rect, Size, Transform, PathBuilder}, image::Image,
    metadata::Metadata, outline::{Outline, OutlineNode}, annotation::{Annotation, LinkAnnotation, Target}, action::LinkAction, destination::XyzDestination,
    num::NormalizedF32, page::PageSettings, paint::Fill, text::KrillaGlyph};
use pulldown_cmark::Alignment;
use std::{collections::BTreeMap, path::Path};
use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN: f32 = 56.69;
const WIDTH: f32 = PAGE_W - 2.0 * MARGIN;
const HEIGHT: f32 = PAGE_H - 2.0 * MARGIN;
#[derive(Clone)]
enum ItemKind { Text { text: String, glyphs: Vec<KrillaGlyph>, font: usize, size: f32, style: Style }, Image(usize) }
#[derive(Clone)]
struct Item { kind: ItemKind, width: f32, height: f32, level: Level, whitespace: bool }
#[derive(Clone, Default)]
struct Line { items: Vec<Item>, width: f32, height: f32 }
enum Draw { Item { item: Item, x: f32, y: f32 }, Rect { x: f32, y: f32, w: f32, h: f32, color: [u8; 3] } }
struct Layout<'a> {
    fonts: Fonts, assets: &'a [Asset], pages: Vec<Vec<Draw>>, y: f32, bytes: usize, glyphs: usize, prepared: usize,
    destinations: BTreeMap<String, (usize, f32)>, headings: Vec<(u8, String, usize, f32)>,
    appearance: &'a Appearance, job: &'a Job<'a>, report: &'a mut ExportReport,
}
pub(super) fn rgb_bytes(value: gpui::Hsla) -> [u8; 3] {
    let c: gpui::Rgba = value.into(); [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8]
}
pub(super) fn render(model: &Model, assets: Vec<Asset>, title: &str, appearance: &Appearance, path: Option<&Path>, job: &Job, report: &mut ExportReport) -> Result<Vec<u8>, String> {
    let mut layout = Layout { fonts: Fonts::new(appearance), assets: &assets, pages: vec![Vec::new()], y: MARGIN, bytes: 0, glyphs: 0, prepared: 0,
        destinations: BTreeMap::new(), headings: Vec::new(), appearance, job, report };
    layout.blocks(&model.blocks)?;
    if !model.footnotes.is_empty() {
        layout.report.warn("PDF 脚注集中置于文末并保留跳转");
        layout.paragraph(&Paragraph { heading: 2, runs: vec![Run::text("注释")], ..Default::default() })?;
        for (label, blocks) in &model.footnotes {
            let p = Paragraph { id: Some(format!("nomo-footnote-{label}")), runs: vec![Run::text(format!("[{label}]"))], ..Default::default() };
            layout.paragraph(&p)?; layout.blocks(blocks)?;
        }
    }
    let Layout { fonts, pages, destinations, headings, .. } = layout;
    job.check()?;
    let images: Vec<Image> = assets.into_iter().map(|asset| { let (w,h) = asset.pixels.dimensions(); Image::from_rgba8(asset.pixels.into_raw(), w, h) }).collect();
    let mut doc = Document::new(); doc.set_metadata(Metadata::new().title(title.into()));
    let mut outline = Outline::new();
    // 递归建立真正的标题层级，层级跳跃也允许（例如 H1 后直接 H3）。
    fn nodes(items: &[(u8, String, usize, f32)], at: &mut usize, parent: u8) -> Vec<OutlineNode> {
        let mut result = Vec::new();
        while let Some((level, text, page, y)) = items.get(*at) {
            if *level <= parent { break; }
            let level = *level;
            let mut node = OutlineNode::new(text.clone(), XyzDestination::new(*page, Point::from_xy(MARGIN, *y)));
            *at += 1;
            for child in nodes(items, at, level) { node.push_child(child); }
            result.push(node);
        }
        result
    }
    for node in nodes(&headings, &mut 0, 0) { outline.push_child(node); } doc.set_outline(outline);
    for draws in pages {
        job.check()?;
        let mut page = doc.start_page_with(PageSettings::from_wh(PAGE_W, PAGE_H).ok_or("PDF 页面尺寸无效")?);
        let mut annotations = Vec::new();
        {
            let mut surface = page.surface();
            rectangle(&mut surface, 0.0, 0.0, PAGE_W, PAGE_H, rgb_bytes(appearance.theme.document_bg));
            for draw in draws {
                job.check()?;
                match draw {
                    Draw::Rect { x, y, w, h, color } => rectangle(&mut surface, x, y, w, h, color),
                    Draw::Item { item, x, y } => match item.kind {
                        ItemKind::Image(id) => {
                            surface.push_transform(&Transform::from_translate(x, y));
                            surface.draw_image(images[id].clone(), Size::from_wh(item.width, item.height).ok_or("图片尺寸无效")?); surface.pop();
                        },
                        ItemKind::Text { text, glyphs, font, size, style } => {
                            let color = style.color.unwrap_or_else(|| if style.link.is_some() { rgb_bytes(appearance.theme.accent) } else { rgb_bytes(appearance.theme.fg) });
                            let baseline = y + item.height * 0.78 - f32::from(style.shift) * size * 0.28;
                            if style.highlight { rectangle(&mut surface, x, y, item.width, item.height, [255, 232, 122]); }
                            else if style.mono { rectangle(&mut surface, x, y, item.width, item.height, rgb_bytes(appearance.theme.inline_code_bg)); }
                            surface.set_fill(Some(fill(color)));
                            surface.draw_glyphs(Point::from_xy(x, baseline), &glyphs, fonts.faces[font].font.clone(), &text, size, false);
                            if style.underline || style.link.is_some() { rectangle(&mut surface, x, baseline + size * 0.12, item.width, 0.55, color); }
                            if style.strike { rectangle(&mut surface, x, baseline - size * 0.28, item.width, 0.55, color); }
                            if let Some(url) = style.link.as_deref().and_then(|url| media::link(url, path)) {
                                let target = if let Some(id) = url.strip_prefix('#') {
                                    let id = percent_decode(id);
                                    destinations.get(&id).map(|(p, y)| Target::Destination(XyzDestination::new(*p, Point::from_xy(MARGIN, *y)).into()))
                                } else { Some(Target::Action(LinkAction::new(url).into())) };
                                if let (Some(target), Some(rect)) = (target, Rect::from_xywh(x, y, item.width.max(0.1), item.height)) {
                                    annotations.push(Annotation::new_link(LinkAnnotation::new(rect, target), Some(text)));
                                }
                            }
                        },
                    }
                }
            }
            surface.finish();
        }
        for annotation in annotations { page.add_annotation(annotation); } page.finish();
    }
    job.check()?;
    let output = doc.finish().map_err(|e| format!("PDF 封装失败：{e:?}"))?;
    job.check()?; Ok(output)
}
pub(super) fn percent_decode(id: &str) -> String {
    // 借助 URL 解析器处理片段中的百分号字节，不将 + 当成空格。
    let mut bytes = Vec::new(); let mut chars = id.as_bytes().iter().copied();
    while let Some(c) = chars.next() {
        if c == b'%' { let a = chars.next(); let b = chars.next();
            if let (Some(a), Some(b)) = (a, b) { if let (Some(a), Some(b)) = ((a as char).to_digit(16), (b as char).to_digit(16)) { bytes.push((a * 16 + b) as u8); continue; } }
            return id.into();
        }
        bytes.push(c);
    }
    String::from_utf8(bytes).unwrap_or_else(|_| id.into())
}
fn fill(c: [u8; 3]) -> Fill { Fill { paint: rgb::Color::new(c[0],c[1],c[2]).into(), opacity: NormalizedF32::ONE, rule: Default::default() } }
fn rectangle(surface: &mut krilla::surface::Surface<'_>, x: f32, y: f32, w: f32, h: f32, color: [u8;3]) {
    if let Some(rect) = Rect::from_xywh(x,y,w,h) { let mut path = PathBuilder::new(); path.push_rect(rect); if let Some(path) = path.finish() { surface.set_fill(Some(fill(color))); surface.draw_path(&path); } }
}
impl Layout<'_> {
    fn add(&mut self, draw: Draw) -> Result<(), String> {
        self.job.check()?;
        self.bytes += 128;
        if let Draw::Item { item: Item { kind: ItemKind::Text { text, glyphs, style, .. }, .. }, .. } = &draw {
            self.glyphs += glyphs.len(); self.bytes += text.len() + glyphs.len() * std::mem::size_of::<KrillaGlyph>() + style.link.as_ref().map_or(0, String::len);
        }
        if self.glyphs > 750_000 || self.bytes > 64 * 1024 * 1024 { return Err("PDF 排版超过 75 万字形／64 MiB 预算，已有输出文件未改动。".into()); }
        self.pages.last_mut().unwrap().push(draw); Ok(())
    }
    fn ensure(&mut self, height: f32) -> Result<(), String> {
        self.job.check()?;
        if self.y + height > PAGE_H - MARGIN && self.y > MARGIN {
            if self.pages.len() >= 1000 { return Err("PDF 超过 1000 页预算。".into()); }
            self.pages.push(Vec::new()); self.y = MARGIN;
        }
        Ok(())
    }
    fn blocks(&mut self, blocks: &[Block]) -> Result<(), String> {
        for block in blocks { self.job.check()?; match block {
            Block::Paragraph(p) => self.paragraph(p)?, Block::Table(t) => self.table(t)?,
            Block::Rule => { self.ensure(18.0)?; self.add(Draw::Rect { x: MARGIN, y: self.y + 6.0, w: WIDTH, h: 0.7, color: rgb_bytes(self.appearance.theme.border) })?; self.y += 18.0; },
            Block::Code { text, language } => {
                let spans = (text.len() <= 256 * 1024).then(|| crate::markdown::highlight::code_block_uncached(text, language));
                for (index, line) in text.lines().enumerate() {
                    let mut runs = Vec::new();
                    if self.appearance.line_numbers { runs.push(Run { content: Content::Text(format!("{:>4}  ", index + 1)), style: Style { mono: true, ..Default::default() } }); }
                    let mut cursor = 0;
                    if let Some(spans) = spans.as_ref().and_then(|s| s.get(index)) { for (range, s) in spans {
                        if range.start > cursor { runs.push(Run { content: Content::Text(line[cursor..range.start].into()), style: Style { mono: true, ..Default::default() } }); }
                        if let Some(value) = line.get(range.clone()) { runs.push(Run { content: Content::Text(value.into()), style: Style { mono: true, bold: s.bold, italic: s.italic, color: Some(rgb_bytes(s.color.resolve(&self.appearance.theme))), ..Default::default() } }); cursor = range.end; }
                    } }
                    if cursor < line.len() || runs.is_empty() { runs.push(Run { content: Content::Text(line[cursor..].into()), style: Style { mono: true, ..Default::default() } }); }
                    let lines = self.lines(&runs, WIDTH - 12.0, (self.appearance.size * 0.7).clamp(7.0, 24.0))?;
                    for line in lines { self.ensure(line.height)?; let h = line.height; self.draw_line(line, MARGIN + 6.0, self.y, WIDTH - 12.0, Alignment::Left)?; self.y += h; }
                }
                self.y += 8.0;
            }
        } }
        Ok(())
    }
    fn lines(&mut self, runs: &[Run], available: f32, size: f32) -> Result<Vec<Line>, String> {
        let mut logical = String::new();
        for run in runs { match &run.content { Content::Raster(_) => logical.push('\u{fffc}'), _ => logical.push_str(run.label()) } }
        if logical.len() > 512 * 1024 { return Err("PDF 单段落超过 512 KiB 排版预算。".into()); }
        let bidi = BidiInfo::new(&logical, None);
        let mut offset = 0; let mut lines = Vec::new(); let mut line = Line { height: size * self.appearance.line_height.clamp(1.0,2.5), ..Default::default() };
        let mut count = 0usize;
        for run in runs {
            self.job.check()?;
            let level = bidi.levels.get(offset).copied().unwrap_or(Level::ltr());
            match &run.content {
                Content::Break => { lines.push(std::mem::take(&mut line)); line.height = size * 1.5; offset += 1; },
                Content::Raster(id) => {
                    let asset = &self.assets[*id];
                    let scale = (available / asset.width).min((HEIGHT - 10.0) / asset.height).min(1.0);
                    if asset.block && !line.items.is_empty() { lines.push(std::mem::take(&mut line)); }
                    let item = Item { kind: ItemKind::Image(*id), width: asset.width*scale, height: asset.height*scale, level, whitespace: false };
                    append(item, &mut line, &mut lines, available);
                    if asset.block { lines.push(std::mem::take(&mut line)); }
                    offset += '\u{fffc}'.len_utf8();
                },
                _ => {
                    let mut style = run.style.clone();
                    let text = match &run.content { Content::Footnote(label) => { style.shift = 1; style.link = Some(format!("#nomo-footnote-{label}")); label.as_str() }, _ => run.label() };
                    let run_size = size * if style.shift != 0 { 0.75 } else { 1.0 };
                    for word in text.split_word_bounds() {
                        if word.contains(['\n','\r']) { for ch in word.chars() { if ch == '\n' { lines.push(std::mem::take(&mut line)); line.height = size*1.5; } offset += ch.len_utf8(); } continue; }
                        let original_length = word.len(); let start = offset;
                        let word = word.replace('\t', "    ");
                        for (font, part) in self.fonts.segments(&word, &style, self.job)? {
                            let level = bidi.levels.get(offset).copied().unwrap_or(level);
                            let (glyphs, width) = self.fonts.shape(font, &part, run_size, level.is_rtl())?;
                            let chunks: Vec<&str> = if width > available { part.graphemes(true).collect() } else { vec![&part] };
                            for (i, chunk) in chunks.iter().enumerate() {
                                count += 1;
                                if count > 100_000 { return Err("单段落排版超过预算。".into()); }
                                let (shaped, width) = if chunks.len() == 1 && i == 0 { (glyphs.clone(), width) } else { self.fonts.shape(font, chunk, run_size, level.is_rtl())? };
                                self.prepared += shaped.len() * std::mem::size_of::<KrillaGlyph>() + chunk.len() + 256;
                                if self.prepared > 64 * 1024 * 1024 { return Err("PDF 待分页字形超过 64 MiB 预算。".into()); }
                                let item = Item { kind: ItemKind::Text { text: chunk.to_string(), glyphs: shaped, font, size: run_size, style: style.clone() }, width,
                                    height: size * self.appearance.line_height.clamp(1.0,2.5), level, whitespace: !style.mono && chunk.chars().all(char::is_whitespace) };
                                append(item, &mut line, &mut lines, available);
                            }
                            offset += part.len();
                        }
                        offset = start + original_length;
                    }
                }
            }
        }
        if !line.items.is_empty() || lines.is_empty() { line.height = line.height.max(size * 1.4); lines.push(line); }
        Ok(lines)
    }
    fn draw_line(&mut self, line: Line, x: f32, y: f32, available: f32, align: Alignment) -> Result<(), String> {
        let mut x = x + match align { Alignment::Center => ((available-line.width)*0.5).max(0.0), Alignment::Right => (available-line.width).max(0.0), _ => 0.0 };
        let levels = line.items.iter().map(|item| item.level).collect::<Vec<_>>();
        let order = BidiInfo::reorder_visual(&levels);
        let mut items: Vec<Option<Item>> = line.items.into_iter().map(Some).collect();
        for i in order { let item = items[i].take().unwrap(); let width = item.width; let offset = (line.height-item.height).max(0.0);
            self.add(Draw::Item { item, x, y: y + offset })?; x += width;
        }
        Ok(())
    }
    fn paragraph(&mut self, p: &Paragraph) -> Result<(), String> {
        let base = (self.appearance.size * 0.75).clamp(8.0, 30.0);
        let size = if p.heading > 0 { base * [2.0,1.6,1.35,1.15,1.0,1.0][usize::from(p.heading-1).min(5)] } else { base };
        let indent = p.indent as f32 * 18.0 + if p.quote { 12.0 } else { 0.0 };
        let mut runs = p.runs.clone();
        if p.heading > 0 { for run in &mut runs { run.style.bold = true; run.style.color = Some(rgb_bytes(self.appearance.theme.heading)); } }
        if let Some(list) = p.list { runs.insert(0, Run::text(list.number.map_or("• ".into(), |n| format!("{n}. ")))); }
        let lines = self.lines(&runs, WIDTH-indent, size)?;
        self.ensure(lines.first().map_or(0.0, |l| l.height) + if p.heading > 0 { base*2.0 } else { 0.0 })?;
        if let Some(id) = &p.id { self.destinations.insert(id.clone(), (self.pages.len()-1, self.y)); }
        if p.heading > 0 { self.headings.push((p.heading, p.runs.iter().map(Run::label).collect(), self.pages.len()-1, self.y)); }
        for line in lines {
            self.ensure(line.height)?; let h = line.height;
            if p.quote { self.add(Draw::Rect { x: MARGIN, y: self.y, w: 2.0, h, color: rgb_bytes(self.appearance.theme.border) })?; }
            let align = if p.runs.len() == 1 { if let Content::Raster(id) = p.runs[0].content {
                match self.assets[id].align.as_str() { "center" => Alignment::Center, "right" => Alignment::Right, _ => p.align }
            } else { p.align } } else { p.align };
            self.draw_line(line, MARGIN+indent, self.y, WIDTH-indent, align)?; self.y += h;
        }
        self.y += if p.heading > 0 { base*0.65 } else { base*0.4 }; Ok(())
    }
    fn table(&mut self, table: &Table) -> Result<(), String> {
        if table.columns == 0 { return Ok(()); }
        let mut widths: Vec<f32> = (0..table.columns).map(|i| table.widths.get(i).copied().unwrap_or(0.0).max(48.0)).collect();
        let total: f32 = widths.iter().sum(); for w in &mut widths { *w = *w / total * WIDTH; }
        if widths.iter().any(|w| *w < 20.0) {
            self.report.warn("过宽表格按单元格顺序展开，完整保留内容");
            for cell in &table.cells {
                self.paragraph(&Paragraph { runs: vec![Run::text(format!("[第 {} 行，第 {} 列]", cell.row+1, cell.column+1))], ..Default::default() })?;
                self.paragraph(&Paragraph { runs: cell.runs.clone(), ..Default::default() })?;
            }
            return Ok(());
        }
        let mut rows = vec![20.0f32; table.rows]; let mut content = Vec::new();
        for cell in &table.cells {
            self.job.check()?;
            let width: f32 = widths[cell.column..cell.column+cell.colspan].iter().sum();
            let mut runs = cell.runs.clone(); if cell.header { for run in &mut runs { run.style.bold = true; } }
            let lines = self.lines(&runs, (width-8.0).max(1.0), (self.appearance.size*0.7).clamp(7.0,20.0))?;
            let height = lines.iter().map(|l| l.height).sum::<f32>() + 8.0;
            let current: f32 = rows[cell.row..cell.row+cell.rowspan].iter().sum();
            if height > current { rows[cell.row+cell.rowspan-1] += height-current; }
            content.push(lines);
        }
        let header_end = if table.cells.iter().filter(|c| c.row == 0).all(|c| c.header && c.rowspan == 1) { 1 } else { 0 };
        let mut row = 0;
        while row < table.rows {
            let mut end = row+1;
            loop { let next = table.cells.iter().filter(|c| c.row >= row && c.row < end).map(|c| c.row+c.rowspan).max().unwrap_or(end); if next <= end { break; } end = next; }
            let height: f32 = rows[row..end].iter().sum();
            if height > HEIGHT - if row > 0 && header_end == 1 { rows[0] } else { 0.0 } {
                self.report.warn("超高表格行按单元格顺序展开，完整保留内容");
                for cell in table.cells.iter().filter(|c| c.row >= row && c.row < end) {
                    self.paragraph(&Paragraph { runs: vec![Run::text(format!("[第 {} 行，第 {} 列]", cell.row+1, cell.column+1))], ..Default::default() })?;
                    self.paragraph(&Paragraph { runs: cell.runs.clone(), ..Default::default() })?;
                }
                row = end; continue;
            }
            let page = self.pages.len(); self.ensure(height)?;
            if self.pages.len() != page && row > 0 && header_end == 1 && rows[0] + height <= HEIGHT { self.draw_rows(table, &widths, &rows, &content, 0, 1)?; }
            self.draw_rows(table, &widths, &rows, &content, row, end)?; row = end;
        }
        self.y += 8.0; Ok(())
    }
    fn draw_rows(&mut self, table: &Table, widths: &[f32], heights: &[f32], content: &[Vec<Line>], from: usize, to: usize) -> Result<(), String> {
        for (index, cell) in table.cells.iter().enumerate().filter(|(_, c)| c.row >= from && c.row < to) {
            let x = MARGIN + widths[..cell.column].iter().sum::<f32>(); let y = self.y + heights[from..cell.row].iter().sum::<f32>();
            let w = widths[cell.column..cell.column+cell.colspan].iter().sum::<f32>(); let h = heights[cell.row..cell.row+cell.rowspan].iter().sum::<f32>();
            if cell.header { self.add(Draw::Rect { x,y,w,h,color: rgb_bytes(self.appearance.theme.surface) })?; }
            let color = rgb_bytes(self.appearance.theme.border);
            for (x,y,w,h) in [(x,y,w,0.5),(x,y+h-0.5,w,0.5),(x,y,0.5,h),(x+w-0.5,y,0.5,h)] { self.add(Draw::Rect { x,y,w,h,color })?; }
            let mut top = y+4.0;
            for line in &content[index] { self.draw_line(line.clone(), x+4.0, top, (w-8.0).max(1.0), cell.align)?; top += line.height; }
        }
        self.y += heights[from..to].iter().sum::<f32>(); Ok(())
    }
}
fn append(item: Item, line: &mut Line, lines: &mut Vec<Line>, available: f32) {
    if !line.items.is_empty() && line.width + item.width > available { lines.push(std::mem::take(line)); }
    if line.items.is_empty() && item.whitespace { return; }
    line.width += item.width; line.height = line.height.max(item.height); line.items.push(item);
}
