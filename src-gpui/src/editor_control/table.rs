//! Markdown 表格的无损源码操作；转义管道与代码中的管道不会变成列边界。
use super::{
    commands::{EditResult, line_ending, map_selection},
    parse::ParsedWindow,
    protocol::*,
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct TableRow {
    pub source: Range<usize>,
    pub cells: Vec<Range<usize>>,
}
#[derive(Clone, Debug)]
pub struct Table {
    pub source: Range<usize>,
    pub rows: Vec<TableRow>,
    pub separator: TableRow,
}
#[derive(Clone, Copy, Debug)]
pub enum TableIntent {
    InsertRow { after: usize },
    DeleteRow { row: usize },
    InsertColumn { at: usize },
    DeleteColumn { column: usize },
    Align { column: usize, alignment: Alignment },
    Resize { rows: usize, columns: usize },
    Header { enabled: bool },
}
#[derive(Clone, Copy, Debug)]
pub enum Alignment {
    Left,
    Center,
    Right,
    None,
}

impl Table {
    pub fn at(parsed: &ParsedWindow, position: usize) -> Option<Self> {
        let node = parsed.nodes.iter().find(|n| {
            n.kind == "table" && n.source.start <= position && position <= n.source.end
        })?;
        let mut offset = node.source.start;
        let mut rows = Vec::new();
        for line in parsed.source[node.source.clone()].split_inclusive('\n') {
            let raw = line.trim_end_matches(['\r', '\n']);
            let end = offset + raw.len();
            rows.push(TableRow {
                source: offset..offset + line.len(),
                cells: cells(&parsed.source, offset..end),
            });
            offset += line.len();
        }
        if rows.len() < 2 {
            return None;
        }
        let separator = rows.remove(1);
        Some(Self {
            source: node.source.clone(),
            rows,
            separator,
        })
    }
    pub fn cell_at(&self, position: usize) -> Option<(usize, usize)> {
        for (row, line) in self.rows.iter().enumerate() {
            for (column, cell) in line.cells.iter().enumerate() {
                if cell.start <= position && position <= cell.end {
                    return Some((row, column));
                }
            }
        }
        None
    }
    pub fn next_cell(&self, position: usize, backwards: bool) -> Option<Range<usize>> {
        let (row, column) = self.cell_at(position)?;
        if backwards {
            if column > 0 {
                self.rows[row].cells.get(column - 1).cloned()
            } else if row > 0 {
                self.rows[row - 1].cells.last().cloned()
            } else {
                None
            }
        } else {
            self.rows[row]
                .cells
                .get(column + 1)
                .or_else(|| self.rows.get(row + 1).and_then(|row| row.cells.first()))
                .cloned()
        }
    }
    pub fn edit(
        &self,
        parsed: &ParsedWindow,
        selection: DocumentSelection,
        intent: TableIntent,
    ) -> Result<EditResult, ControlError> {
        let source = &*parsed.source;
        let base = parsed.identity.from_byte;
        let count = self.separator.cells.len();
        let edit = |range: Range<usize>, text: String| SourceEdit {
            from_byte: base + range.start as u64,
            to_byte: base + range.end as u64,
            inserted_text: text,
        };
        let mut edits = match intent {
            TableIntent::InsertRow { after } => {
                let row = self.rows.get(after).ok_or(ControlError::InvalidRange)?;
                let at = if after == 0 {
                    self.separator.source.end
                } else {
                    row.source.end
                };
                let eol = line_ending(source, at.min(source.len()));
                let mut text = format!("|{}{}", "  |".repeat(count), eol);
                if at > 0 && !source[..at].ends_with('\n') {
                    text.insert_str(0, eol);
                }
                vec![edit(at..at, text)]
            }
            TableIntent::DeleteRow { row } => {
                if row == 0 {
                    let next = self.rows.get(1).ok_or(ControlError::InvalidRange)?;
                    let text = format!(
                        "{}{}",
                        &source[next.source.clone()],
                        &source[self.separator.source.clone()]
                    );
                    return Ok(EditResult {
                        template_update: None,
                        edits: vec![edit(self.rows[0].source.start..next.source.end, text)],
                        selection: DocumentSelection::caret(
                            base + self.rows[0].source.start as u64 + 1,
                        ),
                    });
                }
                vec![edit(
                    self.rows
                        .get(row)
                        .ok_or(ControlError::InvalidRange)?
                        .source
                        .clone(),
                    String::new(),
                )]
            }
            TableIntent::InsertColumn { at } => {
                if at > count {
                    return Err(ControlError::InvalidRange);
                }
                let mut edits = Vec::new();
                for (row, separator) in self
                    .rows
                    .iter()
                    .map(|r| (r, false))
                    .chain(std::iter::once((&self.separator, true)))
                {
                    if row.cells.len() != count {
                        return Err(ControlError::NeedsContext(
                            "不规则表格需要明确补齐列的编辑意图".into(),
                        ));
                    }
                    let text = if separator { " --- " } else { "  " };
                    let point = if at < count {
                        raw_cell_start(source, row, at)
                    } else {
                        raw_cell_end(source, row, count - 1)
                    };
                    let inserted = if at < count {
                        format!("{text}|")
                    } else {
                        format!("|{text}")
                    };
                    edits.push(edit(point..point, inserted));
                }
                edits
            }
            TableIntent::DeleteColumn { column } => {
                if count <= 1 || column >= count {
                    return Err(ControlError::InvalidRange);
                }
                let mut edits = Vec::new();
                for row in self.rows.iter().chain(std::iter::once(&self.separator)) {
                    if row.cells.len() != count {
                        return Err(ControlError::NeedsContext(
                            "不规则表格无法确定列边界".into(),
                        ));
                    }
                    let start = raw_cell_start(source, row, column);
                    let end = raw_cell_end(source, row, column);
                    let range = if column + 1 < count {
                        start..end + 1
                    } else {
                        start - 1..end
                    };
                    edits.push(edit(range, String::new()));
                }
                edits
            }
            TableIntent::Align { column, alignment } => {
                let range = self
                    .separator
                    .cells
                    .get(column)
                    .ok_or(ControlError::InvalidRange)?
                    .clone();
                vec![edit(
                    range,
                    match alignment {
                        Alignment::Left => ":---",
                        Alignment::Center => ":---:",
                        Alignment::Right => "---:",
                        Alignment::None => "---",
                    }
                    .into(),
                )]
            }
            TableIntent::Resize { rows, columns } => {
                if rows == 0 || columns == 0 {
                    return Err(ControlError::InvalidRange);
                }
                rows.checked_mul(columns)
                    .ok_or(ControlError::InvalidRange)?;
                let mut edits = Vec::new();
                for row in self
                    .rows
                    .iter()
                    .take(rows)
                    .chain(std::iter::once(&self.separator))
                {
                    if row.cells.len() != count {
                        return Err(ControlError::NeedsContext(
                            "不规则表格需要先确定列边界".into(),
                        ));
                    }
                    if columns < count {
                        let from = raw_cell_end(source, row, columns - 1);
                        let to = raw_cell_end(source, row, count - 1);
                        edits.push(edit(from..to, String::new()));
                    } else if columns > count {
                        let at = raw_cell_end(source, row, count - 1);
                        let value = if row.source == self.separator.source {
                            "| --- "
                        } else {
                            "|  "
                        };
                        edits.push(edit(at..at, value.repeat(columns - count)));
                    }
                }
                if rows < self.rows.len() {
                    edits.push(edit(
                        self.rows[rows].source.start..self.source.end,
                        String::new(),
                    ));
                } else if rows > self.rows.len() {
                    let at = self.source.end;
                    let eol = line_ending(source, at);
                    let prefix = if at > 0 && !source[..at].ends_with('\n') {
                        eol
                    } else {
                        ""
                    };
                    let row = format!("|{}{eol}", "  |".repeat(columns));
                    edits.push(edit(
                        at..at,
                        format!("{prefix}{}", row.repeat(rows - self.rows.len())),
                    ));
                }
                edits
            }
            TableIntent::Header { enabled } => {
                if enabled {
                    return Ok(EditResult {
                        template_update: None,
                        edits: Vec::new(),
                        selection,
                    });
                }
                let mut text = String::from("<table data-nomo-table=\"semantic\"><tbody>");
                for row in &self.rows {
                    text.push_str("<tr>");
                    for (column, cell) in row.cells.iter().enumerate() {
                        let separator = &source[self
                            .separator
                            .cells
                            .get(column)
                            .ok_or(ControlError::InvalidRange)?
                            .clone()];
                        let alignment = if separator.starts_with(':') && separator.ends_with(':') {
                            " data-align=\"center\""
                        } else if separator.starts_with(':') {
                            " data-align=\"left\""
                        } else if separator.ends_with(':') {
                            " data-align=\"right\""
                        } else {
                            ""
                        };
                        text.push_str(&format!("<td{alignment}><p>"));
                        text.push_str(&cell_html(parsed, cell.clone())?);
                        text.push_str("</p></td>");
                    }
                    text.push_str("</tr>");
                }
                text.push_str("</tbody></table>");
                let caret = super::html_table::HtmlTable::parse(&text, 0..text.len())
                    .and_then(|table| {
                        table
                            .cells
                            .first()
                            .and_then(|cell| cell.runs.first())
                            .map(|run| run.source.start)
                    })
                    .ok_or(ControlError::InvalidRange)?;
                // GFM 节点范围包含最后一行换行。转换为单行 HTML 后仍需保留它，
                // 否则相邻代码块会被 HTML 块吞入，破坏未修改区域的语义。
                let raw = &source[self.source.clone()];
                text.push_str(&raw[raw.trim_end_matches(['\r', '\n']).len()..]);
                return Ok(EditResult {
                    template_update: None,
                    edits: vec![edit(self.source.clone(), text)],
                    selection: DocumentSelection::caret(
                        base + self.source.start as u64 + caret as u64,
                    ),
                });
            }
        };
        edits.sort_by_key(|edit| edit.from_byte);
        Ok(EditResult {
            template_update: None,
            selection: map_selection(selection, &edits),
            edits,
        })
    }
}
fn cell_html(parsed: &ParsedWindow, cell: Range<usize>) -> Result<String, ControlError> {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let mut html = String::new();
    for (event, range) in &parsed.events {
        if range.start < cell.start || range.end > cell.end {
            continue;
        }
        match event {
            Event::Text(text) => html.push_str(&html_escape::encode_text(text)),
            Event::Code(text) => {
                html.push_str(&format!("<code>{}</code>", html_escape::encode_text(text)))
            }
            Event::InlineHtml(text) => html.push_str(text),
            Event::SoftBreak => html.push(' '),
            Event::HardBreak => html.push_str("<br>"),
            Event::Start(tag) => match tag {
                Tag::Strong => html.push_str("<strong>"),
                Tag::Emphasis => html.push_str("<em>"),
                Tag::Strikethrough => html.push_str("<s>"),
                Tag::Superscript => html.push_str("<sup>"),
                Tag::Subscript => html.push_str("<sub>"),
                Tag::Link {
                    dest_url, title, ..
                } => html.push_str(&format!(
                    "<a href=\"{}\" title=\"{}\">",
                    html_escape::encode_double_quoted_attribute(dest_url),
                    html_escape::encode_double_quoted_attribute(title)
                )),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => html.push_str("</strong>"),
                TagEnd::Emphasis => html.push_str("</em>"),
                TagEnd::Strikethrough => html.push_str("</s>"),
                TagEnd::Superscript => html.push_str("</sup>"),
                TagEnd::Subscript => html.push_str("</sub>"),
                TagEnd::Link => html.push_str("</a>"),
                _ => {}
            },
            _ => {
                return Err(ControlError::NeedsContext(
                    "此单元格对象尚未提供 HTML 源码转换".into(),
                ));
            }
        }
    }
    Ok(html)
}
fn raw_cell_start(source: &str, row: &TableRow, column: usize) -> usize {
    let cell = &row.cells[column];
    let mut start = cell.start;
    while start > row.source.start && matches!(source.as_bytes()[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    start
}
fn raw_cell_end(source: &str, row: &TableRow, column: usize) -> usize {
    let cell = &row.cells[column];
    let mut end = cell.end;
    while end < row.source.end && matches!(source.as_bytes()[end], b' ' | b'\t') {
        end += 1;
    }
    end
}
fn cells(source: &str, line: Range<usize>) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    let mut separators = Vec::new();
    let mut at = line.start;
    let mut ticks = 0;
    while at < line.end {
        if bytes[at] == b'\\' {
            at = (at + 2).min(line.end);
            continue;
        }
        if bytes[at] == b'`' {
            let count = bytes[at..line.end]
                .iter()
                .take_while(|b| **b == b'`')
                .count();
            if ticks == 0 {
                ticks = count;
            } else if ticks == count {
                ticks = 0;
            }
            at += count;
            continue;
        }
        if bytes[at] == b'|' && ticks == 0 {
            separators.push(at);
        }
        at += 1;
    }
    let mut ranges = Vec::new();
    let mut start = line.start;
    for end in separators.into_iter().chain(std::iter::once(line.end)) {
        let raw = &source[start..end];
        let left = raw.len() - raw.trim_start_matches([' ', '\t']).len();
        let right = raw.trim_end_matches([' ', '\t']).len();
        // 首尾可选管道产生的空格不属于单元格；中间空单元格保留。
        if !(start == line.start && raw.trim().is_empty() && end < line.end)
            && !(end == line.end && raw.trim().is_empty() && start > line.start)
        {
            let from = start + left.min(right);
            ranges.push(from..start + right);
        }
        start = end + 1;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_control::parse::fixture;
    fn run(source: &str, intent: TableIntent) -> String {
        let parsed = fixture(source);
        let table = Table::at(&parsed, 0).unwrap();
        let result = table
            .edit(&parsed, DocumentSelection::caret(100), intent)
            .unwrap();
        let mut output = source.to_owned();
        for edit in result.edits.into_iter().rev() {
            output.replace_range(
                (edit.from_byte - 100) as usize..(edit.to_byte - 100) as usize,
                &edit.inserted_text,
            );
        }
        output
    }
    #[test]
    fn edits_preserve_escaped_cells_and_crlf() {
        let source = "| A | B |\r\n| --- | ---: |\r\n| a\\|b | **中** |\r\n";
        assert_eq!(
            run(
                source,
                TableIntent::Align {
                    column: 0,
                    alignment: Alignment::Center
                }
            ),
            "| A | B |\r\n| :---: | ---: |\r\n| a\\|b | **中** |\r\n"
        );
        let expanded = run(source, TableIntent::InsertColumn { at: 1 });
        assert!(expanded.contains("a\\|b"));
        assert_eq!(
            run(&expanded, TableIntent::DeleteColumn { column: 1 }),
            source
        );
        let appended = run(source, TableIntent::InsertRow { after: 1 });
        assert!(appended.ends_with("|  |  |\r\n"));
        assert_eq!(run(&appended, TableIntent::DeleteRow { row: 2 }), source);
    }
    #[test]
    fn code_and_escaped_pipes_are_single_cells() {
        let source = "| a\\|b | `x|y` | z |";
        let ranges = cells(source, 0..source.len());
        assert_eq!(ranges.len(), 3);
        assert_eq!(&source[ranges[1].clone()], "`x|y`");
    }
    #[test]
    fn resize_and_header_conversion_preserve_cell_text_and_alignment() {
        let source = "| **A** | B |\r\n| --- | ---: |\r\n| a\\|b | 中 |\r\n";
        let resized = run(
            source,
            TableIntent::Resize {
                rows: 3,
                columns: 3,
            },
        );
        assert!(resized.contains("a\\|b"));
        assert!(resized.contains("---:"));
        assert_eq!(
            run(
                &resized,
                TableIntent::Resize {
                    rows: 2,
                    columns: 2
                }
            ),
            source
        );
        let html = run(source, TableIntent::Header { enabled: false });
        let parsed = super::super::parse::fixture(&html);
        assert_eq!(parsed.html_tables.len(), 1);
        assert!(html.contains("<strong>A</strong>"));
        assert!(html.contains("a|b"));
        assert!(parsed.html_tables[0].cells.iter().all(|cell| !cell.header));
        assert_eq!(
            parsed.html_tables[0].cells[1].align.as_deref(),
            Some("right")
        );
        let followed = format!("{source}\r\n```rust\r\nfn main() {{}}\r\n```\r\n");
        let converted = run(&followed, TableIntent::Header { enabled: false });
        assert!(converted.contains("</table>\r\n\r\n```rust"));
        assert_eq!(fixture(&converted).html_tables.len(), 1);
        assert!(
            fixture(&converted)
                .nodes
                .iter()
                .any(|node| node.kind == "code")
        );
    }
}
