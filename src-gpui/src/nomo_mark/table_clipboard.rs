//! 矩形单元格选区与粘贴；编辑仍合并为宿主的一笔事务。
use super::*;
use crate::nomo_mark::{commands::EditResult, clipboard_content::{self, Payload}, parse::ParsedWindow};

#[derive(Clone)]
pub(super) struct CellSelection {
    pub identity: WindowIdentity,
    pub table: usize,
    pub anchor: (usize, usize),
    pub head: (usize, usize),
}
struct Cell { row: usize, column: usize, rowspan: usize, colspan: usize, content: Range<usize> }
struct Grid { source: Range<usize>, rows: usize, columns: usize, cells: Vec<Cell>, html: bool }
fn grid(parsed: &ParsedWindow, at: usize) -> Option<Grid> {
    if let Some(t) = parsed.html_tables.iter().find(|t| t.source.start <= at && at < t.source.end) {
        return Some(Grid { source: t.source.clone(), rows: t.rows, columns: t.columns, html: true,
            cells: t.cells.iter().map(|c| Cell { row: c.row, column: c.column, rowspan: c.rowspan, colspan: c.colspan, content: c.content.clone() }).collect() });
    }
    let t = crate::nomo_mark::table::Table::at(parsed, at)?;
    let cells = t.rows.iter().enumerate().flat_map(|(row, r)| r.cells.iter().enumerate()
        .map(move |(column, content)| Cell { row, column, rowspan: 1, colspan: 1, content: content.clone() })).collect();
    Some(Grid { source: t.source, rows: t.rows.len(), columns: t.separator.cells.len(), cells, html: false })
}
impl CellSelection {
    fn includes(&self, cell: &Cell) -> bool {
        cell.row <= self.anchor.0.max(self.head.0)
            && cell.row + cell.rowspan > self.anchor.0.min(self.head.0)
            && cell.column <= self.anchor.1.max(self.head.1)
            && cell.column + cell.colspan > self.anchor.1.min(self.head.1)
    }
}
/// 部分覆盖合并格时只拆开相交格，保留区域外内容；整张表仍提交一个局部补丁。
fn paste_spans(parsed: &ParsedWindow, original: &Grid, row: usize, column: usize,
    incoming: &crate::nomo_mark::html_table::HtmlTable, markup: &str) -> Result<EditResult, ControlError> {
    use crate::nomo_mark::html_table::{HtmlTable, HtmlTableIntent};
    let raw = &parsed.source[original.source.clone()];
    let target_markup = if original.html { raw.to_string() } else { clipboard_content::from_html(&clipboard_content::html(raw)) };
    let target = HtmlTable::parse(&target_markup, 0..target_markup.len()).ok_or(ControlError::InvalidRange)?;
    let rows = target.rows.max(row + incoming.rows);
    let columns = target.columns.max(column + incoming.columns);
    if rows > 512 || columns > 128 { return Err(ControlError::Unsupported("粘贴表格超过当前行列预算；原文已保留".into())); }
    let resize = target.edit(&target_markup, 0, HtmlTableIntent::Resize { rows, columns })?;
    let resized = &resize.edits[0].inserted_text;
    let target = HtmlTable::parse(resized, 0..resized.len()).ok_or(ControlError::InvalidRange)?;
    let alignments: Vec<_> = if original.html { Vec::new() } else {
        crate::nomo_mark::table::Table::at(parsed, original.source.start).map_or_else(Vec::new, |table| table.separator.cells.iter().map(|range| {
            let separator = parsed.source[range.clone()].trim();
            match (separator.starts_with(':'), separator.ends_with(':')) {
                (true, true) => Some("center".to_string()), (true, false) => Some("left".to_string()),
                (false, true) => Some("right".to_string()), _ => None,
            }
        }).collect())
    };
    let mut cells = Vec::new();
    let metadata = |cell: &crate::nomo_mark::html_table::Cell| crate::nomo_mark::html_table::Cell {
        source: 0..0, content: cell.content.clone(), row: cell.row, column: cell.column,
        rowspan: cell.rowspan, colspan: cell.colspan, header: cell.header,
        widths: cell.widths.clone(), align: cell.align.clone(), runs: Vec::new(), spans: Vec::new(),
    };
    let budget = crate::nomo_mark::session_window::policy().group_bytes as usize;
    let too_large = || ControlError::NeedsContext("粘贴表格超过局部编辑预算；原文已保留".into());
    for cell in &target.cells {
        let overlaps = cell.row < row + incoming.rows && row < cell.row + cell.rowspan
            && cell.column < column + incoming.columns && column < cell.column + cell.colspan;
        if !overlaps {
            let mut cell = metadata(cell);
            if !original.html { cell.header = cell.row == 0; cell.align = alignments.get(cell.column).cloned().flatten(); }
            let content = resized[cell.content.clone()].to_string();
            cells.push((cell, content));
            continue;
        }
        for r in cell.row..cell.row + cell.rowspan {
            for c in cell.column..cell.column + cell.colspan {
                if (row..row + incoming.rows).contains(&r) && (column..column + incoming.columns).contains(&c) { continue; }
                if cells.len() >= budget / 8 { return Err(too_large()); }
                let mut split = metadata(cell);
                split.row = r; split.column = c; split.rowspan = 1; split.colspan = 1;
                split.widths = cell.widths.get(c - cell.column).copied().into_iter().collect();
                if !original.html { split.header = r == 0; split.align = alignments.get(c).cloned().flatten(); }
                split.runs.clear(); split.spans.clear();
                let content = if r == cell.row && c == cell.column { resized[cell.content.clone()].to_string() } else { "<p></p>".into() };
                cells.push((split, content));
            }
        }
    }
    for cell in &incoming.cells {
        let mut target = metadata(cell);
        target.row += row; target.column += column;
        target.runs.clear(); target.spans.clear();
        cells.push((target, markup[cell.content.clone()].to_string()));
    }
    cells.sort_by_key(|(cell, _)| (cell.row, cell.column));
    let mut output = String::from("<table data-nomo-table=\"semantic\"><tbody>");
    let mut next = cells.iter().peekable();
    for row in 0..rows {
        output.push_str("<tr>");
        while next.peek().is_some_and(|(cell, _)| cell.row == row) {
            let (cell, content) = next.next().unwrap();
            let tag = if cell.header { "th" } else { "td" };
            output.push_str(&format!("<{tag}"));
            if cell.rowspan > 1 { output.push_str(&format!(" rowspan=\"{}\"", cell.rowspan)); }
            if cell.colspan > 1 { output.push_str(&format!(" colspan=\"{}\"", cell.colspan)); }
            if !cell.widths.is_empty() { output.push_str(&format!(" data-colwidth=\"{}\"", cell.widths.iter().map(u32::to_string).collect::<Vec<_>>().join(","))); }
            if let Some(align) = &cell.align { output.push_str(&format!(" data-align=\"{align}\"")); }
            output.push('>'); output.push_str(content); output.push_str(&format!("</{tag}>"));
            if output.len() > budget { return Err(too_large()); }
        }
        output.push_str("</tr>");
    }
    output.push_str("</tbody></table>");
    if raw.ends_with("\r\n") { output.push_str("\r\n"); } else if raw.ends_with('\n') { output.push('\n'); }
    if output.len() > budget { return Err(too_large()); }
    let table = HtmlTable::parse(&output, 0..output.len()).ok_or_else(|| ControlError::HostFailure("粘贴表格产生无效网格，原文已保留".into()))?;
    let caret = table.cells.iter().find(|cell| cell.row == row && cell.column == column)
        .and_then(|cell| cell.runs.first().map(|run| run.source.start).or(Some(cell.content.start + 3)))
        .ok_or(ControlError::InvalidRange)?;
    let base = parsed.identity.from_byte + original.source.start as u64;
    Ok(EditResult { edits: vec![SourceEdit { from_byte: base, to_byte: parsed.identity.from_byte + original.source.end as u64, inserted_text: output }],
        selection: DocumentSelection::caret(base + caret as u64), template_update: None })
}
impl NomoMark {
    pub(super) fn table_cell_at(&self, at: usize) -> Option<(usize, usize, usize)> {
        let t = grid(&self.host.parsed, at)?;
        let cell = t.cells.iter().find(|c| c.content.start <= at && at <= c.content.end)?;
        Some((t.source.start, cell.row, cell.column))
    }
    pub(super) fn selected_cell_ranges(&self) -> Vec<Range<usize>> {
        let Some(selection) = &self.cell_selection else { return Vec::new(); };
        if selection.identity != self.host.parsed.identity { return Vec::new(); }
        grid(&self.host.parsed, selection.table).map_or_else(Vec::new, |t| t.cells.into_iter()
            .filter(|c| selection.includes(c)).map(|c| c.content).collect())
    }
    pub(super) fn selected_cells_edit(&self, intent: &EditIntent) -> Option<Result<EditResult, ControlError>> {
        let ranges = self.selected_cell_ranges();
        if ranges.is_empty() { return None; }
        let base = self.host.parsed.identity.from_byte;
        let mut edits = Vec::new();
        let mut template_update: Option<TemplateUpdate> = None;
        let mut first_caret = base + ranges[0].start as u64;
        for (index, range) in ranges.iter().enumerate() {
            let html_cell = self.host.parsed.html_tables.iter().flat_map(|t| &t.cells).find(|c| c.content == *range);
            if let Some(cell) = html_cell {
                if matches!(intent, EditIntent::Replace(_) | EditIntent::Backspace | EditIntent::Delete) {
                    let text = match intent { EditIntent::Replace(text) if index == 0 => text.as_str(), _ => "" };
                    let text = html_escape::encode_text(text);
                    if index == 0 { first_caret = base + cell.content.start as u64 + 3 + text.len() as u64; }
                    edits.push(SourceEdit { from_byte: base + cell.content.start as u64, to_byte: base + cell.content.end as u64,
                        inserted_text: format!("<p>{text}</p>") });
                    continue;
                }
            }
            let intent = match intent {
                EditIntent::Replace(text) => EditIntent::Replace(if index == 0 { text.clone() } else { String::new() }),
                EditIntent::Backspace | EditIntent::Delete => EditIntent::Replace(String::new()),
                EditIntent::ToggleInline(format) => EditIntent::ToggleInline(*format),
                EditIntent::ClearFormatting => EditIntent::ClearFormatting,
                _ => return Some(Err(ControlError::Unsupported("请先选择单个单元格再执行此操作".into()))),
            };
            let range = html_cell.map_or(range.clone(), |cell| {
                let source = &self.host.parsed.source;
                let start = source[cell.content.clone()].find("<p>").map_or(cell.content.start, |p| cell.content.start + p + 3);
                let end = source[cell.content.clone()].rfind("</p>").map_or(cell.content.end, |p| cell.content.start + p);
                start..end
            });
            match commands::apply_semantic(&self.host.parsed, DocumentSelection {
                anchor_byte: base + range.start as u64, head_byte: base + range.end as u64 }, intent) {
                Ok(result) => {
                    if index == 0 { first_caret = result.selection.head_byte; }
                    if let Some(mut update) = result.template_update {
                        // 每格命令的新增模板已映射自身补丁；再映射前面单元格的位移。
                        crate::markdown_templates::map_templates(&mut update.add, edits.iter().rev()
                            .map(|e: &SourceEdit| (e.from_byte, e.to_byte, e.inserted_text.len() as u64)), true);
                        template_update.get_or_insert_with(TemplateUpdate::default).add.extend(update.add);
                    }
                    edits.extend(result.edits);
                }
                Err(error) => return Some(Err(error)),
            }
        }
        let after = DocumentSelection::caret(first_caret);
        Some(Ok(EditResult { edits, selection: after, template_update }))
    }
    pub(super) fn copy_cells(&self, markdown: bool) -> Option<Result<Payload, ControlError>> {
        let selection = self.cell_selection.as_ref()?;
        if selection.identity != self.host.parsed.identity { return None; }
        let t = grid(&self.host.parsed, selection.table)?;
        let mut text = String::new();
        let mut html = String::from("<table><tbody>");
        for row in selection.anchor.0.min(selection.head.0)..=selection.anchor.0.max(selection.head.0) {
            if row > selection.anchor.0.min(selection.head.0) { text.push('\n'); }
            html.push_str("<tr>");
            if markdown { text.push('|'); }
            for column in selection.anchor.1.min(selection.head.1)..=selection.anchor.1.max(selection.head.1) {
                if column > selection.anchor.1.min(selection.head.1) && !markdown { text.push('\t'); }
                let Some(cell) = t.cells.iter().find(|c| c.row <= row && row < c.row + c.rowspan && c.column <= column && column < c.column + c.colspan) else {
                    if markdown { text.push_str("  |"); } html.push_str("<td></td>"); continue;
                };
                let first_row = cell.row.max(selection.anchor.0.min(selection.head.0));
                let first_column = cell.column.max(selection.anchor.1.min(selection.head.1));
                if row != first_row || column != first_column {
                    if markdown { text.push_str("  |"); }
                    continue;
                }
                let payload = match clipboard_content::selected(&self.host.parsed, cell.content.clone(), markdown) {
                    Ok(p) => p, Err(error) => return Some(Err(error)),
                };
                if markdown { text.push(' '); text.push_str(payload.text.trim()); text.push_str(" |"); }
                else { text.push_str(&payload.text); }
                let rowspan = (cell.row + cell.rowspan).min(selection.anchor.0.max(selection.head.0) + 1) - first_row;
                let colspan = (cell.column + cell.colspan).min(selection.anchor.1.max(selection.head.1) + 1) - first_column;
                html.push_str(&format!("<td rowspan=\"{rowspan}\" colspan=\"{colspan}\">")); html.push_str(&payload.html); html.push_str("</td>");
            }
            html.push_str("</tr>");
            if markdown && row == selection.anchor.0.min(selection.head.0) {
                text.push_str("\n|"); text.push_str(&" --- |".repeat(selection.anchor.1.abs_diff(selection.head.1) + 1));
            }
        }
        html.push_str("</tbody></table>");
        Some(Ok(Payload { text, html }))
    }
    pub(super) fn paste_cells(&mut self, text: &str, html: Option<&str>, cx: &mut Context<Self>) -> bool {
        let Some((table_at, row, column)) = self.table_cell_at(self.local_cursor()) else { return false; };
        let mut values = Vec::new();
        let mut incoming = None;
        if let Some(html) = html {
            let markup = clipboard_content::from_html(html);
            if let Some(table) = crate::nomo_mark::html_table::HtmlTable::parse(&markup, 0..markup.len()) {
                for r in 0..table.rows {
                    values.push((0..table.columns).map(|c| table.cells.iter().find(|cell| cell.row == r && cell.column == c)
                        .map(|cell| clipboard_content::from_html(&markup[cell.content.clone()])).unwrap_or_default()).collect::<Vec<_>>());
                }
                incoming = Some((table, markup));
            }
        }
        if values.is_empty() && (text.contains('\t') || (self.cell_selection.is_some() && text.contains('\n'))) {
            values = text.trim_end_matches(['\r', '\n']).split('\n').map(|line| line.trim_end_matches('\r').split('\t').map(clipboard_content::literal).collect()).collect();
        }
        if values.is_empty() { return false; }
        let parsed = self.host.parsed.clone();
        let Some(original) = grid(&parsed, table_at) else { return false; };
        let (row, column) = self.cell_selection.as_ref().map_or((row, column), |s| (s.anchor.0.min(s.head.0), s.anchor.1.min(s.head.1)));
        let target_rows = original.rows.max(row + values.len());
        let target_columns = original.columns.max(column + values.iter().map(Vec::len).max().unwrap_or(0));
        let result = (|| {
            if target_rows > 512 || target_columns > 128 { return Err(ControlError::Unsupported("粘贴表格超过当前行列预算；原文已保留".into())); }
            if original.cells.iter().any(|cell| cell.rowspan != 1 || cell.colspan != 1)
                || incoming.as_ref().is_some_and(|(table, _)| table.cells.iter().any(|cell| cell.rowspan != 1 || cell.colspan != 1)) {
                if incoming.is_none() {
                    let mut markup = String::from("<table data-nomo-table=\"semantic\"><tbody>");
                    let columns = values.iter().map(Vec::len).max().unwrap_or(0);
                    for line in &values {
                        markup.push_str("<tr>");
                        for c in 0..columns {
                            let content = clipboard_content::html(line.get(c).map_or("", String::as_str));
                            markup.push_str("<td>");
                            if content.is_empty() { markup.push_str("<p></p>"); } else { markup.push_str(&content); }
                            markup.push_str("</td>");
                        }
                        markup.push_str("</tr>");
                    }
                    markup.push_str("</tbody></table>");
                    let table = crate::nomo_mark::html_table::HtmlTable::parse(&markup, 0..markup.len()).ok_or(ControlError::InvalidRange)?;
                    incoming = Some((table, markup));
                }
                let (table, markup) = incoming.as_ref().unwrap();
                return paste_spans(&parsed, &original, row, column, table, markup);
            }
            let selection = DocumentSelection::caret(parsed.identity.from_byte + table_at as u64);
            let resize = crate::nomo_mark::table::TableIntent::Resize { rows: target_rows, columns: target_columns };
            let result = if original.html {
                parsed.html_tables.iter().find(|t| t.source.start == table_at).unwrap().edit(&parsed.source, parsed.identity.from_byte,
                    crate::nomo_mark::html_table::HtmlTableIntent::Resize { rows: target_rows, columns: target_columns })?
            } else { crate::nomo_mark::table::Table::at(&parsed, table_at).unwrap().edit(&parsed, selection, resize)? };
            let mut text = parsed.source.to_string();
            for e in result.edits.iter().rev() { text.replace_range((e.from_byte - parsed.identity.from_byte) as usize..(e.to_byte - parsed.identity.from_byte) as usize, &e.inserted_text); }
            let mut identity = parsed.identity.clone(); identity.to_byte = identity.from_byte + text.len() as u64;
            let mut context = parsed.context.clone();
            crate::markdown_templates::map_templates(&mut context.inline_templates, result.edits.iter().rev()
                .map(|e| (e.from_byte, e.to_byte, e.inserted_text.len() as u64)), false);
            let projected = ParsedWindow::from_source_with_context(identity, text.into(), context);
            let target = grid(&projected, table_at).ok_or(ControlError::InvalidRange)?;
            let mut replacements = Vec::new();
            for (r, line) in values.iter().enumerate() {
                for (c, value) in line.iter().enumerate() {
                    let cell = target.cells.iter().find(|cell| cell.row == row + r && cell.column == column + c)
                        .ok_or_else(|| ControlError::Unsupported("目标区域含合并单元格；原文已保留".into()))?;
                    let content = if target.html {
                        let html = clipboard_content::html(value);
                        if html.starts_with("<p>") { html } else { format!("<p>{}</p>", html_escape::encode_text(value)) }
                    } else { crate::nomo_mark::table_inline::markdown_input("", 0, value).replace('\n', "<br>") };
                    replacements.push((cell.content.clone(), content));
                }
            }
            let mut table_text = projected.source[target.source.clone()].to_string();
            replacements.sort_by_key(|(range, _)| range.start);
            for (range, value) in replacements.into_iter().rev() { table_text.replace_range(range.start - target.source.start..range.end - target.source.start, &value); }
            if table_text.len() > crate::nomo_mark::session_window::policy().group_bytes as usize { return Err(ControlError::NeedsContext("粘贴表格超过局部编辑预算；原文已保留".into())); }
            let cell = target.cells.iter().find(|c| c.row == row && c.column == column).ok_or(ControlError::InvalidRange)?;
            let caret = parsed.identity.from_byte + cell.content.start as u64 + if target.html { 3 } else { 0 };
            Ok(EditResult { edits: vec![SourceEdit { from_byte: parsed.identity.from_byte + original.source.start as u64,
                to_byte: parsed.identity.from_byte + original.source.end as u64, inserted_text: table_text }], selection: DocumentSelection::caret(caret), template_update: None })
        })();
        self.history_serial += 1;
        let group = format!("{}:table-paste:{}", self.host.parsed.identity.window_id, self.history_serial);
        match result { Ok(result) => if let Err(error) = self.queue_result(result, &group, cx) { self.error(error, cx); }, Err(error) => self.error(error, cx) }
        true
    }
}
