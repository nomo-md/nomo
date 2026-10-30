//! 按事件输出 HTML；原始 HTML 只保留编辑器支持的标签和属性。
use super::{Appearance, ExportReport, Output};
use crate::nomo_mark::{nomo_syntax, html_table::{self, InlineAtom}, image_properties::{ImageProperties, ImageWidth}};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{collections::{BTreeMap, BTreeSet}, path::Path, sync::Arc};

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES | Options::ENABLE_MATH | Options::ENABLE_GFM
        | Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT
}

pub(super) struct Renderer<'a, 'file> {
    pub(super) output: &'a mut Output<'file>,
    pub(super) appearance: &'a Appearance,
    pub(super) path: Option<&'a Path>,
    pub(super) report: &'a mut ExportReport,
    pub(super) resource_bytes: usize,
    pub(super) resource_count: usize,
    pub(super) embed_count: usize,
    slugs: BTreeSet<String>,
    suffixes: BTreeMap<String, usize>,
    headings: usize,
    footnotes: BTreeMap<String, usize>,
    references: Arc<BTreeMap<String, (String, String)>>,
    events: usize,
    suppressed: Option<String>,
    comment: bool,
}
impl<'a, 'file> Renderer<'a, 'file> {
    pub(super) fn new(output: &'a mut Output<'file>, appearance: &'a Appearance, path: Option<&'a Path>, report: &'a mut ExportReport) -> Self {
        Self { output, appearance, path, report, resource_bytes: 0, resource_count: 0, embed_count: 0,
            slugs: BTreeSet::new(), suffixes: BTreeMap::new(), headings: 0, footnotes: BTreeMap::new(),
            references: Arc::new(BTreeMap::new()), events: 0, suppressed: None, comment: false }
    }
    pub(super) fn document(&mut self, source: &str, title: &str) -> Result<(), String> {
        self.output.raw("<!doctype html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: https: http:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>")?;
        self.output.text(title)?;
        self.output.raw("</title><style>")?;
        self.output.raw(&stylesheet(self.appearance))?;
        self.output.raw("</style></head><body><main class=\"nomo-document\">")?;
        let source = &source[nomo_syntax::front_matter_end(source).unwrap_or(0)..];
        let projected = nomo_syntax::project_comment_paragraphs(source, options());
        let parser = Parser::new_ext(&projected, options());
        let mut references = BTreeMap::new();
        let mut bytes = 0;
        for (label, target) in parser.reference_definitions().iter() {
            bytes += label.len() + target.dest.len() + target.title.as_ref().map_or(0, |s| s.len());
            if references.len() >= 16384 || bytes > 2 * 1024 * 1024 { return Err("链接定义超过 HTML 导出预算。".into()); }
            references.insert(crate::text_document::normalize_reference_label(label), (target.dest.to_string(), target.title.as_ref().map(ToString::to_string).unwrap_or_default()));
        }
        drop(parser);
        drop(projected);
        self.references = Arc::new(references);
        self.markdown(source, 0)?;
        if !self.report.warnings.is_empty() {
            self.output.raw("<aside class=\"footnote\"><strong>导出说明</strong><ul>")?;
            for warning in &self.report.warnings { self.output.raw("<li>")?; self.output.text(warning)?; self.output.raw("</li>")?; }
            self.output.raw("</ul></aside>")?;
        }
        self.output.raw("</main></body></html>\n")
    }
    fn tick(&mut self) -> Result<(), String> {
        self.events += 1;
        if self.events > 500_000 { return Err("Markdown 结构超过 HTML 导出预算。".into()); }
        self.output.check()
    }
    fn markdown(&mut self, source: &str, depth: usize) -> Result<(), String> {
        if depth > 16 { return Err("Details 嵌套超过 HTML 导出预算。".into()); }
        let references = self.references.clone();
        let mut resolve = |link: pulldown_cmark::BrokenLink<'_>| references.get(&crate::text_document::normalize_reference_label(&link.reference))
            .map(|(url, title)| (url.clone().into(), title.clone().into()));
        let projected = nomo_syntax::project_comment_paragraphs(source, options());
        let mut events = Parser::new_with_broken_link_callback(&projected, options(), Some(&mut resolve))
            .into_offset_iter().map(|(event, range)| (nomo_syntax::restore_inline_comment(source, event, &range), range));
        let mut skip = 0..0;
        let mut table: Vec<(Vec<Alignment>, bool, usize)> = Vec::new();
        while let Some((mut event, range)) = events.next() {
            self.tick()?;
            // End 的区间包含整个容器；保留消费区间外已打开的父容器闭合事件。
            if range.start < skip.end && !(matches!(event, Event::End(_)) && range.start <= skip.start) {
                if range.end <= skip.end { continue; }
                // 图片尾随属性可能与同一文本事件合并，只跳过属性本身。
                if matches!(event, Event::Text(_)) { event = Event::Text(source[skip.end..range.end].into()); } else { continue; }
            }
            if self.suppressed.is_some() || self.comment {
                if let Event::Html(value) | Event::InlineHtml(value) = event { self.raw_html(&value)?; }
                continue;
            }
            match event {
                Event::Start(Tag::CodeBlock(kind)) => {
                    let language = match kind { CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or("").to_owned(), _ => String::new() };
                    let mut code = String::new();
                    for (event, _) in events.by_ref() {
                        self.tick()?;
                        match event { Event::End(TagEnd::CodeBlock) => break, Event::Text(value) => code.push_str(&value), _ => {} }
                    }
                    self.code(&code, &language)?;
                }
                Event::Start(Tag::Image { dest_url, title, .. }) => {
                    let mut alt = String::new();
                    let mut end = range.end;
                    for (event, part) in events.by_ref() {
                        self.tick()?;
                        match event {
                            Event::End(TagEnd::Image) => { end = part.end; break; }
                            Event::Text(value) | Event::Code(value) => alt.push_str(&value),
                            Event::SoftBreak | Event::HardBreak => alt.push(' '), _ => {}
                        }
                    }
                    let mut props = ImageProperties { src: dest_url.into_string(), alt, title: title.into_string(), width: String::new(), align: String::new() };
                    if let Some(attributes) = source[end..].strip_prefix('{').and_then(|s| s.find('}').map(|n| n + 2)).filter(|n| *n <= 512) {
                        if let Some(parsed) = ImageProperties::read(&source[range.start..end + attributes]) {
                            props.width = parsed.width; props.align = parsed.align; skip = end..end + attributes;
                        }
                    }
                    self.image(&props)?;
                }
                Event::Start(Tag::Paragraph) => self.output.raw("<p>")?,
                Event::End(TagEnd::Paragraph) => self.output.raw("</p>\n")?,
                Event::Start(Tag::Heading { level, .. }) => {
                    let id = self.heading_id(&source[range.clone()])?;
                    self.output.raw(&format!("<h{} id=\"", level as u8))?; self.output.attr(&id)?; self.output.raw("\">")?;
                }
                Event::End(TagEnd::Heading(level)) => self.output.raw(&format!("</h{}>\n", level as u8))?,
                Event::Start(Tag::BlockQuote(kind)) => {
                    let class = kind.map(|kind| format!("callout-{}", format!("{kind:?}").to_ascii_lowercase())).unwrap_or_default();
                    self.output.raw("<blockquote class=\"")?; self.output.attr(&class)?; self.output.raw("\">")?;
                    if let Some(kind) = kind { self.output.raw("<strong class=\"callout-title\">")?; self.output.text(&format!("{kind:?}"))?; self.output.raw("</strong>")?; }
                }
                Event::End(TagEnd::BlockQuote(_)) => self.output.raw("</blockquote>\n")?,
                Event::Start(Tag::List(start)) => self.output.raw(&start.map_or("<ul>".into(), |n| format!("<ol start=\"{n}\">")))?,
                Event::End(TagEnd::List(ordered)) => self.output.raw(if ordered { "</ol>\n" } else { "</ul>\n" })?,
                Event::Start(Tag::Item) => self.output.raw("<li>")?, Event::End(TagEnd::Item) => self.output.raw("</li>\n")?,
                Event::Start(Tag::Emphasis) => self.output.raw("<em>")?, Event::End(TagEnd::Emphasis) => self.output.raw("</em>")?,
                Event::Start(Tag::Strong) => self.output.raw("<strong>")?, Event::End(TagEnd::Strong) => self.output.raw("</strong>")?,
                Event::Start(Tag::Strikethrough) => self.output.raw("<del>")?, Event::End(TagEnd::Strikethrough) => self.output.raw("</del>")?,
                Event::Start(Tag::Superscript) => self.output.raw("<sup>")?, Event::End(TagEnd::Superscript) => self.output.raw("</sup>")?,
                Event::Start(Tag::Subscript) => self.output.raw("<sub>")?, Event::End(TagEnd::Subscript) => self.output.raw("</sub>")?,
                Event::Start(Tag::Link { dest_url, title, link_type, .. }) => {
                    let url = if link_type == pulldown_cmark::LinkType::Email { format!("mailto:{dest_url}") } else { dest_url.into_string() };
                    self.output.raw("<a href=\"")?; self.output.attr(&self.link(&url).unwrap_or_default())?;
                    self.output.raw("\" title=\"")?; self.output.attr(&title)?; self.output.raw("\">")?;
                }
                Event::End(TagEnd::Link) => self.output.raw("</a>")?,
                Event::Start(Tag::Table(align)) => { table.push((align, false, 0)); self.output.raw("<div class=\"table-scroll\"><table>")?; }
                Event::Start(Tag::TableHead) => { if let Some(t) = table.last_mut() { t.1 = true; t.2 = 0; } self.output.raw("<thead><tr>")?; }
                Event::End(TagEnd::TableHead) => { if let Some(t) = table.last_mut() { t.1 = false; } self.output.raw("</tr></thead><tbody>")?; }
                Event::Start(Tag::TableRow) => { if let Some(t) = table.last_mut() { t.2 = 0; } self.output.raw("<tr>")?; }
                Event::End(TagEnd::TableRow) => self.output.raw("</tr>")?,
                Event::Start(Tag::TableCell) => {
                    if let Some((align, head, column)) = table.last_mut() {
                        self.output.raw(if *head { "<th" } else { "<td" })?;
                        let align = match align.get(*column) { Some(Alignment::Left) => "left", Some(Alignment::Center) => "center", Some(Alignment::Right) => "right", _ => "" };
                        self.output.raw(&format!(" style=\"text-align:{align}\">"))?;
                    }
                }
                Event::End(TagEnd::TableCell) => if let Some((_, head, column)) = table.last_mut() { self.output.raw(if *head { "</th>" } else { "</td>" })?; *column += 1; },
                Event::End(TagEnd::Table) => { table.pop(); self.output.raw("</tbody></table></div>\n")?; }
                Event::Start(Tag::FootnoteDefinition(label)) => {
                    let id = self.footnote(&label)?;
                    self.output.raw(&format!("<aside class=\"footnote\" id=\"nomo-footnote-{id}\"><sup>"))?;
                    self.output.text(&label)?; self.output.raw("</sup> ")?;
                }
                Event::End(TagEnd::FootnoteDefinition) => self.output.raw("</aside>\n")?,
                Event::FootnoteReference(label) => self.footnote_link(&label)?,
                Event::Text(value) => self.output.text(&value)?,
                Event::Code(value) => { self.output.raw("<code>")?; self.output.text(&value)?; self.output.raw("</code>")?; }
                Event::InlineMath(value) => self.math(&value, false)?, Event::DisplayMath(value) => self.math(&value, true)?,
                Event::Html(value) | Event::InlineHtml(value) => {
                    if let Some(run) = html_table::atom_at(source, range.start).filter(|run| run.atom.is_some()) {
                        self.atom(run.atom.unwrap())?;
                        skip = run.source.clone();
                        if skip.end < range.end { self.raw_html(&source[skip.end..range.end])?; }
                    } else if let Some(details) = nomo_syntax::details(source, range.start) {
                        self.raw_html(&source[range.start..details.body_from])?;
                        self.markdown(&source[details.body_from..details.body_to], depth + 1)?;
                        self.raw_html(&source[details.body_to..details.end])?;
                        skip = range.start..details.end;
                    } else { self.raw_html(&value)?; }
                }
                Event::SoftBreak | Event::HardBreak => self.output.raw("<br>\n")?, Event::Rule => self.output.raw("<hr>\n")?,
                Event::TaskListMarker(done) => self.output.raw(if done { "<input type=\"checkbox\" disabled checked> " } else { "<input type=\"checkbox\" disabled> " })?,
                Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => {},
                _ => self.report.warn("部分扩展结构按文本导出"),
            }
        }
        Ok(())
    }
    fn heading_id(&mut self, source: &str) -> Result<String, String> {
        self.headings += 1;
        if self.headings > 16384 { return Err("标题数量超过 HTML 导出预算。".into()); }
        let mut title = String::new();
        for event in Parser::new_ext(source, options()) {
            match event {
                Event::Text(value) | Event::Code(value) | Event::InlineMath(value) => title.extend(value.chars().take(8192usize.saturating_sub(title.len()))),
                Event::SoftBreak | Event::HardBreak => title.push(' '), _ => {}
            }
        }
        let filtered: String = title.to_lowercase().chars().filter(|ch| ch.is_alphanumeric() || ch.is_whitespace() || *ch == '-').collect();
        let base = filtered.split_whitespace().collect::<Vec<_>>().join("-");
        let base = if base.is_empty() { format!("heading-{}", self.headings) } else { base };
        let mut slug = base.clone();
        let suffix = self.suffixes.entry(base.clone()).or_insert(2);
        while self.slugs.contains(&slug) { slug = format!("{base}-{suffix}"); *suffix += 1; }
        self.slugs.insert(slug.clone());
        Ok(slug)
    }
    fn footnote(&mut self, label: &str) -> Result<usize, String> {
        if let Some(id) = self.footnotes.get(label) { return Ok(*id); }
        if self.footnotes.len() >= 16384 { return Err("脚注数量超过 HTML 导出预算。".into()); }
        let next = self.footnotes.len() + 1;
        self.footnotes.insert(label.to_owned(), next); Ok(next)
    }
    fn footnote_link(&mut self, label: &str) -> Result<(), String> {
        let id = self.footnote(label)?;
        self.output.raw(&format!("<sup><a href=\"#nomo-footnote-{id}\">"))?;
        self.output.text(label)?; self.output.raw("</a></sup>")
    }
    fn atom(&mut self, atom: InlineAtom) -> Result<(), String> {
        match atom {
            InlineAtom::Math(source) => self.math(&source, false),
            InlineAtom::Comment(content) => {
                if matches!(content.trim(), "toc" | "/toc") { return Ok(()); }
                self.output.raw("<span class=\"comment\" title=\"注释\">")?;
                self.output.text(if content.is_empty() { "注释" } else { &content })?;
                self.output.raw("</span>")
            }
            InlineAtom::Footnote(label) => self.footnote_link(&label),
            InlineAtom::Image { src, alt, width, align } => self.image(&ImageProperties { src, alt, title: String::new(),
                width: width.map(|width| match width { ImageWidth::Pixels(n) => format!("{n}px"), ImageWidth::Percent(n) => format!("{n}%") }).unwrap_or_default(), align: align.unwrap_or_default() }),
        }
    }
    fn raw_html(&mut self, input: &str) -> Result<(), String> {
        let mut at = 0;
        while at < input.len() {
            self.tick()?;
            if !self.comment && self.suppressed.is_none() {
                if let Some(run) = html_table::atom_at(input, at).filter(|run| run.atom.is_some()) {
                    self.atom(run.atom.unwrap())?; at = run.source.end; continue;
                }
            }
            if self.suppressed.is_none() && (self.comment || input[at..].starts_with("<!--")) {
                let from = at + if self.comment { 0 } else { 4 };
                if let Some(end) = input[from..].find("-->") { at = from + end + 3; self.comment = false; continue; }
                self.comment = true; break;
            }
            if let Some(tag) = nomo_syntax::html_tag(input, at) {
                at = tag.end;
                if let Some(name) = &self.suppressed { if tag.closing && &tag.name == name { self.suppressed = None; } continue; }
                if matches!(tag.name.as_str(), "script" | "style" | "iframe" | "object" | "textarea" | "xmp") {
                    if !tag.closing { self.suppressed = Some(tag.name); } continue;
                }
                if tag.name == "img" && !tag.closing {
                    let style_width = tag.attrs.get("style").and_then(|style| style.split(';').filter_map(|part| part.split_once(':')).find(|(key, _)| key.trim().eq_ignore_ascii_case("width")).map(|(_, value)| value.trim().to_owned()));
                    let props = ImageProperties { src: tag.attrs.get("src").cloned().unwrap_or_default(), alt: tag.attrs.get("alt").cloned().unwrap_or_default(), title: tag.attrs.get("title").cloned().unwrap_or_default(), width: tag.attrs.get("width").cloned().or(style_width).unwrap_or_default(), align: tag.attrs.get("data-align").or_else(|| tag.attrs.get("align")).cloned().unwrap_or_default() };
                    self.image(&props)?; continue;
                }
                if !matches!(tag.name.as_str(), "p" | "div" | "span" | "br" | "strong" | "b" | "em" | "i" | "u" | "s" | "del" | "mark" | "sub" | "sup" | "pre" | "code" | "blockquote" | "ul" | "ol" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "a" | "table" | "tbody" | "thead" | "tr" | "td" | "th" | "hr" | "details" | "summary" | "colgroup" | "col") { continue; }
                if tag.closing { self.output.raw(&format!("</{}>", tag.name))?; continue; }
                self.output.raw(&format!("<{}", tag.name))?;
                for (name, value) in &tag.attrs {
                    let value = match name.as_str() {
                        "href" if tag.name == "a" => self.link(value),
                        "title" => Some(value.clone()),
                        "open" if tag.name == "details" => Some(String::new()),
                        "rowspan" | "colspan" | "span" | "start" if value.parse::<u16>().is_ok() => Some(value.clone()),
                        "style" => Some(safe_style(value)),
                        "data-align" | "align" if matches!(value.as_str(), "left" | "center" | "right") => Some(value.clone()),
                        _ => None,
                    };
                    if let Some(value) = value { self.output.raw(&format!(" {name}=\""))?; self.output.attr(&value)?; self.output.raw("\"")?; }
                }
                self.output.raw(">")?;
            } else {
                let step = input[at..].chars().next().unwrap().len_utf8();
                let end = input[at + step..].find('<').map_or(input.len(), |n| at + step + n);
                if self.suppressed.is_none() { self.output.text(&html_escape::decode_html_entities(&input[at..end]))?; }
                at = end;
            }
        }
        Ok(())
    }
}

