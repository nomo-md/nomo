//! 键盘动作复用现有事务和窗口请求；草稿与 IME 期间不移动宿主选区。
use super::*;
use crate::nomo_mark::navigation::{self, CursorUnit};

impl NomoMark {
    pub(super) fn keyboard_protected(&self) -> bool { self.host.interaction.composing || self.host.interaction.draft }

    pub(crate) fn defer_navigation_caret(&mut self, cx: &mut Context<Self>) {
        self.pending_navigation_caret = true;
        cx.notify();
    }

    pub(super) fn resolve_navigation_caret(&mut self, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        if self.effective_mode() != ViewMode::Semantic { self.pending_navigation_caret = false; return; }
        let selection = self.host.selection;
        if !self.pending_navigation_caret && !selection.range().is_empty() { return; }
        let identity = &self.host.parsed.identity;
        if selection.head_byte < identity.from_byte || selection.head_byte > identity.to_byte { return; }
        let Some(at) = self.layout.as_ref().and_then(|layout| layout.nearest_caret(self.local_cursor())) else { return; };
        let navigation = self.pending_navigation_caret;
        self.pending_navigation_caret = false;
        let head_byte = identity.from_byte + at as u64;
        if head_byte == selection.head_byte { return; }
        let selection = DocumentSelection {
            anchor_byte: if selection.range().is_empty() { head_byte } else { selection.anchor_byte },
            head_byte,
        };
        // 恢复只校正本视图；真实导航才同步全局锚点，避免把模式/历史恢复伪装成用户移动。
        self.host.selection = selection;
        if navigation { cx.emit(ControlEvent::SelectionChanged { identity: identity.clone(), selection, reveal: true }); }
        cx.notify();
    }

    pub(super) fn document_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        self.preferred_x = None;
        if self.windowed_navigation {
            cx.emit(ControlEvent::DocumentBoundary { identity: self.host.parsed.identity.clone(), end, extend_selection: extend });
        } else { self.move_to(if end { self.host.parsed.source.len() } else { 0 }, extend, cx); }
    }

    pub(super) fn page(&mut self, down: bool, extend: bool, window: &Window, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        if self.move_in_code(down, extend, true, cx) { return; }
        if self.windowed_navigation {
            cx.emit(ControlEvent::VerticalNavigation { identity: self.host.parsed.identity.clone(), down, extend_selection: extend, page: true });
            return;
        }
        let Some(layout) = &self.layout else { return; };
        let Some((x,y,h)) = self.visual_caret_at(self.local_cursor()) else { return; };
        let x = self.preferred_x.unwrap_or(x);
        let visible_height = self.visible_bounds.map_or(f32::from(window.viewport_size().height), |bounds| f32::from(bounds.size.height));
        let distance = (visible_height - h).max(h);
        let target = (y + if down { distance } else { -distance }).clamp(0.0, (layout.height - h).max(0.0));
        if let Some(at) = layout.source_at(x, target) { self.preferred_x = Some(x); self.move_to_row(at, target, extend, cx); }
    }

    pub(super) fn delete_boundary(&mut self, forward: bool, unit: CursorUnit, cx: &mut Context<Self>) {
        if self.keyboard_protected() { return; }
        let identity = &self.host.parsed.identity;
        let selected = self.host.selection.range();
        let at = self.local_cursor();
        let target = if unit != CursorUnit::Line && self.effective_mode() == ViewMode::Semantic {
            match self.layout.as_ref().and_then(|layout| layout.horizontal_caret(at,unit,forward)) {
                Some(target) => target,
                None if at == 0 || at == self.host.parsed.source.len() => at,
                None => { self.error(ControlError::NeedsContext("此结构边界不能按词删除隐藏语法；请移动到文字后删除".into()),cx); return; }
            }
        } else { navigation::boundary(&self.host.parsed.source, at, unit, forward) };
        if selected.is_empty() && self.windowed_navigation && ((target == 0 && identity.from_byte > 0) || (forward && target == self.host.parsed.source.len())) {
            cx.emit(ControlEvent::CursorBoundary { identity: identity.clone(), unit, forward, extend_selection: false, delete: true });
            return;
        }
        let selection = if selected.is_empty() { DocumentSelection { anchor_byte: self.host.selection.head_byte, head_byte: identity.from_byte + target as u64 } } else { self.host.selection };
        if selection.range().is_empty() { return; }
        if local_range(&self.host.parsed.source, identity.from_byte, selection.range()).is_err() {
            cx.emit(ControlEvent::EditOutsideWindow { identity: identity.clone(), selection, intent: EditIntent::Replace(String::new()) });
            return;
        }
        let result = if self.effective_mode() == ViewMode::Semantic { commands::apply_semantic(&self.host.parsed, selection, EditIntent::Backspace) }
            else { commands::apply(&self.host.parsed, selection, EditIntent::Backspace) };
        match result {
            Ok(result) => { self.history_serial += 1; let group = format!("{}:delete:{}", identity.window_id, self.history_serial); if let Err(error) = self.queue_result(result, &group, cx) { self.error(error, cx); } }
            Err(error) => self.error(error, cx),
        }
        self.preferred_x = None;
    }

    pub(super) fn resolve_page_cursor(&mut self, cx: &mut Context<Self>) {
        let Some((x, fraction, anchor)) = self.pending_page.take() else { return; };
        let Some(layout) = &self.layout else { return; };
        if let Some(at) = layout.source_at(x, (layout.height * fraction).clamp(0.0, (layout.height - 1.0).max(0.0))) {
            let head_byte = self.host.parsed.identity.from_byte + at as u64;
            self.preferred_x = Some(x);
            self.select(DocumentSelection { anchor_byte: anchor.unwrap_or(head_byte), head_byte }, cx);
        }
    }

    pub(crate) fn page_origin(&self) -> Option<(f32, f32, f32)> {
        if self.layout_key.as_ref()?.0.identity != self.host.parsed.identity { return None; }
        let (x,y,h) = self.visual_caret_at(self.local_cursor())?;
        Some((self.preferred_x.unwrap_or(x),y,h))
    }
    pub(crate) fn move_page_to(&mut self, x: f32, y: f32, extend: bool, cx: &mut Context<Self>) {
        if let Some(at) = self.layout.as_ref().and_then(|layout| layout.source_at(x,y)) { self.preferred_x = Some(x); self.move_to_row(at, y, extend, cx); }
    }
    pub(crate) fn defer_page_cursor(&mut self, x: f32, fraction: f32, anchor: Option<u64>, cx: &mut Context<Self>) {
        self.pending_page = Some((x,fraction,anchor));
        self.invalidate();
        cx.notify();
    }
}
