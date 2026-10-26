//! 视图交互保护不等于文档历史；宿主仍决定何时回收窗口。
use super::parse::ParsedWindow;
use super::protocol::{ControlError, Presentation};
use std::ops::Range;

#[derive(Default)]
pub struct EditorInteraction {
    pub composing: bool,
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
    pub fn end_composition(&mut self) {
        self.composing = false;
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
    fn drag_reveals_union_and_composition_pins_window() {
        let parsed = crate::editor_control::parse::fixture("**甲** *乙*");
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
