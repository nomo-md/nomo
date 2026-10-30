//! 表格内的操作入口复用当前窗口、表格事务和历史，不挂载另一个编辑器。
use super::*;
use crate::nomo_mark::table::{Alignment, TableIntent};
use crate::ui::components::{icon, tooltip, IconName};
use crate::ui::motion::{self, ScalarMotion};
use crate::ui::toolbar_i18n::ToolbarLocale;
use crate::markdown::render::TABLE_TOOLBAR_SPACE;
use gpui::{Animation, AnimationExt, AnyElement, BoxShadow, ClickEvent, RenderOnce};
use crate::ui::popup::PopupPlacement;
use super::node_controls::block_popup;
use std::{cell::Cell, time::Duration};

#[derive(Clone, Copy)]
enum Action { Edit(TableIntent), Size, Delete }

/// 动效状态仅属于已挂载的按钮，隐藏表格或回收窗口时由 GPUI 一起释放。
#[derive(IntoElement)]
struct TableEdgeButton {
    id: String,
    hitbox: gpui::Stateful<gpui::Div>,
    delete: bool,
    zoom: f32,
    font: gpui::SharedString,
}

impl RenderOnce for TableEdgeButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.with_global_id(format!("{}-hover", self.id).into(), |id, window| {
            window.with_element_state(id, |state: Option<Rc<Cell<ScalarMotion>>>, _| {
                let state = state.unwrap_or_default();
                (state.clone(), state)
            })
        });
        let mut animation = state.get();
        animation.retime(crate::ui::popup::reduced_motion(cx), Duration::from_millis(150));
        state.set(animation);
        if animation.running() { window.request_animation_frame(); }
        let progress = animation.value();
        let theme = cx.global::<Theme>();
        let foreground = if self.delete { theme.danger } else { motion::color(theme.muted, theme.accent_strong, progress) };
        let border = if self.delete { theme.danger } else { motion::color(theme.table_border.opacity(0.86), theme.accent, progress) };
        let zoom = self.zoom;
        let visual = div().flex_none().w(px(if self.delete { 24.0 } else { 20.0 } * zoom))
            .h(px(if self.delete { 22.0 } else { 20.0 } * zoom))
            .flex().items_center().justify_center().border(px(zoom)).border_color(border)
            .rounded(px(if self.delete { 5.0 } else { 10.0 } * zoom)).bg(theme.surface)
            .font_family(self.font)
            .font_weight(gpui::FontWeight::BOLD).text_size(px(if self.delete { 11.0 } else { 13.0 } * zoom))
            .line_height(relative(1.0)).text_color(foreground).opacity(progress)
            .shadow(vec![BoxShadow { color: gpui::rgba(0x0f172a1a).into(), offset: point(px(0.0), px(2.0 * zoom)),
                blur_radius: px(6.0 * zoom), spread_radius: px(0.0), inset: false }])
            .child(if self.delete { "−" } else { "+" });
        self.hitbox.child(visual).on_hover(move |hovered, window, cx| {
            let mut animation = state.get();
            animation.set(*hovered, crate::ui::popup::reduced_motion(cx), Duration::from_millis(150));
            state.set(animation);
            window.refresh();
        })
    }
}

