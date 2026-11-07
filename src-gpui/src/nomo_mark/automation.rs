//! 测试侧只投影当前原生状态，命令沿用正式编辑/导航入口，不持有控件副本。
use super::*;
use gpui::App;
use serde_json::{Value, json};

impl DocumentHost {
    pub(crate) fn automation_state(&self, cx: &Context<Self>) -> Value {
        let view = self.control.read(cx);
        let pending: usize = self.panes.iter().flat_map(|p| &p.viewport.mounts).map(|m| m.view.read(cx).host.pending_count()).sum();
        let editable = !self.hidden && self.ready(cx).is_ok() && view.host.presentation == Presentation::Editable && pending == 0;
        let panes: Vec<_> = self.panes.iter().enumerate().map(|(index, pane)| {
            let bounds = pane.scroll.bounds();
            let source: usize = pane.viewport.mounts.iter().map(|m| m.view.read(cx).host.parsed.source.len()).sum();
            let estimated: usize = pane.viewport.mounts.iter().map(|m| m.view.read(cx).estimated_bytes()).sum();
            json!({"id":pane.id,"mode":format!("{:?}",pane.mode),"views":pane.viewport.mounts.len(),
                "sourceBytes":source,"estimatedBytes":estimated,"loading":pane.viewport.loading,"dirty":pane.viewport.dirty,
                "referencesLoading":pane.viewport.references_loading,"referencesFailed":pane.viewport.references_failed,
                "visibleReady":self.pane_visible_ready(index,cx),"error":pane.load_error,
                "scrollTop":-f32::from(pane.scroll.offset().y),"viewportHeight":f32::from(pane.scroll.bounds().size.height),
                "viewportBounds":{"x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),
                    "width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)},
                "contentHeight":pane.viewport.heights.total()})
        }).collect();
        let stable = editable && panes.iter().all(|p| p["visibleReady"] == true && p["referencesLoading"] == false
            && p["referencesFailed"] == false && p["error"].is_null()) && !view.host.parsed.context.pending_references;
        let reading = self.workspace_reading_position(cx).map(|(byte,fraction)| json!({"byte":byte,"fraction":fraction}));
        let search = self.search();
        json!({"document":self.status(),"selection":self.selection(cx),"reading":reading,
            "readingByte":reading.as_ref().and_then(|r|r["byte"].as_u64()),
            "ready":{"readable":!self.closed && view.measured_height().is_some(),"editable":editable,"stable":stable},
            "pendingEdits":pending,"saving":self.saving,"navigating":self.navigating,"visible":self.is_visible(),
            "failure":view.host.failure().map(|f|format!("{f:?}")),"panes":panes,
            "search":{"pending":search.pending,"count":search.total,"matches":search.matches,"error":search.error},
            "controlId":format!("{:?}",self.control.entity_id()),"message":self.message})
    }

    pub(crate) fn automation_select(&mut self, selection: DocumentSelection, cx: &mut Context<Self>) -> Result<(), String> {
        self.ready(cx).map_err(|e|e.to_string())?;
        if selection.anchor_byte > self.total_bytes || selection.head_byte > self.total_bytes { return Err("选区越界".into()); }
        let view = self.control.read(cx);
        let identity = &view.host.parsed.identity;
        if selection.head_byte >= identity.from_byte && selection.head_byte <= identity.to_byte {
            let at = (selection.head_byte - identity.from_byte) as usize;
            if !view.host.parsed.source.is_char_boundary(at) { return Err("选区不在 UTF-8 边界".into()); }
            self.control.update(cx,|view,cx|view.select(selection,cx));
            self.panes[self.active_pane].reveal_selection = true;
            self.focus_pending = true;
            cx.notify();
        } else { self.navigate(selection.head_byte,Some(selection),cx); }
        Ok(())
    }
    pub(crate) fn automation_table(&mut self, command: &str, alignment: &str, cx: &mut Context<Self>) -> Result<(), String> {
        use crate::nomo_mark::table::{TableIntent, Alignment};
        self.ready(cx).map_err(|e|e.to_string())?;
        let (row,column) = self.control.read(cx).active_cell().ok_or("光标不在表格单元格")?;
        let intent = match command {
            "addTableRowAfter" => TableIntent::InsertRow {after:row},
            "addTableColumnAfter" => TableIntent::InsertColumn {at:column+1},
            "deleteTableRow" => TableIntent::DeleteRow {row},
            "deleteTableColumn" => TableIntent::DeleteColumn {column},
            "setTableColumnAlignment" => TableIntent::Align {column, alignment:match alignment {
                "left"=>Alignment::Left,"right"=>Alignment::Right,"center"=>Alignment::Center,_=>return Err("无效对齐".into())}},
            _ => return Err("不支持的表格命令".into()),
        };
        self.control.update(cx,|view,cx|view.table_command(intent,cx));
        Ok(())
    }
    pub(crate) fn automation_scroll(&mut self, pixels: f32, cx: &mut Context<Self>) -> Result<(), String> {
        if !pixels.is_finite() || pixels.abs()>100_000.0 {return Err("滚动距离无效".into());}
        let pane=&mut self.panes[self.active_pane];
        let offset=pane.scroll.offset();
        pane.reveal_selection=false;
        pane.scroll.set_offset(gpui::point(offset.x,(offset.y-px(pixels)).min(px(0.0))));
        self.schedule_viewport(16,cx);
        cx.notify();
        Ok(())
    }
    pub(crate) fn automation_focus(&self, window: &mut Window, cx: &mut App) {window.focus(&self.focus_handle(cx),cx);}
}
