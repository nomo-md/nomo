//! 两种表格共用的行内对象边界与图片属性；只访问当前窗口的事件和源码。
use super::{html_table::{self, InlineAtom, InlineRun}, image_properties::ImageProperties, parse::ParsedWindow};
use pulldown_cmark::{Event, Tag, TagEnd};
use std::ops::Range;

pub(crate) fn html_cell<'a>(parsed: &'a ParsedWindow, range: &Range<usize>) -> Option<&'a html_table::Cell> {
    parsed.html_tables.iter().find_map(|table| table.cell_at(range.start)).filter(|cell| range.end <= cell.content.end)
}

pub(crate) fn validate_insertion(parsed: &ParsedWindow, range: &Range<usize>) -> Result<(), super::protocol::ControlError> {
    use super::{commands, protocol::{ControlError, SourceEdit}};
    let intersects = |table: &Range<usize>| if range.is_empty() { table.contains(&range.start) } else { table.start < range.end && range.start < table.end };
    if parsed.html_tables.iter().any(|table| intersects(&table.source)) {
        let cell = html_cell(parsed, range).ok_or_else(|| ControlError::Unsupported("请在单个表格单元格内插入行内对象".into()))?;
        if !commands::html_text_boundary(parsed, cell, range.start) || !commands::html_text_boundary(parsed, cell, range.end) {
            return Err(ControlError::Unsupported("行内对象不能截断 HTML 标记或实体".into()));
        }
        let (text, _) = commands::html_replacement(parsed, range.clone(), "")?;
        commands::validate_html_edits(parsed, &[SourceEdit { from_byte: parsed.identity.from_byte + range.start as u64, to_byte: parsed.identity.from_byte + range.end as u64, inserted_text: text }])?;
    } else if parsed.nodes.iter().any(|node| node.kind == "table" && intersects(&node.source)) && !markdown_cell(parsed, range) {
        return Err(ControlError::Unsupported("请在单个表格单元格内插入行内对象".into()));
    }
    Ok(())
}

pub(crate) fn markup(parsed: &ParsedWindow, range: &Range<usize>, text: &str) -> Result<String, super::protocol::ControlError> {
    if markdown_cell(parsed, range) { return Ok(escape_html_in_markdown(text)); }
    if html_cell(parsed, range).is_some() {
        let (suffix, _) = super::commands::html_replacement(parsed, range.clone(), "")?;
        return Ok(format!("{text}{suffix}"));
    }
    Ok(text.into())
}

pub(crate) fn markdown_cell(parsed: &ParsedWindow, range: &Range<usize>) -> bool {
    parsed.nodes.iter().any(|node| node.kind == "table-cell" && node.source.start <= range.start && range.end <= node.source.end)
}

/// 语义格内输入允许 Markdown 标记，但管道不能意外成为新的列边界。
/// 计入插入点之前的反斜杠，避免重复转义已有的管道。
pub(crate) fn markdown_input(source: &str, start: usize, text: &str) -> String {
    let mut slashes = source[..start].bytes().rev().take_while(|byte| *byte == b'\\').count();
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '|' && slashes % 2 == 0 { escaped.push('\\'); }
        escaped.push(ch);
        slashes = if ch == '\\' { slashes + 1 } else { 0 };
    }
    escaped
}

