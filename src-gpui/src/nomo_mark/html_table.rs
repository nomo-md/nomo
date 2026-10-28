//! 原版 data-nomo-table 表格的源码模型。保留合并关系和属性，未知 HTML 回到源码显示。
use super::{
    commands::EditResult,
    nomo_syntax::html_tag,
    parse::{InlineFormat, InlineSpan},
    protocol::*,
};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug)]
pub struct InlineRun {
    pub atom: Option<InlineAtom>,
    pub text: String,
    pub source: Range<usize>,
    pub formats: Vec<InlineFormat>,
    pub link: Option<String>,
}
#[derive(Clone, Debug)]
pub enum InlineAtom {
    Image {
        src: String,
        alt: String,
        width: Option<super::image_properties::ImageWidth>,
    },
    Math(String),
    Comment(String),
    Footnote(String),
}
impl InlineAtom {
    pub(crate) fn display_text(&self) -> String {
        match self {
            Self::Image { alt, .. } => format!("[图片：{alt}]"),
            Self::Math(tex) => tex.clone(),
            Self::Comment(_) => "注释".into(),
            Self::Footnote(id) => format!("[{id}]"),
        }
    }
    pub fn estimated_bytes(&self) -> usize {
        match self {
            Self::Image { src, alt, .. } => src.capacity() + alt.capacity(),
            Self::Math(s) | Self::Comment(s) | Self::Footnote(s) => s.capacity(),
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Image { .. } => None,
            Self::Math(s) | Self::Comment(s) | Self::Footnote(s) => Some(s),
        }
    }
    pub fn with_text(&self, text: &str) -> Option<String> {
        let attr = html_escape::encode_double_quoted_attribute(text);
        let body = html_escape::encode_text(text);
        match self {
            Self::Image { .. } => None,
            Self::Math(_) => Some(format!(
                "<span class=\"math-inline\" data-tex=\"{attr}\">${body}$</span>"
            )),
            Self::Comment(_) => Some(format!(
                "<span class=\"comment-inline\" data-comment=\"{attr}\"></span>"
            )),
            Self::Footnote(_) => Some(format!(
                "<sup class=\"footnote-ref\" data-footnote-id=\"{attr}\">{body}</sup>"
            )),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Cell {
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub row: usize,
    pub column: usize,
    pub rowspan: usize,
    pub colspan: usize,
    pub header: bool,
    pub widths: Vec<u32>,
    pub align: Option<String>,
    pub runs: Vec<InlineRun>,
    pub spans: Vec<InlineSpan>,
}
#[derive(Clone, Debug)]
pub struct HtmlTable {
    pub source: Range<usize>,
    pub rows: usize,
    pub columns: usize,
    pub cells: Vec<Cell>,
}
struct Element {
    name: String,
    attrs: BTreeMap<String, String>,
    source: Range<usize>,
    content: Range<usize>,
    children: Vec<Element>,
}
fn element(source: &str, at: &mut usize, depth: usize) -> Option<Element> {
    if depth > 128 {
        return None;
    }
    let start = *at;
    if source.get(start..)?.starts_with("<!--") {
        let end = start + 4 + source[start + 4..].find("-->")?;
        *at = end + 3;
        return Some(Element { name: "#comment".into(), attrs: BTreeMap::new(), source: start..*at, content: start + 4..end, children: Vec::new() });
    }
    let opening = html_tag(source, *at)?;
    if opening.closing {
        return None;
    }
    *at = opening.end;
    let mut children = Vec::new();
    if opening.name == "br" || opening.name == "img" {
        return Some(Element {
            name: opening.name,
            attrs: opening.attrs,
            source: start..*at,
            content: *at..*at,
            children,
        });
    }
    if opening.self_closing {
        return None;
    }
    while *at < source.len() {
        if source.as_bytes()[*at] != b'<' {
            let end = *at + source[*at..].find('<')?;
            children.push(Element {
                name: String::new(),
                attrs: BTreeMap::new(),
                source: *at..end,
                content: *at..end,
                children: Vec::new(),
            });
            *at = end;
            continue;
        }
        let tag = html_tag(source, *at)?;
        if tag.closing {
            if tag.name != opening.name {
                return None;
            }
            let end = *at;
            *at = tag.end;
            return Some(Element {
                name: opening.name,
                attrs: opening.attrs,
                source: start..*at,
                content: opening.end..end,
                children,
            });
        }
        children.push(element(source, at, depth + 1)?);
    }
    None
}
fn only_attrs(element: &Element, names: &[&str]) -> bool {
    element
        .attrs
        .keys()
        .all(|key| names.contains(&key.as_str()))
}
fn elements<'a>(element: &'a Element, source: &str) -> Option<Vec<&'a Element>> {
    if element
        .children
        .iter()
        .any(|child| child.name.is_empty() && !source[child.source.clone()].trim().is_empty())
    {
        return None;
    }
    Some(
        element
            .children
            .iter()
            .filter(|child| !child.name.is_empty())
            .collect(),
    )
}
fn format(name: &str) -> Option<InlineFormat> {
    Some(match name {
        "strong" | "b" => InlineFormat::Strong,
        "em" | "i" => InlineFormat::Emphasis,
        "code" => InlineFormat::Code,
        "s" | "del" | "strike" => InlineFormat::Strike,
        "u" => InlineFormat::Underline,
        "mark" => InlineFormat::Highlight,
        "sup" => InlineFormat::Superscript,
        "sub" => InlineFormat::Subscript,
        _ => return None,
    })
}
fn text_runs(
    source: &str,
    range: Range<usize>,
    formats: &[InlineFormat],
    link: Option<&str>,
    runs: &mut Vec<InlineRun>,
) {
    let mut at = range.start;
    while at < range.end {
        if source.as_bytes()[at] == b'&' {
            if let Some(end) = source[at..range.end].find(';').map(|end| at + end + 1) {
                let decoded = html_escape::decode_html_entities(&source[at..end]);
                if decoded.as_ref() != &source[at..end] {
                    runs.push(InlineRun {
                        atom: None,
                        text: decoded.into_owned(),
                        source: at..end,
                        formats: formats.into(),
                        link: link.map(str::to_owned),
                    });
                    at = end;
                    continue;
                }
            }
        }
        let next = at + source[at..range.end].chars().next().unwrap().len_utf8();
        let end = source[next..range.end]
            .find('&')
            .map_or(range.end, |end| next + end);
        runs.push(InlineRun {
            atom: None,
            text: source[at..end].into(),
            source: at..end,
            formats: formats.into(),
            link: link.map(str::to_owned),
        });
        at = end;
    }
}
fn inline(
    source: &str,
    element: &Element,
    formats: &[InlineFormat],
    link: Option<&str>,
    runs: &mut Vec<InlineRun>,
    spans: &mut Vec<InlineSpan>,
) -> Option<()> {
    if element.name == "#comment" {
        let atom = InlineAtom::Comment(source[element.content.clone()].trim().into());
        runs.push(InlineRun { text: atom.display_text(), atom: Some(atom), source: element.source.clone(), formats: formats.into(), link: link.map(str::to_owned) });
        return Some(());
    }
    if element.name.is_empty() {
        text_runs(source, element.source.clone(), formats, link, runs);
        return Some(());
    }
    if element.name == "br"
        && only_attrs(element, &["data-nomo-break"])
        && element
            .attrs
            .get("data-nomo-break")
            .is_none_or(|s| matches!(s.as_str(), "soft" | "hard" | ""))
    {
        runs.push(InlineRun {
            atom: None,
            text: "\n".into(),
            source: element.source.clone(),
            formats: formats.into(),
            link: link.map(str::to_owned),
        });
        return Some(());
    }
    let attr = |name: &str| {
        element
            .attrs
            .get(name)
            .map(|value| html_escape::decode_html_entities(value).into_owned())
    };
    let atom = if element.name == "img"
        && only_attrs(element, &["src", "alt", "title", "width", "align"])
    {
        let src = attr("src")?;
        let lower = src.to_ascii_lowercase();
        if src.chars().any(char::is_control)
            || lower.starts_with("javascript:")
            || lower.starts_with("vbscript:")
            || element
                .attrs
                .get("align")
                .is_some_and(|a| !matches!(a.as_str(), "left" | "center" | "right" | ""))
        {
            return None;
        }
        Some(InlineAtom::Image {
            src,
            alt: attr("alt").unwrap_or_default(),
            width: attr("width")
                .and_then(|s| super::image_properties::ImageWidth::parse(&s)),
        })
    } else if element.name == "span"
        && attr("class").as_deref() == Some("math-inline")
        && only_attrs(element, &["class", "data-tex"])
    {
        Some(InlineAtom::Math(attr("data-tex")?))
    } else if element.name == "span"
        && attr("class").as_deref() == Some("comment-inline")
        && only_attrs(element, &["class", "data-comment"])
    {
        Some(InlineAtom::Comment(attr("data-comment")?))
    } else if element.name == "sup"
        && attr("class").as_deref() == Some("footnote-ref")
        && only_attrs(element, &["class", "data-footnote-id"])
    {
        Some(InlineAtom::Footnote(attr("data-footnote-id")?))
    } else {
        None
    };
    if let Some(atom) = atom {
        if element.children.iter().any(|child| !child.name.is_empty()) {
            return None;
        }
        let text = atom.display_text();
        runs.push(InlineRun {
            atom: Some(atom),
            text,
            source: element.source.clone(),
            formats: formats.into(),
            link: link.map(str::to_string),
        });
        return Some(());
    }
    let mut formats = formats.to_vec();
    let mut link = link.map(str::to_owned);
    let mark = if element.name == "a" {
        if !only_attrs(element, &["href", "title", "id", "name"]) {
            return None;
        }
        link = attr("href");
        InlineFormat::Link
    } else {
        if !element.attrs.is_empty() {
            return None;
        }
        format(&element.name)?
    };
    formats.push(mark);
    spans.push(InlineSpan {
        format: mark,
        open: element.source.start..element.content.start,
        content: element.content.clone(),
        close: element.content.end..element.source.end,
    });
    for child in &element.children {
        inline(source, child, &formats, link.as_deref(), runs, spans)?;
    }
    Some(())
}

/// Markdown 表格内的 HTML 行内对象与 HTML 表格共用同一套属性校验。
pub(crate) fn atom_at(source: &str, start: usize) -> Option<InlineRun> {
    if source.get(start..)?.starts_with("<!--") {
        let end = start + 4 + source[start + 4..].find("-->")? + 3;
        let atom = InlineAtom::Comment(source[start + 4..end - 3].trim().into());
        return Some(InlineRun { text: atom.display_text(), atom: Some(atom), source: start..end, formats: Vec::new(), link: None });
    }
    let tag = html_tag(source, start)?;
    if tag.closing || !match tag.name.as_str() {
        "img" => true,
        "span" => tag.attrs.get("class").is_some_and(|class| matches!(class.as_str(), "math-inline" | "comment-inline")),
        "sup" => tag.attrs.get("class").is_some_and(|class| class == "footnote-ref"),
        _ => false,
    } { return None; }
    let mut end = start;
    let element = element(source, &mut end, 0)?;
    let mut runs = Vec::new();
    inline(source, &element, &[], None, &mut runs, &mut Vec::new())?;
    if runs.len() != 1 || runs[0].atom.is_none() || runs[0].source != element.source { return None; }
    runs.pop()
}

impl HtmlTable {
    pub fn parse(source: &str, range: Range<usize>) -> Option<Self> {
        let text = source.get(range.clone())?;
        let start = range.start + text.len() - text.trim_start().len();
        let mut at = start;
        let table = element(source, &mut at, 0)?;
        if !source.get(at..range.end)?.trim().is_empty()
            || table.name != "table"
            || table.attrs.get("data-nomo-table").map(String::as_str) != Some("semantic")
            || !only_attrs(&table, &["data-nomo-table"])
        {
            return None;
        }
        let bodies = elements(&table, source)?;
        if bodies.len() != 1 || bodies[0].name != "tbody" || !bodies[0].attrs.is_empty() {
            return None;
        }
        let rows = elements(bodies[0], source)?;
        if rows.is_empty() {
            return None;
        }
        let mut cells: Vec<Cell> = Vec::new();
        let mut columns = 0;
        let mut active: Vec<(usize, usize, usize)> = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            if row.name != "tr" || !row.attrs.is_empty() {
                return None;
            }
            active.retain(|(_, _, end_row)| *end_row > row_index);
            let mut column = 0;
            for cell in elements(row, source)? {
                while let Some((_, end, _)) = active
                    .iter()
                    .find(|(start, end, _)| *start <= column && column < *end)
                {
                    column = *end;
                }
                if !matches!(cell.name.as_str(), "td" | "th")
                    || !only_attrs(cell, &["rowspan", "colspan", "data-colwidth", "data-align"])
                {
                    return None;
                }
                let number = |name: &str| {
                    cell.attrs
                        .get(name)
                        .map_or(Some(1), |value| value.parse::<usize>().ok())
                        .filter(|value| *value > 0)
                };
                let rowspan = number("rowspan")?;
                let colspan = number("colspan")?;
                if row_index.checked_add(rowspan)? > rows.len()
                    || column.checked_add(colspan)? > source.len().max(1)
                {
                    return None;
                }
                if active
                    .iter()
                    .any(|(start, end, _)| column < *end && *start < column + colspan)
                {
                    return None;
                }
                let widths = cell
                    .attrs
                    .get("data-colwidth")
                    .map(|value| {
                        value
                            .split(',')
                            .map(str::parse::<u32>)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()
                    .ok()?
                    .unwrap_or_default();
                if !widths.is_empty() && widths.len() != colspan {
                    return None;
                }
                let align = cell.attrs.get("data-align").cloned();
                if align
                    .as_deref()
                    .is_some_and(|align| !matches!(align, "left" | "center" | "right"))
                {
                    return None;
                }
                let paragraphs = elements(cell, source)?;
                if paragraphs.is_empty() {
                    return None;
                }
                let mut runs = Vec::new();
                let mut spans = Vec::new();
                for (index, paragraph) in paragraphs.into_iter().enumerate() {
                    if paragraph.name != "p" || !paragraph.attrs.is_empty() {
                        return None;
                    }
                    if index > 0 {
                        runs.push(InlineRun {
                            atom: None,
                            text: "\n".into(),
                            source: paragraph.source.start..paragraph.content.start,
                            formats: Vec::new(),
                            link: None,
                        });
                    }
                    if paragraph.children.is_empty() {
                        runs.push(InlineRun {
                            atom: None,
                            text: String::new(),
                            source: paragraph.content.clone(),
                            formats: Vec::new(),
                            link: None,
                        });
                    }
                    for child in &paragraph.children {
                        inline(source, child, &[], None, &mut runs, &mut spans)?;
                    }
                }
                cells.push(Cell {
                    source: cell.source.clone(),
                    content: cell.content.clone(),
                    row: row_index,
                    column,
                    rowspan,
                    colspan,
                    header: cell.name == "th",
                    widths,
                    align,
                    runs,
                    spans,
                });
                if rowspan > 1 {
                    active.push((column, column + colspan, row_index + rowspan));
                }
                column += colspan;
                columns = columns.max(column);
            }
        }
        // 每行用区间校验完整网格，不为跨格分配 rows × columns 的第二份矩阵。
        let mut active = Vec::new();
        let mut next = 0;
        for row in 0..rows.len() {
            active.retain(|index: &usize| row < cells[*index].row + cells[*index].rowspan);
            while next < cells.len() && cells[next].row == row {
                active.push(next);
                next += 1;
            }
            let mut intervals: Vec<_> = active
                .iter()
                .map(|index| cells[*index].column..cells[*index].column + cells[*index].colspan)
                .collect();
            intervals.sort_by_key(|range| range.start);
            let mut end = 0;
            for range in intervals {
                if range.start != end {
                    return None;
                }
                end = range.end;
            }
            if end != columns {
                return None;
            }
        }
        Some(Self {
            source: table.source.clone(),
            rows: rows.len(),
            columns,
            cells,
        })
    }
    pub fn cell_at(&self, at: usize) -> Option<&Cell> {
        self.cells
            .iter()
            .find(|cell| cell.content.start <= at && at <= cell.content.end)
    }
    pub fn next_cell(&self, at: usize, backwards: bool) -> Option<Range<usize>> {
        let index = self
            .cells
            .iter()
            .position(|cell| cell.content.start <= at && at <= cell.content.end)?;
        let next = if backwards {
            index.checked_sub(1)?
        } else {
            index + 1
        };
        self.cells
            .get(next)?
            .runs
            .first()
            .map(|run| run.source.clone())
    }
    pub fn edit(
        &self,
        source: &str,
        base: u64,
        intent: HtmlTableIntent,
    ) -> Result<EditResult, ControlError> {
        let mut rows = self.rows;
        let mut columns = self.columns;
        let mut cells: Vec<_> = self
            .cells
            .iter()
            .map(|cell| (cell.clone(), source[cell.content.clone()].to_string()))
            .collect();
        let empty = |row, column, header| {
            (
                Cell {
                    source: 0..0,
                    content: 0..0,
                    row,
                    column,
                    rowspan: 1,
                    colspan: 1,
                    header,
                    widths: Vec::new(),
                    align: None,
                    runs: Vec::new(),
                    spans: Vec::new(),
                },
                "<p></p>".into(),
            )
        };
        match intent {
            HtmlTableIntent::InsertRow { at } => {
                if at > rows {
                    return Err(ControlError::InvalidRange);
                }
                for (cell, _) in &mut cells {
                    if cell.row < at && at < cell.row + cell.rowspan {
                        cell.rowspan += 1;
                    } else if cell.row >= at {
                        cell.row += 1;
                    }
                }
                for column in 0..columns {
                    if !cells.iter().any(|(cell, _)| {
                        cell.row <= at
                            && at < cell.row + cell.rowspan
                            && cell.column <= column
                            && column < cell.column + cell.colspan
                    }) {
                        cells.push(empty(at, column, false));
                    }
                }
                rows += 1;
            }
            HtmlTableIntent::DeleteRow { row } => {
                if row >= rows || rows == 1 {
                    return Err(ControlError::InvalidRange);
                }
                cells.retain(|(cell, _)| cell.row != row || cell.rowspan > 1);
                for (cell, _) in &mut cells {
                    if cell.row <= row && row < cell.row + cell.rowspan {
                        cell.rowspan -= 1;
                    }
                    if cell.row > row {
                        cell.row -= 1;
                    }
                }
                rows -= 1;
            }
            HtmlTableIntent::InsertColumn { at } => {
                if at > columns {
                    return Err(ControlError::InvalidRange);
                }
                for (cell, _) in &mut cells {
                    if cell.column < at && at < cell.column + cell.colspan {
                        let local = at - cell.column;
                        cell.colspan += 1;
                        if !cell.widths.is_empty() {
                            cell.widths.insert(local, 0);
                        }
                    } else if cell.column >= at {
                        cell.column += 1;
                    }
                }
                for row in 0..rows {
                    if !cells.iter().any(|(cell, _)| {
                        cell.row <= row
                            && row < cell.row + cell.rowspan
                            && cell.column <= at
                            && at < cell.column + cell.colspan
                    }) {
                        cells.push(empty(
                            row,
                            at,
                            row == 0 && self.cells.iter().any(|cell| cell.row == 0 && cell.header),
                        ));
                    }
                }
                columns += 1;
            }
            HtmlTableIntent::DeleteColumn { column } => {
                if column >= columns || columns == 1 {
                    return Err(ControlError::InvalidRange);
                }
                cells.retain(|(cell, _)| !(cell.column == column && cell.colspan == 1));
                for (cell, _) in &mut cells {
                    if cell.column <= column && column < cell.column + cell.colspan {
                        cell.colspan -= 1;
                        if !cell.widths.is_empty() {
                            cell.widths.remove(column - cell.column);
                        }
                    }
                    if cell.column > column {
                        cell.column -= 1;
                    }
                }
                columns -= 1;
            }
            HtmlTableIntent::Resize {
                rows: new_rows,
                columns: new_columns,
            } => {
                if new_rows == 0 || new_columns == 0 {
                    return Err(ControlError::InvalidRange);
                }
                new_rows
                    .checked_mul(new_columns)
                    .ok_or(ControlError::InvalidRange)?;
                cells.retain(|(cell, _)| cell.row < new_rows && cell.column < new_columns);
                for (cell, _) in &mut cells {
                    cell.rowspan = cell.rowspan.min(new_rows - cell.row);
                    cell.colspan = cell.colspan.min(new_columns - cell.column);
                    cell.widths.truncate(cell.colspan);
                }
                for row in 0..new_rows {
                    for column in if row < rows {
                        columns.min(new_columns)..new_columns
                    } else {
                        0..new_columns
                    } {
                        cells.push(empty(
                            row,
                            column,
                            row == 0 && self.cells.iter().any(|cell| cell.row == 0 && cell.header),
                        ));
                    }
                }
                rows = new_rows;
                columns = new_columns;
            }
            HtmlTableIntent::Header { enabled } => {
                for (cell, _) in &mut cells {
                    if cell.row == 0 {
                        cell.header = enabled;
                    }
                }
            }
            HtmlTableIntent::Align { column, alignment } => {
                if column >= columns {
                    return Err(ControlError::InvalidRange);
                }
                for (cell, _) in &mut cells {
                    if cell.column <= column && column < cell.column + cell.colspan {
                        cell.align = match alignment {
                            super::table::Alignment::Left => Some("left"),
                            super::table::Alignment::Center => Some("center"),
                            super::table::Alignment::Right => Some("right"),
                            super::table::Alignment::None => None,
                        }
                        .map(str::to_owned);
                    }
                }
            }
        }
        cells.sort_by_key(|(cell, _)| (cell.row, cell.column));
        let mut output = String::from("<table data-nomo-table=\"semantic\"><tbody>");
        let mut index = 0;
        for row in 0..rows {
            output.push_str("<tr>");
            while let Some((cell, content)) = cells.get(index).filter(|(cell, _)| cell.row == row) {
                let tag = if cell.header { "th" } else { "td" };
                output.push('<');
                output.push_str(tag);
                if cell.rowspan > 1 {
                    output.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
                }
                if cell.colspan > 1 {
                    output.push_str(&format!(" colspan=\"{}\"", cell.colspan));
                }
                if !cell.widths.is_empty() {
                    output.push_str(&format!(
                        " data-colwidth=\"{}\"",
                        cell.widths
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                if let Some(align) = &cell.align {
                    output.push_str(&format!(" data-align=\"{align}\""));
                }
                output.push('>');
                output.push_str(content);
                output.push_str(&format!("</{tag}>"));
                index += 1;
            }
            output.push_str("</tr>");
        }
        output.push_str("</tbody></table>");
        let _ = columns;
        let validated = Self::parse(&output, 0..output.len())
            .ok_or_else(|| ControlError::HostFailure("表格操作产生无效网格".into()))?;
        let caret = base
            + self.source.start as u64
            + validated
                .cells
                .first()
                .and_then(|cell| cell.runs.first())
                .map_or(0, |run| run.source.start) as u64;
        Ok(EditResult {
            template_update: None,
            edits: vec![SourceEdit {
                from_byte: base + self.source.start as u64,
                to_byte: base + self.source.end as u64,
                inserted_text: output,
            }],
            selection: DocumentSelection::caret(caret),
        })
    }
}
#[derive(Clone, Copy)]
pub enum HtmlTableIntent {
    InsertRow {
        at: usize,
    },
    DeleteRow {
        row: usize,
    },
    InsertColumn {
        at: usize,
    },
    DeleteColumn {
        column: usize,
    },
    Resize {
        rows: usize,
        columns: usize,
    },
    Header {
        enabled: bool,
    },
    Align {
        column: usize,
        alignment: super::table::Alignment,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = "<table data-nomo-table=\"semantic\"><tbody><tr><th rowspan=\"2\" data-colwidth=\"80\"><p>中&amp;😀</p></th><th colspan=\"2\"><p><strong>BC</strong></p></th></tr><tr><td><p>b</p></td><td><p>c</p></td></tr></tbody></table>";
    #[test]
    fn original_html_contract_preserves_spans_entities_and_attributes() {
        let table = HtmlTable::parse(SAMPLE, 0..SAMPLE.len()).unwrap();
        assert_eq!((table.rows, table.columns, table.cells.len()), (2, 3, 4));
        let cell = &table.cells[0];
        assert_eq!(cell.rowspan, 2);
        assert_eq!(cell.widths, vec![80]);
        assert_eq!(
            cell.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "中&😀"
        );
        assert_eq!(&SAMPLE[cell.runs[1].source.clone()], "&amp;");
        assert!(
            HtmlTable::parse(
                &SAMPLE.replace("data-colwidth=\"80\"", "style=\"color:red\""),
                0..SAMPLE.len()
            )
            .is_none()
        );
    }
    #[test]
    fn structural_edits_keep_merged_content_and_valid_grid() {
        let table = HtmlTable::parse(SAMPLE, 0..SAMPLE.len()).unwrap();
        for (intent, expected) in [
            (HtmlTableIntent::InsertRow { at: 1 }, (3, 3)),
            (HtmlTableIntent::DeleteRow { row: 0 }, (1, 3)),
            (HtmlTableIntent::InsertColumn { at: 2 }, (2, 4)),
            (HtmlTableIntent::DeleteColumn { column: 1 }, (2, 2)),
            (
                HtmlTableIntent::Resize {
                    rows: 4,
                    columns: 5,
                },
                (4, 5),
            ),
        ] {
            let result = table.edit(SAMPLE, 100, intent).unwrap();
            let changed = &result.edits[0].inserted_text;
            let parsed = HtmlTable::parse(changed, 0..changed.len()).unwrap();
            assert_eq!((parsed.rows, parsed.columns), expected);
            assert!(changed.contains("中&amp;😀"));
        }
    }
    #[test]
    fn resizing_headerless_tables_does_not_create_header_cells() {
        let text = SAMPLE.replace("<th ", "<td ").replace("</th>", "</td>");
        let table = HtmlTable::parse(&text, 0..text.len()).unwrap();
        let result = table
            .edit(
                &text,
                0,
                HtmlTableIntent::Resize {
                    rows: 3,
                    columns: 20,
                },
            )
            .unwrap();
        let next = &result.edits[0].inserted_text;
        let table = HtmlTable::parse(next, 0..next.len()).unwrap();
        assert!(table.cells.iter().all(|cell| !cell.header));
        assert_eq!((table.rows, table.columns), (3, 20));
    }
}
#[cfg(test)]
mod atom_tests {
    use super::*;
    #[test]
    fn cell_atoms_roundtrip_and_delete_as_whole_source_tokens() {
        let source = "<table data-nomo-table=\"semantic\"><tbody><tr><td><p>a<img src=\"a.png\" alt=\"中&amp;文\" width=\"80\"><br data-nomo-break=\"soft\"><span class=\"math-inline\" data-tex=\"x^2\">$x^2$</span><span class=\"comment-inline\" data-comment=\"注释\"></span><sup class=\"footnote-ref\" data-footnote-id=\"n\">n</sup>b</p></td></tr></tbody></table>";
        let table = HtmlTable::parse(source, 0..source.len()).unwrap();
        let atoms: Vec<_> = table.cells[0]
            .runs
            .iter()
            .filter(|r| r.atom.is_some())
            .collect();
        assert_eq!(atoms.len(), 4);
        let parsed = super::super::parse::fixture(source);
        let image = atoms[0];
        let result = super::super::commands::apply_semantic(
            &parsed,
            DocumentSelection::caret(parsed.identity.from_byte + image.source.end as u64),
            super::super::commands::EditIntent::Backspace,
        )
        .unwrap();
        assert_eq!(result.edits.len(), 1);
        assert_eq!(
            result.edits[0].from_byte,
            parsed.identity.from_byte + image.source.start as u64
        );
        assert_eq!(
            result.edits[0].to_byte,
            parsed.identity.from_byte + image.source.end as u64
        );
        let atom = atoms[1].atom.as_ref().unwrap();
        let replacement = atom.with_text("a < b & c").unwrap();
        let changed = source.replace(&source[atoms[1].source.clone()], &replacement);
        let table = HtmlTable::parse(&changed, 0..changed.len()).unwrap();
        assert!(
            table.cells[0]
                .runs
                .iter()
                .any(|r| matches!(&r.atom, Some(InlineAtom::Math(tex)) if tex == "a < b & c"))
        );
        assert!(
            HtmlTable::parse(
                &source.replace("width=\"80\"", "onclick=\"x()\""),
                0..source.replace("width=\"80\"", "onclick=\"x()\"").len()
            )
            .is_none()
        );
    }
}
