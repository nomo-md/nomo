//! 原生 OOXML：可编辑段落、列表、超链接、表格与脚注。媒体内嵌，无外部模板。
use super::{ExportReport, Job, MAX_OUTPUT_BYTES, media::{self, Asset}, model::*};
use image::ImageEncoder;
use pulldown_cmark::Alignment;
use std::{collections::BTreeMap, fs::File, io::{self, Seek, SeekFrom, Write}, path::Path};
use zip::{ZipWriter, CompressionMethod, write::SimpleFileOptions};

const NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";
const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
pub(super) fn write(file: &mut File, model: &Model, assets: Vec<Asset>, title: &str, path: Option<&Path>, job: &Job, report: &mut ExportReport) -> Result<(), String> {
    let mut writer = ZipWriter::new(LimitedFile { file, position: 0, job });
    let mut document = Word { xml: String::new(), job, path, assets: &assets, links: BTreeMap::new(), bookmarks: BTreeMap::new(),
        bookmark_id: 0, drawing_id: 0, footnotes: BTreeMap::new(), in_footnote: false, available: 481.9, report };
    visit(&model.blocks, &mut |p| {
        if let Some(id) = &p.id { let number = document.bookmarks.len()+1; document.bookmarks.entry(id.clone()).or_insert_with(|| format!("nomo_{number}")); }
        for run in &p.runs { if let Content::Footnote(label) = &run.content { if model.footnotes.contains_key(label) { let id = document.footnotes.len()+1; document.footnotes.entry(label.clone()).or_insert(id); } } }
    });
    for blocks in model.footnotes.values() { visit(blocks, &mut |p| { if let Some(id) = &p.id { let number = document.bookmarks.len()+1; document.bookmarks.entry(id.clone()).or_insert_with(|| format!("nomo_{number}")); } }); }
    document.raw(HEADER)?; document.raw(&format!("<w:document {NS}><w:body>"))?;
    document.blocks(&model.blocks)?;
    for (label, blocks) in &model.footnotes { if !document.footnotes.contains_key(label) {
        document.paragraph(&Paragraph { runs: vec![Run::text(format!("[{label}]"))], ..Default::default() })?; document.blocks(blocks)?;
    } }
    document.raw("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>")?;
    part(&mut writer, "word/document.xml", document.xml.as_bytes(), job)?; document.xml.clear();
    if !document.footnotes.is_empty() {
        document.in_footnote = true;
        document.raw(HEADER)?; document.raw(&format!("<w:footnotes {NS}><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>"))?;
        let notes = document.footnotes.clone();
        for (label, id) in notes {
            document.raw(&format!("<w:footnote w:id=\"{id}\"><w:p><w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r></w:p>"))?;
            document.blocks(&model.footnotes[&label])?;
            document.raw("</w:footnote>")?;
        }
        document.raw("</w:footnotes>")?; part(&mut writer, "word/footnotes.xml", document.xml.as_bytes(), job)?; document.xml.clear();
    }
    document.raw(HEADER)?; document.raw("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">")?;
    for (id, kind, target) in [("styles", "styles", "styles.xml"), ("numbering", "numbering", "numbering.xml")] { document.raw(&format!("<Relationship Id=\"{id}\" Type=\"{REL}{kind}\" Target=\"{target}\"/>"))?; }
    if !document.footnotes.is_empty() { document.raw(&format!("<Relationship Id=\"footnotes\" Type=\"{REL}footnotes\" Target=\"footnotes.xml\"/>"))?; }
    let links = document.links.clone();
    for (url, id) in links { document.raw(&format!("<Relationship Id=\"link{id}\" Type=\"{REL}hyperlink\" Target=\""))?; document.text(&url)?; document.raw("\" TargetMode=\"External\"/>")?; }
    for id in 0..assets.len() { document.raw(&format!("<Relationship Id=\"image{id}\" Type=\"{REL}image\" Target=\"media/image{id}.png\"/>"))?; }
    document.raw("</Relationships>")?;
    part(&mut writer, "word/_rels/document.xml.rels", document.xml.as_bytes(), job)?;
    // 两个部件的相对路径都以 word/ 为根；脚注中的图像和链接共用同一组 ID。
    if !document.footnotes.is_empty() { part(&mut writer, "word/_rels/footnotes.xml.rels", document.xml.as_bytes(), job)?; }
    let has_notes = !document.footnotes.is_empty(); drop(document);
    part(&mut writer, "word/styles.xml", styles().as_bytes(), job)?;
    let mut numbering = format!("{HEADER}<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">");
    for (index, start) in model.lists.iter().enumerate() {
        let id = index+1; numbering.push_str(&format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"multilevel\"/>"));
        for level in 0..9 { let left = (level+1)*360;
            numbering.push_str(&format!("<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"{}\"/><w:numFmt w:val=\"{}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:tabs><w:tab w:val=\"num\" w:pos=\"{left}\"/></w:tabs><w:ind w:left=\"{left}\" w:hanging=\"240\"/></w:pPr></w:lvl>", start.unwrap_or(1).min(2_147_483_647), if start.is_some() { "decimal" } else { "bullet" }, if start.is_some() { format!("%{}.", level+1) } else { "•".into() }));
        }
        numbering.push_str("</w:abstractNum>");
        job.check()?;
        if numbering.len() > 16 * 1024 * 1024 { return Err("Word 列表定义超过预算。".into()); }
    }
    // OOXML 要求 abstractNum 集合在 num 集合之前。
    for index in 0..model.lists.len() { let id = index+1; numbering.push_str(&format!("<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{id}\"/></w:num>")); }
    numbering.push_str("</w:numbering>"); part(&mut writer, "word/numbering.xml", numbering.as_bytes(), job)?;
    let types = format!("{HEADER}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>{}</Types>", if has_notes { "<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>" } else { "" });
    part(&mut writer, "[Content_Types].xml", types.as_bytes(), job)?;
    part(&mut writer, "_rels/.rels", format!("{HEADER}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"document\" Type=\"{REL}officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"core\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>").as_bytes(), job)?;
    let core = format!("{HEADER}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>Nomo</dc:creator></cp:coreProperties>", xml(title));
    part(&mut writer, "docProps/core.xml", core.as_bytes(), job)?;
    for (id, asset) in assets.into_iter().enumerate() {
        job.check()?;
        writer.start_file(format!("word/media/image{id}.png"), options()).map_err(|e| e.to_string())?;
        let (w,h) = asset.pixels.dimensions();
        image::codecs::png::PngEncoder::new(&mut writer).write_image(&asset.pixels, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())?; job.check()
}
fn visit(blocks: &[Block], f: &mut impl FnMut(&Paragraph)) { for b in blocks { match b {
    Block::Paragraph(p) => f(p), Block::Table(t) => for c in &t.cells { f(&Paragraph { runs: c.runs.clone(), ..Default::default() }); }, _ => {}
} } }
fn options() -> SimpleFileOptions { SimpleFileOptions::default().compression_method(CompressionMethod::Deflated) }
fn part<W: Write+Seek>(writer: &mut ZipWriter<W>, name: &str, data: &[u8], job: &Job) -> Result<(), String> {
    job.check()?; writer.start_file(name, options()).map_err(|e| e.to_string())?;
    for chunk in data.chunks(64 * 1024) { job.check()?; writer.write_all(chunk).map_err(|e| e.to_string())?; } Ok(())
}
struct LimitedFile<'a> { file: &'a mut File, position: u64, job: &'a Job<'a> }
impl Write for LimitedFile<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.job.check().map_err(io::Error::other)?;
        if self.position.saturating_add(bytes.len() as u64) > MAX_OUTPUT_BYTES { return Err(io::Error::other("Word 文件超过 128 MiB")); }
        let n = self.file.write(bytes)?; self.position += n as u64; Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> { self.file.flush() }
}
impl Seek for LimitedFile<'_> { fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
    self.job.check().map_err(io::Error::other)?; let position = self.file.seek(to)?;
    if position > MAX_OUTPUT_BYTES { return Err(io::Error::other("Word 文件定位超过预算")); } self.position = position; Ok(position)
} }
struct Word<'a> {
    xml: String, job: &'a Job<'a>, path: Option<&'a Path>, assets: &'a [Asset], links: BTreeMap<String, usize>, bookmarks: BTreeMap<String,String>,
    bookmark_id: usize, drawing_id: usize, footnotes: BTreeMap<String, usize>, in_footnote: bool, available: f32, report: &'a mut ExportReport,
}
impl Word<'_> {
    fn raw(&mut self, value: &str) -> Result<(), String> {
        self.job.check()?;
        if self.xml.len() + value.len() > 64 * 1024 * 1024 { return Err("Word XML 部件超过 64 MiB 预算。".into()); }
        self.xml.push_str(value); Ok(())
    }
    fn text(&mut self, mut value: &str) -> Result<(), String> {
        while !value.is_empty() { let mut end = value.len().min(64 * 1024); while !value.is_char_boundary(end) { end -= 1; }
            self.raw(&xml(&value[..end]))?; value = &value[end..]; } Ok(())
    }
    fn blocks(&mut self, blocks: &[Block]) -> Result<(), String> {
        for block in blocks { match block {
            Block::Paragraph(p) => self.paragraph(p)?, Block::Table(t) => self.table(t)?,
            Block::Rule => self.raw("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"D9DDE3\"/></w:pBdr></w:pPr></w:p>")?,
            Block::Code { text, .. } => { for line in text.lines() { self.paragraph(&Paragraph { runs: vec![Run { content: Content::Text(line.into()), style: Style { mono: true, ..Default::default() } }], ..Default::default() })?; } },
        } } Ok(())
    }
    fn paragraph(&mut self, p: &Paragraph) -> Result<(), String> {
        self.raw("<w:p><w:pPr>")?;
        if p.heading > 0 { self.raw(&format!("<w:pStyle w:val=\"Heading{}\"/>", p.heading))?; }
        if let Some(item) = p.list { self.raw(&format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>", item.level, item.id))?; }
        if p.quote { self.raw("<w:pBdr><w:left w:val=\"single\" w:sz=\"12\" w:space=\"6\" w:color=\"D9DDE3\"/></w:pBdr>")?; }
        if p.list.is_none() && (p.indent > 0 || p.quote) { self.raw(&format!("<w:ind w:left=\"{}\"/>", p.indent*360 + if p.quote { 240 } else { 0 }))?; }
        let align = if p.runs.len() == 1 { if let Content::Raster(id) = p.runs[0].content { match self.assets[id].align.as_str() { "center" => Alignment::Center, "right" => Alignment::Right, _ => p.align } } else { p.align } } else { p.align };
        self.raw(&format!("<w:jc w:val=\"{}\"/>", alignment(align)))?; self.raw("</w:pPr>")?;
        let bookmark = p.id.as_ref().and_then(|id| self.bookmarks.get(id)).cloned();
        let id = self.bookmark_id; if let Some(name) = bookmark.as_ref() { self.bookmark_id += 1; self.raw(&format!("<w:bookmarkStart w:id=\"{id}\" w:name=\"{name}\"/>"))?; }
        for run in &p.runs { self.run(run)?; }
        if bookmark.is_some() { self.raw(&format!("<w:bookmarkEnd w:id=\"{id}\"/>"))?; }
        self.raw("</w:p>")
    }
    fn run(&mut self, run: &Run) -> Result<(), String> {
        let link = run.style.link.as_deref().and_then(|s| media::link(s, self.path));
        let mut hyperlink = false;
        if let Some(url) = link {
            if let Some(id) = url.strip_prefix('#') {
                if let Some(name) = self.bookmarks.get(&super::pdf::percent_decode(id)).cloned() { self.raw(&format!("<w:hyperlink w:anchor=\"{name}\" w:history=\"1\">"))?; hyperlink = true; }
            } else {
                if self.links.len() >= 16_384 { return Err("Word 超链接超过预算。".into()); }
                let next = self.links.len()+1; let id = *self.links.entry(url).or_insert(next);
                self.raw(&format!("<w:hyperlink r:id=\"link{id}\" w:history=\"1\">"))?; hyperlink = true;
            }
        }
        self.raw("<w:r><w:rPr>")?;
        let s = &run.style;
        if s.bold { self.raw("<w:b/><w:bCs/>")?; } if s.italic { self.raw("<w:i/><w:iCs/>")?; }
        if s.mono { self.raw("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:eastAsia=\"宋体\"/><w:sz w:val=\"18\"/><w:shd w:val=\"clear\" w:fill=\"F3F4F6\"/>")?; }
        if s.strike { self.raw("<w:strike/>")?; } if s.underline || hyperlink { self.raw("<w:u w:val=\"single\"/>")?; }
        if s.highlight { self.raw("<w:highlight w:val=\"yellow\"/>")?; }
        if s.shift != 0 { self.raw(if s.shift > 0 { "<w:vertAlign w:val=\"superscript\"/>" } else { "<w:vertAlign w:val=\"subscript\"/>" })?; }
        if let Some([r,g,b]) = s.color { self.raw(&format!("<w:color w:val=\"{r:02X}{g:02X}{b:02X}\"/>"))?; }
        else if hyperlink { self.raw("<w:color w:val=\"2563EB\"/>")?; }
        self.raw("</w:rPr>")?;
        match &run.content {
            Content::Break => self.raw("<w:br/>")?,
            Content::Raster(id) => self.drawing(*id)?,
            Content::Footnote(label) if !self.in_footnote && self.footnotes.contains_key(label) => self.raw(&format!("<w:footnoteReference w:id=\"{}\"/>", self.footnotes[label]))?,
            _ => {
                if matches!(&run.content, Content::Footnote(_)) && !self.in_footnote { self.report.warn("未定义的脚注引用已按原标记保留"); }
                for (index, line) in run.label().split('\n').enumerate() {
                    if index > 0 { self.raw("<w:br/>")?; }
                    for (index, text) in line.trim_end_matches('\r').split('\t').enumerate() {
                        if index > 0 { self.raw("<w:tab/>")?; }
                        self.raw("<w:t xml:space=\"preserve\">")?; self.text(text)?; self.raw("</w:t>")?;
                    }
                }
            },
        }
        self.raw("</w:r>")?; if hyperlink { self.raw("</w:hyperlink>")?; } Ok(())
    }
    fn drawing(&mut self, id: usize) -> Result<(), String> {
        let asset = &self.assets[id];
        let scale = (self.available/asset.width).min(720.0/asset.height).min(1.0);
        let (w,h) = ((asset.width*scale*12700.0).round().max(1.0) as u64, (asset.height*scale*12700.0).round().max(1.0) as u64);
        self.drawing_id += 1; let drawing = self.drawing_id;
        let alt = asset.alt.clone();
        self.raw(&format!("<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{w}\" cy=\"{h}\"/><wp:docPr id=\"{drawing}\" name=\"Image {drawing}\" descr=\""))?;
        self.text(&alt)?;
        self.raw(&format!("\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{drawing}\" name=\"Image {drawing}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"image{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"))
    }
    fn table(&mut self, table: &Table) -> Result<(), String> {
        if table.columns == 0 { return Ok(()); }
        let raw: Vec<f32> = (0..table.columns).map(|i| table.widths.get(i).copied().unwrap_or(0.0).max(48.0)).collect();
        let total: f32 = raw.iter().sum(); let widths: Vec<u32> = raw.iter().map(|w| (w/total*9638.0).round() as u32).collect();
        if widths.iter().any(|w| *w < 400) {
            self.report.warn("过宽表格按单元格顺序展开，完整保留内容");
            for cell in &table.cells {
                self.paragraph(&Paragraph { runs: vec![Run::text(format!("[第 {} 行，第 {} 列]", cell.row+1, cell.column+1))], ..Default::default() })?;
                self.paragraph(&Paragraph { runs: cell.runs.clone(), ..Default::default() })?;
            }
            return Ok(());
        }
        self.raw("<w:tbl><w:tblPr><w:tblW w:w=\"9638\" w:type=\"dxa\"/><w:tblBorders>")?;
        for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] { self.raw(&format!("<w:{edge} w:val=\"single\" w:sz=\"4\" w:color=\"D9DDE3\"/>"))?; }
        self.raw("</w:tblBorders><w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"80\" w:type=\"dxa\"/><w:left w:w=\"80\" w:type=\"dxa\"/><w:bottom w:w=\"80\" w:type=\"dxa\"/><w:right w:w=\"80\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid>")?;
        for width in &widths { self.raw(&format!("<w:gridCol w:w=\"{width}\"/>"))?; } self.raw("</w:tblGrid>")?;
        let mut grid = vec![None; table.rows*table.columns];
        for cell in &table.cells { for row in cell.row..cell.row+cell.rowspan { grid[row*table.columns+cell.column] = Some(cell); } }
        for row in 0..table.rows {
            self.raw("<w:tr><w:trPr>")?;
            if row == 0 && table.cells.iter().filter(|c| c.row == 0).all(|c| c.header && c.rowspan == 1) { self.raw("<w:tblHeader/>")?; }
            self.raw("</w:trPr>")?;
            let mut column = 0;
            while column < table.columns {
                let cell = grid[row*table.columns+column];
                let span = cell.map_or(1, |c| c.colspan);
                let width: u32 = widths[column..column+span].iter().sum();
                self.raw(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>"))?;
                if span > 1 { self.raw(&format!("<w:gridSpan w:val=\"{span}\"/>"))?; }
                if let Some(cell) = cell {
                    if cell.rowspan > 1 { self.raw(if cell.row == row { "<w:vMerge w:val=\"restart\"/>" } else { "<w:vMerge/>" })?; }
                    if cell.header { self.raw("<w:shd w:val=\"clear\" w:fill=\"EDF0F4\"/>")?; }
                }
                self.raw("</w:tcPr>")?;
                if let Some(cell) = cell.filter(|c| c.row == row) {
                    let mut runs = cell.runs.clone(); if cell.header { for run in &mut runs { run.style.bold = true; } }
                    let available = self.available; self.available = (width as f32 / 20.0 - 8.0).max(1.0);
                    self.paragraph(&Paragraph { runs, align: cell.align, ..Default::default() })?;
                    self.available = available;
                } else { self.raw("<w:p/>")?; }
                self.raw("</w:tc>")?; column += span;
            }
            self.raw("</w:tr>")?;
        }
        self.raw("</w:tbl>")
    }
}
fn alignment(a: Alignment) -> &'static str { match a { Alignment::Center => "center", Alignment::Right => "right", _ => "left" } }
fn xml(value: &str) -> String {
    let clean: String = value.chars().filter(|c| matches!(*c, '\t' | '\n' | '\r') || *c >= ' ' && *c != '\u{fffe}' && *c != '\u{ffff}').collect();
    html_escape::encode_double_quoted_attribute(&clean).into_owned()
}
fn styles() -> String {
    let mut xml = format!("{HEADER}<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"宋体\"/><w:color w:val=\"202124\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"en-US\" w:eastAsia=\"zh-CN\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"360\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>");
    for (index, size) in [40,32,28,24,22,22].iter().enumerate() { let level = index+1;
        xml.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"{index}\"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>"));
    } xml.push_str("</w:styles>"); xml
}