/// 退格／删除使用完整对象和换行范围；源码模式不走此路径。
pub(crate) fn atomic_range_at(parsed: &ParsedWindow, at: usize) -> Option<Range<usize>> {
    if let Some(run) = atom_at(parsed, at) { return Some(run.source); }
    for (index, (event, range)) in parsed.events.iter().enumerate() {
        if range.start > at { continue; }
        match event {
            Event::HardBreak if range.contains(&at) => return Some(range.clone()),
            Event::InlineHtml(_) if range.contains(&at)
                && super::nomo_syntax::html_tag(&parsed.source, range.start)
                    .is_some_and(|tag| tag.name == "br" && !tag.closing && tag.end == range.end) =>
            {
                return Some(range.clone());
            }
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let (_, source) = image(&parsed.source, &mut parsed.events[index + 1..].iter().cloned(), dest_url, title, range.clone());
                if source.contains(&at) { return Some(source); }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                if let Some(run) = html_table::atom_at(&parsed.source, range.start)
                    .filter(|run| matches!(run.atom, Some(InlineAtom::Image { .. })) && run.source.contains(&at))
                { return Some(run.source); }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn after_atom<'a>(source: &str, mut event: Event<'a>, mut range: Range<usize>, skip: &Option<Range<usize>>) -> Option<(Event<'a>, Range<usize>)> {
    // 整格只有一个对象时，解析器可能复用其范围；不能连同行列结束事件一起跳过。
    if matches!(&event, Event::Start(Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell)
        | Event::End(TagEnd::Table | TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell))
    { return Some((event, range)); }
    if let Some(skip) = skip {
        if skip.start <= range.start && range.end <= skip.end { return None; }
        if skip.start <= range.start && range.start < skip.end && skip.end < range.end {
            if let Event::Text(text) = &event {
                if let Some(rest) = text.strip_prefix(source.get(range.start..skip.end)?) {
                    event = Event::Text(rest.to_owned().into());
                    range.start = skip.end;
                }
            }
        }
    }
    Some((event, range))
}

pub(crate) fn image<'a>(source: &str, events: &mut impl Iterator<Item = (Event<'a>, Range<usize>)>,
    destination: &str, title: &str, mut range: Range<usize>) -> (ImageProperties, Range<usize>)
{
    let mut properties = ImageProperties { src: destination.into(), alt: String::new(), title: title.into(), width: String::new(), align: String::new() };
    let mut depth = 1usize;
    for (event, _) in events.by_ref() {
        match event {
            Event::Text(text) | Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => properties.alt.push_str(&text),
            Event::SoftBreak | Event::HardBreak => properties.alt.push(' '),
            Event::Start(Tag::Image { .. }) => depth += 1,
            Event::End(TagEnd::Image) => { depth -= 1; if depth == 0 { break; } }
            _ => {}
        }
    }
    if let Some(attributes) = source.get(range.end..).and_then(|tail| tail.strip_prefix('{')).and_then(|tail| tail.split_once('}').map(|(attributes, _)| attributes)) {
        let mut width = String::new();
        let mut align = String::new();
        let valid = attributes.split_whitespace().all(|attribute| {
            match attribute.split_once('=') {
                Some(("width", value)) if value.trim_end_matches("px").parse::<f32>().is_ok_and(|number| number.is_finite() && number > 0.0) => { width = value.into(); true }
                Some(("align", value)) if matches!(value, "left" | "center" | "right") => { align = value.into(); true }
                _ => false,
            }
        });
        if valid && !attributes.is_empty() {
            properties.width = width;
            properties.align = align;
            range.end += attributes.len() + 2;
        }
    }
    (properties, range)
}

pub(crate) fn atom_at(parsed: &ParsedWindow, at: usize) -> Option<InlineRun> {
    if let Some(run) = parsed.html_tables.iter().flat_map(|table| &table.cells).flat_map(|cell| &cell.runs)
        .find(|run| run.source.contains(&at) && run.atom.is_some())
    { return Some(run.clone()); }
    let cell = parsed.nodes.iter().find(|node| node.kind == "table-cell" && node.source.contains(&at))?;
    for (index, (event, range)) in parsed.events.iter().enumerate() {
        if range.start < cell.source.start || range.end > cell.source.end || range.start > at { continue; }
        let atom = match event {
            Event::InlineMath(tex) | Event::DisplayMath(tex) if range.contains(&at) => InlineAtom::Math(tex.to_string()),
            Event::FootnoteReference(id) if range.contains(&at) => InlineAtom::Footnote(id.to_string()),
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let (image, source) = image(&parsed.source, &mut parsed.events[index + 1..].iter().cloned(), dest_url, title, range.clone());
                if !source.contains(&at) { continue; }
                let atom = InlineAtom::Image { src: image.src, alt: image.alt, width: super::image_properties::ImageWidth::parse(&image.width) };
                return Some(InlineRun { text: atom.display_text(), atom: Some(atom), source, formats: Vec::new(), link: None });
            }
            Event::InlineHtml(_) => {
                if let Some(run) = html_table::atom_at(&parsed.source, range.start)
                    .filter(|run| run.source.contains(&at) && run.source.end <= cell.source.end)
                { return Some(run); }
                continue;
            }
            _ => continue,
        };
        return Some(InlineRun { text: atom.display_text(), atom: Some(atom), source: range.clone(), formats: Vec::new(), link: None });
    }
    None
}

pub(crate) fn image_properties(parsed: &ParsedWindow, range: &Range<usize>) -> Option<ImageProperties> {
    if !markdown_cell(parsed, range) { return None; }
    let (index, (_, source)) = parsed.events.iter().enumerate().find(|(_, (event, source))| source.start == range.start && matches!(event, Event::Start(Tag::Image { .. })))?;
    let Event::Start(Tag::Image { dest_url, title, .. }) = &parsed.events[index].0 else { return None; };
    let (image, source) = image(&parsed.source, &mut parsed.events[index + 1..].iter().cloned(), dest_url, title, source.clone());
    (source == *range).then_some(image)
}

pub(crate) fn escape_html_in_markdown(text: &str) -> String {
    text.replace('|', "&#124;").replace('\r', "&#13;").replace('\n', "&#10;")
}
