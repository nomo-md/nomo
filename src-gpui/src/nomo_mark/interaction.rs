//! 视图交互保护不等于文档历史；宿主仍决定何时回收窗口。
use super::parse::ParsedWindow;
use super::protocol::{ControlError, Presentation};
use std::ops::Range;

#[derive(Default)]
pub struct EditorInteraction {
    pub composing: bool,
    pub(super) marked: Option<Range<usize>>,
    pub dragging: bool,
    pub drag_selection_started: bool,
    pub draft: bool,
    /// 全局源码锚点上的临时展开状态，不改写 Details 的默认 open 属性。
    pub details_open: std::collections::BTreeMap<u64, bool>,
    pub horizontal_offsets: std::collections::BTreeMap<u64, f32>,
    revealed: Vec<Range<usize>>,
    composition_anchor: Option<usize>,
}
impl EditorInteraction {
    pub fn protected(&self) -> bool {
        self.composing || self.dragging || self.draft
    }
    pub fn begin_composition(&mut self, at: usize) {
        self.composing = true;
        self.composition_anchor.get_or_insert(at);
    }
    /// 空组字串表示取消或上屏后的收尾，不能留下占用旧光标位置的空标记。
    pub fn mark_text(&mut self, range: Range<usize>) {
        if range.is_empty() {
            self.end_composition();
        } else {
            self.begin_composition(range.start);
            self.marked = Some(range);
        }
    }
    pub fn end_composition(&mut self) {
        self.composing = false;
        self.marked = None;
        self.composition_anchor = None;
        self.revealed.clear();
    }
    pub fn markers(&mut self, parsed: &ParsedWindow, selection: Range<usize>) -> &[Range<usize>] {
        let selection = self.composition_anchor.map_or(selection, |at| at..at);
        let next = parsed.markers_for(&selection);
        if !self.drag_selection_started && !self.composing {
            self.revealed.clear();
        }
        for range in next {
            if !self.revealed.contains(&range) {
                self.revealed.push(range);
            }
        }
        self.revealed.sort_by_key(|r| (r.start, r.end));
        &self.revealed
    }
    pub fn prepare_detach(
        &self,
        pending: usize,
        presentation: Presentation,
    ) -> Result<(), ControlError> {
        if presentation == Presentation::Detached {
            return Ok(());
        }
        if self.protected() || pending > 0 || presentation == Presentation::Failed {
            Err(ControlError::Busy)
        } else {
            Ok(())
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_ime_preedit_releases_selection_after_commit_cancel_and_retype() {
        use crate::nomo_mark::{commands::{self, EditIntent}, host::WindowHostAdapter, protocol::*};
        let mut host = WindowHostAdapter::new(DocumentWindow {
            document_id: "ime".into(), revision: 1, from_byte: 100, to_byte: 106,
            text: "你好".into(), inline_templates: Vec::new(),
        }, "input", 0, WindowContext::Complete).unwrap();
        host.mark_layout_ready(LayoutStamp {
            identity: host.parsed.identity.clone(), width: 600.0,
            font_generation: 0, resource_generation: 0, zoom: 1.0,
        }, 24.0).unwrap();
        host.selection = DocumentSelection::caret(106);
        // RESULTSTR 已上屏，随后 COMPSTR 为空；两者都必须释放组字保护。
        host.interaction.mark_text(0..5);
        host.interaction.end_composition();
        host.interaction.mark_text(6..6);
        assert_eq!(host.interaction.marked, None);
        assert!(host.interaction.prepare_detach(0, Presentation::Editable).is_ok());

        let edit = commands::apply(&host.parsed, host.selection, EditIntent::Backspace).unwrap();
        host.queue_edit(edit.edits, edit.selection, "delete").unwrap();
        assert_eq!(&*host.parsed.source, "你");
        let selected = crate::nomo_mark::mapping::local_range(
            &host.parsed.source, 100, host.selection.range(),
        ).unwrap();
        let range = host.interaction.marked.clone().unwrap_or(selected);
        assert_eq!(range, 3..3);
        host.queue_edit(vec![SourceEdit {
            from_byte: 100 + range.start as u64, to_byte: 100 + range.end as u64,
            inserted_text: "h".into(),
        }], DocumentSelection::caret(104), "typing").unwrap();
        host.interaction.mark_text(3..4);
        assert_eq!(&*host.parsed.source, "你h");
        assert!(host.interaction.protected());

        // 取消这次组字后再在新位置输入，不能复用第一次的锚点和范围。
        host.queue_edit(vec![SourceEdit { from_byte: 103, to_byte: 104, inserted_text: String::new() }],
            DocumentSelection::caret(103), "typing").unwrap();
        host.interaction.mark_text(3..3);
        assert!(!host.interaction.protected());
        assert_eq!(host.interaction.marked, None);
        assert_eq!(host.interaction.composition_anchor, None);
        host.interaction.mark_text(0..1);
        assert_eq!(host.interaction.composition_anchor, Some(0));
        host.interaction.end_composition();
        assert_eq!(host.interaction.marked, None);
    }
    #[test]
    fn drag_reveals_union_and_composition_pins_window() {
        let parsed = crate::nomo_mark::parse::fixture("**甲** *乙*");
        let mut state = EditorInteraction::default();
        state.dragging = true;
        state.drag_selection_started = true;
        assert_eq!(state.markers(&parsed, 3..3).len(), 2);
        assert_eq!(state.markers(&parsed, 11..11).len(), 4);
        assert_eq!(
            state.prepare_detach(0, Presentation::Editable),
            Err(ControlError::Busy)
        );
        state.dragging = false;
        state.begin_composition(3);
        assert_eq!(
            state.prepare_detach(0, Presentation::Editable),
            Err(ControlError::Busy)
        );
        state.end_composition();
        assert!(state.prepare_detach(0, Presentation::Editable).is_ok());
    }
}
