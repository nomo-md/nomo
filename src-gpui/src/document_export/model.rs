//! PDF/Word 共用的临时语义模型；只在固定快照的后台导出任务中存在。
use super::{Job, ExportReport};
use crate::{markdown_templates::InlineFormat, nomo_mark::{html_table::{self, InlineAtom, HtmlTable, HtmlFragment}, image_properties::ImageProperties, nomo_syntax}};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{collections::{BTreeMap, BTreeSet}, sync::Arc};

#[derive(Clone, Default, PartialEq)]
pub(super) struct Style {
    pub bold: bool, pub italic: bool, pub mono: bool, pub strike: bool,
    pub underline: bool, pub highlight: bool, pub shift: i8,
    pub color: Option<[u8; 3]>, pub link: Option<String>,
}
impl Style {
    fn format(&mut self, format: InlineFormat) {
        match format {
            InlineFormat::Strong => self.bold = true, InlineFormat::Emphasis => self.italic = true,
            InlineFormat::Code => self.mono = true, InlineFormat::Strike => self.strike = true,
            InlineFormat::Underline => self.underline = true, InlineFormat::Highlight => self.highlight = true,
            InlineFormat::Superscript => self.shift = 1, InlineFormat::Subscript => self.shift = -1,
            InlineFormat::Link => {},
        }
    }
}
#[derive(Clone)]
pub(super) enum Content {
    Text(String), Break, Image(ImageProperties), Math(String, bool), Footnote(String), Raster(usize),
}
#[derive(Clone)]
pub(super) struct Run { pub content: Content, pub style: Style }
impl Run {
    pub fn text(text: impl Into<String>) -> Self { Self { content: Content::Text(text.into()), style: Style::default() } }
    pub fn label(&self) -> &str {
        match &self.content { Content::Text(s) | Content::Math(s, _) | Content::Footnote(s) => s, Content::Image(p) => &p.alt, Content::Break => "\n", Content::Raster(_) => "" }
    }
}
#[derive(Clone)]
pub(super) struct Paragraph {
    pub runs: Vec<Run>, pub heading: u8, pub id: Option<String>, pub indent: usize,
    pub quote: bool, pub align: Alignment, pub list: Option<ListItem>,
}
impl Default for Paragraph {
    fn default() -> Self { Self { runs: Vec::new(), heading: 0, id: None, indent: 0, quote: false, align: Alignment::None, list: None } }
}
#[derive(Clone, Copy)]
pub(super) struct ListItem { pub id: usize, pub level: usize, pub number: Option<u64> }
#[derive(Clone)]
pub(super) struct Cell {
    pub row: usize, pub column: usize, pub rowspan: usize, pub colspan: usize,
    pub header: bool, pub align: Alignment, pub runs: Vec<Run>,
}
pub(super) struct Table { pub rows: usize, pub columns: usize, pub cells: Vec<Cell>, pub widths: Vec<f32> }
pub(super) enum Block { Paragraph(Paragraph), Code { text: String, language: String }, Table(Table), Rule }
#[derive(Default)]
pub(super) struct Model {
    pub blocks: Vec<Block>, pub footnotes: BTreeMap<String, Vec<Block>>,
    pub lists: Vec<Option<u64>>,
}
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH | Options::ENABLE_GFM | Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT
}
pub(super) fn parse(source: &str, job: &Job, report: &mut ExportReport) -> Result<Model, String> {
    let source = &source[nomo_syntax::front_matter_end(source).unwrap_or(0)..];
    let mut builder = Builder { model: Model::default(), paragraph: Paragraph::default(), styles: Vec::new(),
        lists: Vec::new(), quote: 0, pending: None, note: None, slugs: BTreeSet::new(), suffixes: BTreeMap::new(),
        references: Arc::new(BTreeMap::new()), nodes: 0, bytes: 0, events: 0, job, report };
    let parser = Parser::new_ext(source, options());
    let mut reference_bytes = 0;
    for (label, target) in parser.reference_definitions().iter() {
        let bytes = label.len() + target.dest.len() + target.title.as_ref().map_or(0, |v| v.len());
        reference_bytes += bytes;
        if builder.references.len() >= 16_384 || reference_bytes > 2 * 1024 * 1024 { return Err("链接定义超过导出预算。".into()); }
        builder.budget(bytes)?;
        Arc::get_mut(&mut builder.references).unwrap().insert(crate::text_document::normalize_reference_label(label),
            (target.dest.to_string(), target.title.as_ref().map(ToString::to_string).unwrap_or_default()));
    }
    drop(parser);
    builder.markdown(source, 0)?;
    builder.flush()?;
    Ok(builder.model)
}
struct Builder<'a> {
    model: Model, paragraph: Paragraph, styles: Vec<(String, Style)>, lists: Vec<ListItem>,
    quote: usize, pending: Option<ListItem>, note: Option<String>, slugs: BTreeSet<String>,
    suffixes: BTreeMap<String, usize>, references: Arc<BTreeMap<String, (String, String)>>,
    nodes: usize, bytes: usize, events: usize, job: &'a Job<'a>, report: &'a mut ExportReport,
}
impl Builder<'_> {
    fn budget(&mut self, bytes: usize) -> Result<(), String> {
        self.nodes += 1; self.bytes += bytes + 256;
        if self.nodes > 100_000 || self.bytes > 48 * 1024 * 1024 { return Err("导出结构超过预算（10 万节点／48 MiB），原文未改动。".into()); }
        self.job.check()
    }
    fn tick(&mut self) -> Result<(), String> {
        self.events += 1;
        if self.events > 500_000 { return Err("导出解析事件超过预算。".into()); }
        self.job.check()
    }
    fn style(&self) -> Style { self.styles.last().map(|s| s.1.clone()).unwrap_or_default() }
    fn run(&mut self, content: Content) -> Result<(), String> {
        let style = self.style(); self.push_run(Run { content, style })
    }
    fn push_run(&mut self, run: Run) -> Result<(), String> {
        let bytes = match &run.content { Content::Image(p) => p.src.len() + p.alt.len() + p.title.len(), _ => run.label().len() };
        self.budget(bytes + run.style.link.as_ref().map_or(0, String::len))?;
        if self.paragraph.runs.is_empty() {
            self.paragraph.indent = self.lists.len().saturating_sub(1).min(8);
            self.paragraph.quote = self.quote > 0;
            self.paragraph.list = self.pending.take();
        }
        self.paragraph.runs.push(run); Ok(())
    }
    fn block(&mut self, block: Block) -> Result<(), String> {
        self.budget(0)?;
        if let Some(label) = &self.note { self.model.footnotes.entry(label.clone()).or_default().push(block); }
        else { self.model.blocks.push(block); }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        if self.paragraph.runs.is_empty() && self.paragraph.heading == 0 { return Ok(()); }
        let mut p = std::mem::take(&mut self.paragraph);
        if p.heading > 0 {
            let title: String = p.runs.iter().map(Run::label).collect();
            let filtered: String = title.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-').take(8192).collect();
            let slug = filtered.split_whitespace().collect::<Vec<_>>().join("-");
            let base = if slug.is_empty() { format!("heading-{}", self.slugs.len() + 1) } else { slug };
            let mut id = base.clone();
            let suffix = self.suffixes.entry(base.clone()).or_insert(2);
            while self.slugs.contains(&id) { id = format!("{base}-{suffix}"); *suffix += 1; }
            self.slugs.insert(id.clone()); p.id = Some(id);
        }
        self.block(Block::Paragraph(p))
    }
    fn start_style(&mut self, name: &str, format: Option<InlineFormat>, link: Option<String>) -> Result<(), String> {
        if self.styles.len() >= 64 { return Err("行内样式嵌套超过导出预算。".into()); }
        let mut style = self.style();
        if let Some(format) = format { style.format(format); }
        if let Some(link) = link { style.link = Some(link); }
        self.styles.push((name.into(), style)); Ok(())
    }
    fn end_style(&mut self, name: &str) {
        if let Some(at) = self.styles.iter().rposition(|s| s.0 == name) { self.styles.truncate(at); }
    }
    fn markdown(&mut self, source: &str, depth: usize) -> Result<(), String> {
        if depth > 16 { return Err("Details 嵌套超过导出预算。".into()); }
        // 引用表按文档保存；Details 递归仅借用这份有界索引。
        let references = self.references.clone();
        let mut resolve = |link: pulldown_cmark::BrokenLink<'_>| references.get(&crate::text_document::normalize_reference_label(&link.reference))
            .map(|(url, title)| (url.clone().into(), title.clone().into()));
        let mut events = Parser::new_with_broken_link_callback(source, options(), Some(&mut resolve)).into_offset_iter().peekable();
        let mut skip = 0..0;
        let mut table: Option<Table> = None;
        let mut alignments = Vec::new();
        let (mut row, mut column, mut header) = (0, 0, false);
        while let Some((mut event, range)) = events.next() {
            self.tick()?;
            // End 的区间包含整个容器；保留消费区间外已打开的父容器闭合事件。
            if range.start < skip.end && !(matches!(event, Event::End(_)) && range.start <= skip.start) {
                if range.end <= skip.end { continue; }
                if matches!(event, Event::Text(_)) { event = Event::Text(source[skip.end..range.end].into()); } else { continue; }
            }
            match event {
                Event::Start(Tag::Paragraph) => { if table.is_none() { self.flush()?; } },
                Event::End(TagEnd::Paragraph) => { if table.is_none() { self.flush()?; } },
                Event::Start(Tag::Heading { level, .. }) => { self.flush()?; self.paragraph.heading = level as u8; },
                Event::End(TagEnd::Heading(_)) => self.flush()?,
                Event::Start(Tag::BlockQuote(kind)) => {
                    self.flush()?; self.quote += 1;
                    if let Some(kind) = kind { let mut run = Run::text(format!("{kind:?}")); run.style.bold = true; self.push_run(run)?; self.flush()?; }
                },
                Event::End(TagEnd::BlockQuote(_)) => { self.flush()?; self.quote = self.quote.saturating_sub(1); },
                Event::Start(Tag::List(start)) => {
                    self.flush()?;
                    if self.lists.len() >= 9 { return Err("列表嵌套超过 9 层。".into()); }
                    self.model.lists.push(start);
                    self.lists.push(ListItem { id: self.model.lists.len(), level: self.lists.len(), number: start });
                },
                Event::End(TagEnd::List(_)) => { self.flush()?; self.lists.pop(); },
                Event::Start(Tag::Item) => { self.flush()?; self.pending = self.lists.last().copied(); },
                Event::End(TagEnd::Item) => {
                    self.flush()?; self.pending = None;
                    if let Some(item) = self.lists.last_mut() { item.number = item.number.map(|n| n.saturating_add(1)); }
                },
                Event::Start(Tag::CodeBlock(kind)) => {
                    self.flush()?;
                    let language = match kind { CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or("").to_owned(), _ => String::new() };
                    let mut text = String::new();
                    for (event, _) in events.by_ref() { self.tick()?; match event { Event::End(TagEnd::CodeBlock) => break, Event::Text(t) => text.push_str(&t), _ => {} } }
                    self.budget(text.len())?; self.block(Block::Code { text, language })?;
                },
                Event::Start(Tag::Image { dest_url, title, .. }) => {
                    let mut alt = String::new(); let mut end = range.end;
                    for (event, part) in events.by_ref() { self.tick()?; match event {
                        Event::End(TagEnd::Image) => { end = part.end; break; },
                        Event::Text(s) | Event::Code(s) => alt.push_str(&s), Event::SoftBreak | Event::HardBreak => alt.push(' '), _ => {}
                    } }
                    let mut props = ImageProperties { src: dest_url.into_string(), alt, title: title.into_string(), width: String::new(), align: String::new() };
                    if let Some(n) = source[end..].strip_prefix('{').and_then(|s| s.find('}').map(|n| n + 2)).filter(|n| *n <= 512) {
                        if let Some(p) = ImageProperties::read(&source[range.start..end+n]) { props.width = p.width; props.align = p.align; skip = end..end+n; }
                    }
                    self.run(Content::Image(props))?;
                },
                Event::Start(Tag::Emphasis) => self.start_style("em", Some(InlineFormat::Emphasis), None)?,
                Event::Start(Tag::Strong) => self.start_style("strong", Some(InlineFormat::Strong), None)?,
                Event::Start(Tag::Strikethrough) => self.start_style("del", Some(InlineFormat::Strike), None)?,
                Event::Start(Tag::Superscript) => self.start_style("sup", Some(InlineFormat::Superscript), None)?,
                Event::Start(Tag::Subscript) => self.start_style("sub", Some(InlineFormat::Subscript), None)?,
                Event::Start(Tag::Link { dest_url, link_type, .. }) => self.start_style("a", None, Some(if link_type == pulldown_cmark::LinkType::Email { format!("mailto:{dest_url}") } else { dest_url.into_string() }))?,
                Event::End(TagEnd::Emphasis) => self.end_style("em"), Event::End(TagEnd::Strong) => self.end_style("strong"),
                Event::End(TagEnd::Strikethrough) => self.end_style("del"), Event::End(TagEnd::Superscript) => self.end_style("sup"),
                Event::End(TagEnd::Subscript) => self.end_style("sub"), Event::End(TagEnd::Link) => self.end_style("a"),
                Event::Text(t) => {
                    if source.get(range.clone()) == Some(t.as_ref()) {
                        let mut range = range;
                        while let Some((Event::Text(next), part)) = events.peek() {
                            if part.start != range.end || source.get(part.clone()) != Some(next.as_ref()) { break; }
                            range.end = part.end; events.next(); self.tick()?;
                        }
                        if range.len() > 512 * 1024 || source[range.clone()].bytes().filter(|b| matches!(b, b'^' | b'~')).count() > 16_384 {
                            return Err("单段连续文本超过导出解析预算。".into());
                        }
                        let mut expanded = Vec::new();
                        crate::nomo_mark::parse::expand_script_text(source, range, &mut expanded);
                        for (event, _) in expanded { self.tick()?; match event {
                            Event::Text(t) => self.run(Content::Text(t.into_string()))?,
                            Event::Start(Tag::Subscript) => self.start_style("sub", Some(InlineFormat::Subscript), None)?,
                            Event::Start(Tag::Superscript) => self.start_style("sup", Some(InlineFormat::Superscript), None)?,
                            Event::End(TagEnd::Subscript) => self.end_style("sub"), Event::End(TagEnd::Superscript) => self.end_style("sup"), _ => {}
                        } }
                    } else { self.run(Content::Text(t.into_string()))?; }
                },
                Event::Code(t) => { let mut style = self.style(); style.mono = true; self.push_run(Run { content: Content::Text(t.into_string()), style })?; },
                Event::InlineMath(t) => self.run(Content::Math(t.into_string(), false))?,
                Event::DisplayMath(t) => { self.flush()?; self.run(Content::Math(t.into_string(), true))?; self.flush()?; },
                Event::SoftBreak | Event::HardBreak => self.run(Content::Break)?,
                Event::Rule => { self.flush()?; self.block(Block::Rule)?; },
                Event::TaskListMarker(done) => self.run(Content::Text(if done { "☑ " } else { "☐ " }.into()))?,
                Event::FootnoteReference(label) => self.run(Content::Footnote(label.into_string()))?,
                Event::Start(Tag::FootnoteDefinition(label)) => { self.flush()?; self.note = Some(label.into_string()); },
                Event::End(TagEnd::FootnoteDefinition) => { self.flush()?; self.note = None; },
                Event::Start(Tag::Table(align)) => {
                    self.flush()?; alignments = align;
                    if alignments.len() > 64 { return Err("表格超过 64 列。".into()); }
                    table = Some(Table { rows: 0, columns: alignments.len(), cells: Vec::new(), widths: Vec::new() }); row = 0;
                },
                Event::Start(Tag::TableHead) => { header = true; column = 0; },
                Event::End(TagEnd::TableHead) => { header = false; row += 1; },
                Event::Start(Tag::TableRow) => { column = 0; },
                Event::End(TagEnd::TableRow) => { row += 1; if row > 2048 { return Err("表格超过 2048 行。".into()); } },
                Event::Start(Tag::TableCell) => {},
                Event::End(TagEnd::TableCell) => {
                    if let Some(t) = table.as_mut() {
                        let p = std::mem::take(&mut self.paragraph);
                        t.cells.push(Cell { row, column, rowspan: 1, colspan: 1, header, align: alignments.get(column).copied().unwrap_or(Alignment::None), runs: p.runs }); column += 1;
                    }
                },
                Event::End(TagEnd::Table) => { if let Some(mut t) = table.take() { t.rows = row; self.block(Block::Table(t))?; } },
                Event::Start(Tag::HtmlBlock) => {
                    // HTML 事件可能逐行给出；先以完整块范围解析跨行的 Nomo 合并表格。
                    if nomo_syntax::html_tag(source, range.start).is_some_and(|t| t.name == "table" && !t.closing) {
                        if range.len() > 512 * 1024 { return Err("单个 HTML 表格超过 512 KiB 导出预算。".into()); }
                        if let Some(table) = HtmlTable::parse(source, range.clone()) {
                            self.flush()?; self.html_table(table)?; skip = range.clone();
                        }
                    }
                },
                Event::Html(value) | Event::InlineHtml(value) => {
                    if value.len() > 512 * 1024 { return Err("单个 HTML 结构超过 512 KiB 导出预算。".into()); }
                    if let Some(run) = html_table::atom_at(source, range.start).filter(|r| r.atom.is_some()) {
                        skip = run.source.clone(); self.html_run(run)?;
                        if skip.end < range.end { self.raw_html(&source[skip.end..range.end])?; }
                    } else if let Some(details) = nomo_syntax::details(source, range.start) {
                        self.flush()?; self.raw_html(&source[range.start..details.body_from])?; self.flush()?;
                        self.markdown(&source[details.body_from..details.body_to], depth + 1)?; self.flush()?; skip = range.start..details.end;
                    } else if let Some(t) = HtmlTable::parse(source, range.clone()) {
                        self.flush()?; self.html_table(t)?;
                    } else if let Some(fragments) = HtmlFragment::parse_all(source, range.clone()) {
                        self.flush()?;
                        for fragment in fragments { for cell in fragment.cells { for run in cell.runs { self.html_run(run)?; } self.flush()?; } }
                    } else { self.raw_html(&value)?; }
                },
                Event::End(TagEnd::HtmlBlock) => {},
                _ => self.report.warn("部分扩展结构按文本保留"),
            }
        }
        Ok(())
    }
    fn html_run(&mut self, run: html_table::InlineRun) -> Result<(), String> {
        let mut style = self.style(); for format in run.formats { style.format(format); } style.link = run.link.or(style.link);
        let content = match run.atom {
            Some(InlineAtom::Image { src, alt, width, align }) => Content::Image(ImageProperties { src, alt, title: String::new(), width: width.map(|w| match w {
                crate::nomo_mark::image_properties::ImageWidth::Pixels(v) => format!("{v}px"),
                crate::nomo_mark::image_properties::ImageWidth::Percent(v) => format!("{v}%"),
            }).unwrap_or_default(), align: align.unwrap_or_default() }),
            Some(InlineAtom::Math(tex)) => Content::Math(tex, false),
            Some(InlineAtom::Comment(text)) => { style.color = Some([55, 120, 65]); Content::Text(text) },
            Some(InlineAtom::Footnote(label)) => Content::Footnote(label), None => Content::Text(run.text),
        };
        self.push_run(Run { content, style })
    }
    fn html_table(&mut self, t: HtmlTable) -> Result<(), String> {
        if t.columns > 64 || t.rows > 2048 { return Err("表格超过导出预算（2048 行／64 列）。".into()); }
        let mut table = Table { rows: t.rows, columns: t.columns, cells: Vec::new(), widths: vec![0.0; t.columns] };
        for c in t.cells {
            for (offset, width) in c.widths.iter().enumerate() { if let Some(w) = table.widths.get_mut(c.column + offset) { *w = (*width as f32).max(*w); } }
            for run in c.runs { self.html_run(run)?; }
            let p = std::mem::take(&mut self.paragraph);
            table.cells.push(Cell { row: c.row, column: c.column, rowspan: c.rowspan, colspan: c.colspan, header: c.header,
                align: match c.align.as_deref() { Some("center") => Alignment::Center, Some("right") => Alignment::Right, _ => Alignment::Left }, runs: p.runs });
        }
        self.block(Block::Table(table))
    }
    fn raw_html(&mut self, source: &str) -> Result<(), String> {
        let mut at = 0;
        while at < source.len() {
            self.tick()?;
            if source[at..].starts_with("<!--") {
                if let Some(end) = source[at+4..].find("-->") {
                    let text = source[at+4..at+4+end].trim();
                    if !matches!(text, "toc" | "/toc") { let mut r = Run::text(text); r.style.color = Some([55,120,65]); self.push_run(r)?; }
                    at += end+7; continue;
                }
            }
            if let Some(run) = html_table::atom_at(source, at).filter(|r| r.atom.is_some()) { at = run.source.end; self.html_run(run)?; continue; }
            if let Some(tag) = nomo_syntax::html_tag(source, at) {
                at = tag.end;
                let format = match tag.name.as_str() { "b" | "strong" => Some(InlineFormat::Strong), "i" | "em" => Some(InlineFormat::Emphasis),
                    "u" => Some(InlineFormat::Underline), "mark" => Some(InlineFormat::Highlight), "code" => Some(InlineFormat::Code),
                    "s" | "strike" | "del" => Some(InlineFormat::Strike), "sup" => Some(InlineFormat::Superscript), "sub" => Some(InlineFormat::Subscript), _ => None };
                if format.is_some() || tag.name == "a" {
                    if tag.closing { self.end_style(&tag.name); } else { self.start_style(&tag.name, format, tag.attrs.get("href").cloned())?; }
                } else { match tag.name.as_str() {
                    "br" => self.run(Content::Break)?, "hr" => { self.flush()?; self.block(Block::Rule)?; },
                    "p" | "div" | "section" | "details" | "summary" => self.flush()?,
                    "span" => {},
                    _ => { self.report.warn("未支持的 HTML 已保留为源码文本"); self.run(Content::Text(source[..at].rsplit_once('<').map(|(_, t)| format!("<{t}")).unwrap_or_default()))?; }
                } }
                continue;
            }
            let end = source[at..].char_indices().skip(1).find(|(_, c)| *c == '<').map_or(source.len(), |(i, _)| at+i);
            self.run(Content::Text(html_escape::decode_html_entities(&source[at..end]).into_owned()))?; at = end;
        }
        Ok(())
    }
}