fn safe_style(input: &str) -> String {
    input.split(';').filter_map(|entry| {
        let (key, value) = entry.split_once(':')?;
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let allowed = match key.as_str() {
            "text-align" => matches!(value, "left" | "center" | "right" | "justify"),
            "color" | "background-color" => value.strip_prefix('#').is_some_and(|s| matches!(s.len(), 3 | 4 | 6 | 8) && s.bytes().all(|c| c.is_ascii_hexdigit())) || matches!(value, "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray" | "black" | "white" | "transparent"),
            "width" | "max-width" => crate::image_import::normalize_width(value).is_some(),
            _ => false,
        };
        allowed.then(|| format!("{key}:{value}"))
    }).collect::<Vec<_>>().join(";")
}

pub(super) fn color(value: gpui::Hsla) -> String {
    let value = gpui::Rgba::from(value);
    format!("rgba({},{},{},{:.3})", (value.r * 255.0).round() as u8, (value.g * 255.0).round() as u8, (value.b * 255.0).round() as u8, value.a)
}
fn css_font(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars().take(256) {
        if ch.is_alphanumeric() || ch == ' ' || ch == '-' { escaped.push(ch); }
        else { escaped.push_str(&format!("\\{:x} ", ch as u32)); }
    }
    escaped.push('"'); escaped
}
fn stylesheet(a: &Appearance) -> String {
    let t = &a.theme;
    format!("html{{color-scheme:{scheme};background:{bg};color:{fg};font-family:{font},sans-serif;font-size:{size}px;line-height:{line}}}body{{margin:0}}.nomo-document{{box-sizing:border-box;max-width:{width}px;margin:auto;padding:48px 32px;overflow-wrap:anywhere;text-align:{align}}}h1,h2,h3,h4,h5,h6{{color:{heading};line-height:1.35;margin:1.5em 0 .65em}}h1{{font-size:2em}}h2{{font-size:1.5em}}h3{{font-size:1.25em}}p{{margin:.75em 0}}a{{color:{link}}}img{{max-width:100%;height:auto}}code,pre{{font-family:{mono},monospace}}code{{background:{inline_bg};color:{inline_fg};border-radius:4px;padding:.15em .3em}}pre{{white-space:pre;overflow:auto;text-align:left;padding:16px;margin:0;font-size:.875em;line-height:1.65}}pre code{{background:none;color:inherit;padding:0}}.code-block{{border:1px solid {code_border};background:{code_bg};color:{code_fg};border-radius:8px;margin:1em 0;overflow:hidden}}.code-language{{padding:6px 16px;border-bottom:1px solid {code_border};font:12px {mono};opacity:.75;text-align:left}}.line-number{{display:inline-block;min-width:3em;margin-right:1em;opacity:.4;text-align:right;user-select:none}}blockquote{{border-left:4px solid {quote_border};color:{quote_fg};padding:.2em 1em;margin:1em 0}}.comment{{display:inline-block;color:{comment_fg};background:rgba(22,163,74,.08);padding:.1em .4em;border-radius:4px;white-space:pre-wrap;font-size:.85em}}.callout-title{{display:block}}.callout-note{{border-color:{note}}}.callout-tip{{border-color:{tip}}}.callout-important{{border-color:{important}}}.callout-warning{{border-color:{warning}}}.callout-caution{{border-color:{caution}}}.table-scroll{{overflow:auto}}table{{border-collapse:collapse;width:100%;margin:1em 0;text-align:left}}th,td{{border:1px solid {table_border};padding:8px 12px}}th{{background:{table_head};font-weight:800}}mark{{background:{mark};color:inherit}}hr{{border:0;border-top:1px solid {border};margin:1.5em 0}}details{{border:1px solid {border};border-radius:8px;padding:12px 16px;margin:1em 0}}summary{{cursor:pointer;font-weight:600}}.math-block,.mermaid{{display:block;text-align:center;overflow:auto;margin:1em 0}}.math-inline{{vertical-align:middle;display:inline-block}}.math-inline img{{max-width:none}}.footnote{{font-size:.85em;color:{muted};border-top:1px solid {border};margin-top:1em}}.resource-missing{{display:inline-block;border:1px dashed {border};padding:.3em .6em;color:{muted}}}[data-align=left]{{text-align:left}}[data-align=center]{{text-align:center}}[data-align=right]{{text-align:right}}@media(max-width:600px){{.nomo-document{{padding:24px 16px}}}}@media print{{html{{background:white}}.nomo-document{{max-width:none;padding:0}}pre{{white-space:pre-wrap}}.table-scroll{{overflow:visible}}}}",
        scheme=if t.dark { "dark" } else { "light" }, bg=color(t.document_bg), fg=color(t.fg), font=css_font(&a.font), size=a.size.clamp(10.0,40.0), line=a.line_height.clamp(1.2,2.4), width=(1440.0*a.width).clamp(480.0,1440.0), align=if a.justify { "justify" } else { "start" }, heading=color(t.heading), comment_fg=color(t.success), link=color(t.link), mono=css_font(&a.mono), inline_bg=color(t.inline_code_bg), inline_fg=color(t.inline_code_fg), code_border=color(t.code_border), code_bg=color(t.code_bg), code_fg=color(t.code_fg), quote_border=color(t.quote_border), quote_fg=color(t.quote_fg), note=color(t.callout_note), tip=color(t.callout_tip), important=color(t.callout_important), warning=color(t.callout_warning), caution=color(t.callout_caution), table_border=color(t.table_border), table_head=color(t.table_header_bg), mark=color(t.mark_bg), border=color(t.border), muted=color(t.muted))
}
