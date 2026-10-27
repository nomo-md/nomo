//! 单行输入框（搜索、替换用）。基于 gpui `examples/input.rs` 改写：支持输入法组字，
//! 并修正示例中组字新选区误加 `range.end` 导致的越界。

use crate::theme::{Family, Theme, font_for};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(text_field, [FieldBackspace, FieldDelete, FieldLeft, FieldRight, FieldSelectAll, FieldHome, FieldEnd, FieldPaste, FieldCopy, FieldCut, FieldSubmit, FieldSubmitBack, FieldCancel]);

pub enum FieldEvent {
    Changed,
    Submit { backwards: bool },
    Cancel,
}

pub struct TextField {
    focus: FocusHandle,
    pub content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    popover_style: bool,
    emoji_search_style: bool,
    invalid: bool,
}

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn is_composing(&self) -> bool { self.marked.is_some() }
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.to_string().into(),
            selected: 0..0,
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            popover_style: false,
            emoji_search_style: false,
            invalid: false,
        }
    }

    pub fn set_placeholder(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.placeholder.as_ref() != text { self.placeholder = text.to_string().into(); cx.notify(); }
    }
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selected = 0..self.content.len();
        self.marked = None;
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }
    pub fn select_file_stem(&mut self, cx: &mut Context<Self>) {
        let end = self.content.rfind('.').filter(|index| *index > 0).unwrap_or(self.content.len());
        self.selected = 0..end;
        self.reversed = false;
        cx.notify();
    }
    pub fn set_tab_stop(&mut self) {
        self.focus = self.focus.clone().tab_index(0).tab_stop(true);
    }
    pub fn set_popover_style(&mut self, cx:&mut Context<Self>) {
        self.popover_style=true;
        cx.notify();
    }
    pub fn set_emoji_search_style(&mut self, cx: &mut Context<Self>) {
        self.popover_style = true;
        self.emoji_search_style = true;
        cx.notify();
    }
    pub fn set_invalid(&mut self, invalid:bool, cx:&mut Context<Self>) {
        self.invalid=invalid;
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(index, _)| (index < offset).then_some(index)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(index, _)| (index > offset).then_some(index)).unwrap_or(self.content.len())
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.content.replace_range(range.clone(), text);
        self.selected = range.start + text.len()..range.start + text.len();
        cx.emit(FieldEvent::Changed);
        cx.notify();
    }

    fn on_backspace(&mut self, _: &FieldBackspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.selected.start = self.previous_boundary(self.cursor());
        }
        let range = self.selected.clone();
        self.replace(range, "", cx);
    }

    fn on_delete(&mut self, _: &FieldDelete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.selected.end = self.next_boundary(self.cursor());
        }
        let range = self.selected.clone();
        self.replace(range, "", cx);
    }

    fn on_left(&mut self, _: &FieldLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selected.is_empty() { self.previous_boundary(self.cursor()) } else { self.selected.start };
        self.move_to(target, cx);
    }

    fn on_right(&mut self, _: &FieldRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selected.is_empty() { self.next_boundary(self.cursor()) } else { self.selected.end };
        self.move_to(target, cx);
    }

    fn on_select_all(&mut self, _: &FieldSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        cx.notify();
    }

    fn on_home(&mut self, _: &FieldHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn on_end(&mut self, _: &FieldEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.move_to(end, cx);
    }

    fn on_paste(&mut self, _: &FieldPaste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let range = self.selected.clone();
            self.replace(range, &text.replace('\n', " "), cx);
        }
    }

    fn on_copy(&mut self, _: &FieldCopy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
        }
    }

    fn on_cut(&mut self, _: &FieldCut, window: &mut Window, cx: &mut Context<Self>) {
        let expected = self.content[self.selected.clone()].to_string();
        self.on_copy(&FieldCopy, window, cx);
        if cx.read_from_clipboard().and_then(|item| item.text()) != Some(expected) { return; }
        let range = self.selected.clone();
        self.replace(range, "", cx);
    }

    fn on_submit(&mut self, _: &FieldSubmit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FieldEvent::Submit { backwards: false });
    }

    fn on_submit_back(&mut self, _: &FieldSubmitBack, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FieldEvent::Submit { backwards: true });
    }

    fn on_cancel(&mut self, _: &FieldCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FieldEvent::Cancel);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let offset = match (&self.last_layout, self.last_bounds) {
            (Some(layout), Some(bounds)) => layout.closest_index_for_x(event.position.x - bounds.left()),
            _ => self.content.len(),
        };
        self.move_to(offset, cx);
    }

    fn utf16_to_utf8(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (index, ch) in self.content.char_indices() {
            if units >= utf16 {
                return index;
            }
            units += ch.len_utf16();
        }
        self.content.len()
    }

    fn utf8_to_utf16(&self, utf8: usize) -> usize {
        self.content[..utf8.min(self.content.len())].chars().map(char::len_utf16).sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_utf8(range.start)..self.utf16_to_utf8(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf8_to_utf16(range.start)..self.utf8_to_utf16(range.end)
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|range| self.range_from_utf16(range)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.marked = None;
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, selected_utf16: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|range| self.range_from_utf16(range)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        self.selected = match selected_utf16 {
            Some(selected) => {
                let to_utf8 = |utf16: usize| {
                    let mut units = 0;
                    for (index, ch) in text.char_indices() {
                        if units >= utf16 {
                            return index;
                        }
                        units += ch.len_utf16();
                    }
                    text.len()
                };
                range.start + to_utf8(selected.start)..range.start + to_utf8(selected.end)
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        cx.notify();
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + layout.x_for_index(range.start), bounds.top()),
            point(bounds.left() + layout.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let index = layout.index_for_x(position.x - bounds.left())?;
        Some(self.utf8_to_utf16(index))
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let border=if self.invalid {gpui::rgb(0xef4444).into()}else if self.emoji_search_style {theme.rail_bg}else if self.popover_style && self.focus.is_focused(window) {theme.accent}else{theme.border};
        div()
            .relative().children(animate_theme.then(crate::ui::motion::theme_frame))
            .flex_1()
            .key_context("TextField")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .h(px(if self.popover_style {34.0}else{26.0}))
            .px(px(if self.emoji_search_style {0.0}else if self.popover_style {10.0}else{8.0}))
            .rounded(px(if self.popover_style {4.0}else{6.0}))
            .bg(if self.emoji_search_style {theme.rail_bg}else if self.popover_style {theme.app_bg}else{theme.surface})
            .border_1()
            .border_color(border)
            .flex()
            .items_center()
            .overflow_hidden()
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_submit))
            .on_action(cx.listener(Self::on_submit_back))
            .on_action(cx.listener(Self::on_cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(FieldElement { field: cx.entity() })
    }
}

struct FieldElement {
    field: Entity<TextField>,
}

impl IntoElement for FieldElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct FieldPrepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = FieldPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(20.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> FieldPrepaint {
        let theme = Theme::ui(cx);
        let field = self.field.read(cx);
        let (text, color): (SharedString, Hsla) = if field.content.is_empty() {
            (field.placeholder.clone(), theme.muted)
        } else {
            (field.content.clone().into(), theme.fg)
        };
        let font = if field.popover_style {gpui::font(crate::theme::ui_font_family())}else{font_for(Family::Text, false, false)};
        let base = TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
        let runs = match field.marked.as_ref().filter(|_| !field.content.is_empty()) {
            Some(marked) => vec![
                TextRun { len: marked.start, ..base.clone() },
                TextRun { len: marked.end - marked.start, underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.0), wavy: false }), ..base.clone() },
                TextRun { len: text.len() - marked.end, ..base },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![base],
        };
        let line = window.text_system().shape_line(text, px(if field.popover_style {14.0}else{13.0}), &runs, None);
        let selected = field.selected.clone();
        let (selection, cursor) = if selected.is_empty() || field.content.is_empty() {
            let x = if field.content.is_empty() { px(0.0) } else { line.x_for_index(field.cursor()) };
            (None, Some(fill(Bounds::new(point(bounds.left() + x, bounds.top() + px(2.0)), size(px(1.5), bounds.size.height - px(4.0))), theme.caret)))
        } else {
            (
                Some(fill(
                    Bounds::from_corners(point(bounds.left() + line.x_for_index(selected.start), bounds.top()), point(bounds.left() + line.x_for_index(selected.end), bounds.bottom())),
                    theme.selection,
                )),
                None,
            )
        };
        FieldPrepaint { line: Some(line), cursor, selection }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), prepaint: &mut FieldPrepaint, window: &mut Window, cx: &mut App) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.field.clone()), cx);
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        let line = prepaint.line.take().expect("prepaint");
        let _ = line.paint(bounds.origin, px(20.0), gpui::TextAlign::Left, None, window, cx);
        if focus.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.field.update(cx, |field, _| {
            field.last_layout = Some(line);
            field.last_bounds = Some(bounds);
        });
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let context = Some("TextField");
    cx.bind_keys([
        KeyBinding::new("backspace", FieldBackspace, context),
        KeyBinding::new("delete", FieldDelete, context),
        KeyBinding::new("left", FieldLeft, context),
        KeyBinding::new("right", FieldRight, context),
        crate::ui::key_binding("secondary-a", FieldSelectAll, context, cx),
        KeyBinding::new("home", FieldHome, context),
        KeyBinding::new("end", FieldEnd, context),
        crate::ui::key_binding("secondary-left", FieldHome, context, cx),
        crate::ui::key_binding("secondary-right", FieldEnd, context, cx),
        crate::ui::key_binding("secondary-v", FieldPaste, context, cx),
        crate::ui::key_binding("secondary-c", FieldCopy, context, cx),
        crate::ui::key_binding("secondary-x", FieldCut, context, cx),
        KeyBinding::new("enter", FieldSubmit, context),
        KeyBinding::new("shift-enter", FieldSubmitBack, context),
        KeyBinding::new("escape", FieldCancel, context),
    ]);
}