impl NomoMark {
    pub(super) fn table_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.effective_mode() != ViewMode::Semantic || self.host.presentation != Presentation::Editable || self.host.interaction.protected() || self.object.is_some() { return None; }
        let (_, column) = self.active_cell()?;
        let at = self.local_cursor();
        let parsed = &self.host.parsed;
        let (source, rows, columns, header) = if let Some(table) = parsed.html_tables.iter().find(|t| t.source.contains(&at)) {
            (table.source.clone(), table.rows, table.columns, table.cells.iter().filter(|c| c.row == 0).all(|c| c.header))
        } else {
            let table = crate::nomo_mark::table::Table::at(parsed, at)?;
            (table.source, table.rows.len(), table.separator.cells.len(), true)
        };
        let layout = self.layout.as_ref()?;
        let geometry = layout.table_cells(source.start);
        let mut points = geometry.iter().map(|c| (c.0, c.1, c.4, c.5, c.7)).collect::<Vec<_>>();
        if points.is_empty() { return None; }
        points.sort_by_key(|p| (p.0, p.1));
        let left = points.iter().map(|p| p.2).fold(f32::INFINITY, f32::min);
        let top = points.iter().map(|p| p.3).fold(f32::INFINITY, f32::min);
        let bottom = points.iter().map(|p| p.3 + p.4).fold(0.0, f32::max);
        let right = geometry.iter().map(|c| c.4 + c.6).fold(0.0, f32::max);
        let theme = cx.global::<Theme>().clone();
        let zoom = cx.global::<Settings>().metrics().zoom;
        let mono = crate::theme::font_for(Family::Mono, true, false).family;
        let locale = ToolbarLocale::current(cx);
        let identity = parsed.identity.clone();
        let button = |id: String, label: String, utility: Option<usize>, action: Action, cx: &mut Context<Self>| {
            let identity = identity.clone(); let source = source.clone();
            let symbol = if utility.is_none() {
                div().into_any_element()
            } else if utility.is_some_and(|index| index < 4) {
                let mut grid = div().absolute().left(px(1.0 * zoom)).top(px(2.0 * zoom)).size(px(13.0 * zoom)).border(px(zoom)).border_color(theme.accent.opacity(0.56)).rounded(px(2.0 * zoom))
                    .child(div().absolute().left(px(5.0 * zoom)).top_0().w(px(zoom)).h_full().bg(theme.accent.opacity(0.28)))
                    .child(div().absolute().left_0().top(px(5.0 * zoom)).h(px(zoom)).w_full().bg(theme.accent.opacity(0.28)));
                let edge = div().absolute().bg(theme.accent_strong);
                grid = grid.child(match utility {
                    Some(0) => edge.left_0().top_0().w_full().h(px(2.0 * zoom)),
                    Some(1) => edge.left_0().top_0().h_full().w(px(2.0 * zoom)),
                    Some(2) => edge.left_0().bottom_0().w_full().h(px(2.0 * zoom)),
                    _ => edge.right_0().top_0().h_full().w(px(2.0 * zoom)),
                });
                div().relative().size(px(18.0 * zoom)).child(grid)
                    .child(div().absolute().right_0().top_0().size(px(7.0 * zoom)).bg(theme.surface)
                        .child(div().absolute().left(px(3.0 * zoom)).w(px(zoom)).h_full().bg(theme.accent_strong))
                        .child(div().absolute().top(px(3.0 * zoom)).h(px(zoom)).w_full().bg(theme.accent_strong))).into_any_element()
            } else if utility.is_some_and(|index| (4..=6).contains(&index)) {
                let mut lines = div().w(px(18.0 * zoom)).flex().flex_col().gap(px(3.0 * zoom));
                for i in 0..4 {
                    let short = i % 2 == 1;
                    let offset = if !short || utility == Some(4) { 0.0 } else if utility == Some(5) { 3.0 } else { 6.0 };
                    lines = lines.child(div().ml(px(offset * zoom)).w(px(if short { 10.0 } else { 16.0 } * zoom)).h(px(2.0 * zoom)).rounded(px(zoom)).bg(theme.muted)
                        .group_hover("table-utility", |s| s.bg(theme.accent_strong)));
                }
                lines.into_any_element()
            } else if utility == Some(7) {
                div().relative().size(px(18.0 * zoom)).flex().items_center().justify_center().text_size(px(14.0 * zoom)).font_weight(gpui::FontWeight::EXTRA_BOLD).text_color(theme.muted)
                    .group_hover("table-utility", |s| s.text_color(theme.accent_strong)).child("H")
                    .child(div().absolute().left(px(zoom)).right(px(zoom)).top(px(zoom)).h(px(2.0 * zoom)).bg(theme.muted.opacity(0.62))
                        .group_hover("table-utility", |s| s.bg(theme.accent_strong.opacity(0.62)))).into_any_element()
            } else if utility == Some(8) {
                div().relative().size(px(18.0 * zoom))
                    .child(div().absolute().left(px(zoom)).top(px(zoom)).size(px(15.0 * zoom)).border(px(1.5 * zoom)).rounded(px(2.0 * zoom)).border_color(theme.muted)
                        .group_hover("table-utility", |s| s.border_color(theme.accent_strong))
                        .child(div().absolute().left(px(6.0 * zoom)).h_full().w(px(zoom)).bg(theme.muted).group_hover("table-utility", |s| s.bg(theme.accent_strong)))
                        .child(div().absolute().top(px(6.0 * zoom)).w_full().h(px(zoom)).bg(theme.muted).group_hover("table-utility", |s| s.bg(theme.accent_strong))))
                    .child(div().absolute().right_0().bottom_0().size(px(7.0 * zoom)).border_b(px(2.0 * zoom)).border_r(px(2.0 * zoom)).border_color(theme.muted)
                        .group_hover("table-utility", |s| s.border_color(theme.accent_strong))).into_any_element()
            } else { icon(IconName::Trash2, 16.0 * zoom, theme.muted).group_hover("table-utility", |s| s.text_color(theme.danger)).into_any_element() };
            let delete = matches!(action, Action::Edit(TableIntent::DeleteRow { .. } | TableIntent::DeleteColumn { .. }));
            let hitbox = div().id(id.clone()).flex().flex_none().items_center().justify_center().cursor_pointer()
                .tooltip(tooltip(label))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    if view.host.parsed.identity != identity || view.host.interaction.protected() || !source.contains(&view.local_cursor()) { return; }
                    match action {
                        Action::Edit(intent) => view.table_command(intent, cx),
                        Action::Size => { view.table_size_picker = !view.table_size_picker; view.table_size_hover = None; cx.notify(); }
                        Action::Delete => {
                            let base = identity.from_byte;
                            view.select(DocumentSelection { anchor_byte: base + source.start as u64, head_byte: base + source.end as u64 }, cx);
                            view.command(EditIntent::Replace(String::new()), cx);
                        }
                    }
                    if view.object.is_none() { window.focus(&view.focus, cx); }
                }));
            if let Some(index) = utility {
                hitbox.group("table-utility").w(px(if index < 4 { 32.0 } else { 34.0 } * zoom)).h(px(30.0 * zoom))
                    .border(px(zoom)).border_color(gpui::transparent_black()).rounded(px(5.0 * zoom))
                    .hover(|s| if index == 9 { s.bg(theme.danger.opacity(0.1)).border_color(theme.danger) } else { s.bg(theme.accent.opacity(0.1)) })
                    .child(symbol).into_any_element()
            } else {
                // 命中区包含原版伪元素的外扩范围，透明时仍能找到按钮。
                let hitbox = hitbox.w(px(36.0 * zoom)).h(px(if delete { 34.0 } else { 36.0 } * zoom));
                TableEdgeButton { id, hitbox, delete, zoom, font: mono.clone() }.into_any_element()
            }
        };
        let mut overlay = div().id(("table-controls", source.start)).absolute().top_0().left_0().w_full().h(px(0.0));
        let available = self.bounds.map_or(383.0 * zoom, |bounds| (f32::from(bounds.size.width) - left).max(0.0));
        let mut bar = div().id("table-toolbar").w(px((383.0 * zoom).min(available))).h(px(38.0 * zoom))
            .flex().items_center().gap(px(3.0 * zoom)).overflow_x_scroll()
            .px(px(4.0 * zoom)).py(px(3.0 * zoom)).rounded(px(8.0 * zoom)).border(px(zoom))
            .border_color(theme.table_border.opacity(0.58)).bg(theme.surface.opacity(0.98))
            .shadow(vec![BoxShadow { color: gpui::rgba(0x0f172a1a).into(), offset: point(px(0.0), px(10.0 * zoom)),
                blur_radius: px(24.0 * zoom), spread_radius: px(0.0), inset: false }]);
        for (index, (label, action)) in [
            ("insertRowBeforeFirst", Action::Edit(TableIntent::InsertRowBefore { row: 0 })),
            ("insertColumnBeforeFirst", Action::Edit(TableIntent::InsertColumn { at: 0 })),
            ("insertRowAfterLast", Action::Edit(TableIntent::InsertRow { after: rows - 1 })),
            ("insertColumnAfterLast", Action::Edit(TableIntent::InsertColumn { at: columns })),
            ("alignLeft", Action::Edit(TableIntent::Align { column, alignment: Alignment::Left })),
            ("alignCenterTable", Action::Edit(TableIntent::Align { column, alignment: Alignment::Center })),
            ("alignRight", Action::Edit(TableIntent::Align { column, alignment: Alignment::Right })),
            ("toggleHeaderRow", Action::Edit(TableIntent::Header { enabled: !header })),
            ("resizeTable", Action::Size),
            ("deleteTable", Action::Delete),
        ].into_iter().enumerate() {
            if index == 4 { bar = bar.child(div().flex_none().mx(px(5.0 * zoom)).w(px(zoom)).h(px(16.0 * zoom)).bg(theme.table_border.opacity(0.54))); }
            bar = bar.child(button(format!("table-util-{index}"), locale.text(label).into(), Some(index), action, cx));
        }
        let reduced = crate::ui::popup::reduced_motion(cx);
        let toolbar = div().absolute().left(px(left)).top(px(top - TABLE_TOOLBAR_SPACE * zoom))
            .child(bar.with_animation("table-toolbar-enter", Animation::new(Duration::from_millis(if reduced { 90 } else { 150 })).with_easing(motion::ease),
                move |bar, progress| bar.opacity(progress).relative().top(px(if reduced { 0.0 } else { 4.0 * zoom * (1.0 - progress) }))));
        if self.table_size_picker {
            let (pr, pc) = self.table_size_hover.unwrap_or((rows, columns));
            let mut grid = div().id("table-size-grid").w_full().max_h(px(260.0)).min_w_0()
                .overflow_y_scroll().overflow_x_scroll().flex().flex_col().gap(px(3.0));
            for r in 1..=rows.clamp(5, 32) {
                let mut line = div().flex().flex_none().gap(px(3.0));
                for c in 1..=columns.clamp(6, 32) {
                    let identity = identity.clone();
                    line = line.child(div().id(("table-size-cell", r * 33 + c)).w(px(28.0)).h(px(28.0)).flex_none().border_1().border_color(theme.border)
                        .when(r <= pr && c <= pc, |el| el.bg(theme.selection))
                        .cursor_pointer().tooltip(tooltip(locale.text("resizeTableSize").replace("{rows}", &r.to_string()).replace("{columns}", &c.to_string())))
                        .on_mouse_move(cx.listener(move |view, _, _, cx| { if view.table_size_hover != Some((r, c)) { view.table_size_hover = Some((r, c)); cx.notify(); } }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if view.host.parsed.identity != identity { return; }
                            view.table_size_picker = false;
                            view.table_command(TableIntent::Resize { rows: r, columns: c }, cx);
                            window.focus(&view.focus, cx);
                        })));
                }
                grid = grid.child(line);
            }
            let picker = div().id("table-size-picker").w(px(((columns + 3).clamp(6, 32) as f32 * 31.0 + 16.0).min(540.0)))
                .flex().flex_col().gap(px(6.0)).p(px(8.0)).bg(theme.surface).border_1().border_color(theme.border).rounded(px(6.0))
                .child(div().text_size(px(12.0)).child(format!("{}    {pr} × {pc}", locale.text("resizeTable")))).child(grid);
            overlay = overlay.child(block_popup("table-size-open", left, top, picker, PopupPlacement::BelowLeft, 4, cx));
        }
        // 与旧版的层次一致：插入、删除、工具条；外扩命中区交叠时删除优先。
        let mut insertions = Vec::new();
        let mut deletions = Vec::new();
        for row in 0..rows {
            let Some(cell) = geometry.iter().filter(|c| c.0 <= row && row < c.0 + c.2).min_by_key(|c| c.2) else { continue; };
            let y = cell.5 + cell.7 * (row - cell.0) as f32 / cell.2 as f32;
            let h = cell.7 / cell.2 as f32;
            deletions.push(div().absolute().left(px(left - 38.0 * zoom)).top(px(y + h / 2.0 - 17.0 * zoom))
                .child(button(format!("table-row-delete-{row}"), locale.text("deleteTableRow").replace("{index}", &(row + 1).to_string()), None, Action::Edit(TableIntent::DeleteRow { row }), cx)));
            if row > 0 { for (side, x) in [left, right].into_iter().enumerate() {
                insertions.push(div().absolute().left(px(x - 18.0 * zoom)).top(px(y - 18.0 * zoom))
                    .child(button(format!("table-row-add-{row}-{side}"), locale.text("insertRowBetween").replace("{from}", &row.to_string()).replace("{to}", &(row + 1).to_string()), None, Action::Edit(TableIntent::InsertRowBefore { row }), cx)));
            }}
        }
        for column in 0..columns {
            let Some(cell) = geometry.iter().filter(|c| c.1 <= column && column < c.1 + c.3).min_by_key(|c| c.3) else { continue; };
            let x = cell.4 + cell.6 * (column - cell.1) as f32 / cell.3 as f32;
            let w = cell.6 / cell.3 as f32;
            deletions.push(div().absolute().left(px(x + w / 2.0 - 18.0 * zoom)).top(px(bottom + 2.0 * zoom))
                .child(button(format!("table-col-delete-{column}"), locale.text("deleteTableColumn").replace("{index}", &(column + 1).to_string()), None, Action::Edit(TableIntent::DeleteColumn { column }), cx)));
            if column > 0 { for (side, y) in [top, bottom].into_iter().enumerate() {
                insertions.push(div().absolute().left(px(x - 18.0 * zoom)).top(px(y - 18.0 * zoom))
                    .child(button(format!("table-col-add-{column}-{side}"), locale.text("insertColumnBetween").replace("{from}", &column.to_string()).replace("{to}", &(column + 1).to_string()), None, Action::Edit(TableIntent::InsertColumn { at: column }), cx)));
            }}
        }
        // 工具条在预留行内最后绘制，列插入按钮的扩展命中区不能挡住工具条。
        Some(overlay.children(insertions).children(deletions).child(toolbar).into_any_element())
    }
}
