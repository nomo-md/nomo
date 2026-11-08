//! 正式工作区与 NomoMark 共用的编辑动作和视图模式。
//! 动作名称保持 editor 命名空间；键位绑定由各活动控件负责。
use gpui::actions;

actions!(
    editor,
    [
        MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown, MoveWordLeft,
        MoveWordRight, SelectWordLeft, SelectWordRight, MoveLineStart, MoveLineEnd, SelectLineStart, SelectLineEnd,
        MoveDocStart, MoveDocEnd, SelectDocStart, SelectDocEnd, Backspace, Delete, DeleteWordLeft, DeleteLineLeft,
        Newline, Indent, Outdent, SelectAll, Undo, Redo, Copy, Cut, Paste, PageUp, PageDown, CharacterPalette
    ]
);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Source,
    Semantic,
}
