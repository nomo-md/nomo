//! 链接快捷编辑和表格尺寸选择器。只持有浮层输入状态，文档/选区/修订验证由宿主负责。
//! 外观与网格基准：master 339fb932 的 LinkQuickEditor、EditorToolbar 和 app-chrome.css。

use super::{components::{IconName, tooltip}, text_field::{FieldEvent, FieldSelectAll, TextField}};
use crate::theme::{Family, Theme, font_for, ui_font_family};
use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, Hsla, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Render, SharedString, Stateful, Subscription, Window, div, point, prelude::*, px, rgba,
};

pub const LINK_POPOVER_WIDTH: f32 = 392.0;
pub const TABLE_POPOVER_WIDTH: f32 = 236.0;
const TABLE_ROWS: usize = 5;
const TABLE_COLUMNS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorPopoverKind { Link, Table }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorPopoverEvent {
    ApplyLink { text: String, href: String },
    RemoveLink,
    /// 与主分支保持一致：rows 是正文行数，宿主另建表头；columns 在宿主至少取 2。
    InsertTable { rows: usize, columns: usize },
    Dismiss,
}

enum PopoverContent {
    Link { text: Entity<TextField>, href: Entity<TextField>, can_remove: bool },
    Table { rows: usize, columns: usize, keyboard_cell: Option<usize> },
}

pub struct EditorPopover {
    focus: FocusHandle,
    content: PopoverContent,
    error: Option<SharedString>,
    trigger_bounds: Option<std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>>,
    /// 仅订阅本浮层拥有的输入框；丢弃浮层时一起释放，不捕获文档或宿主。
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorPopoverEvent> for EditorPopover {}
impl Focusable for EditorPopover {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.initial_focus_handle(cx) }
}

impl EditorPopover {
    pub fn new_link(text: impl Into<String>, href: impl Into<String>, can_remove: bool, cx: &mut Context<Self>) -> Self {
        let text_value = text.into();
        let href_value = href.into();
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let text = cx.new(|cx| {
            let mut field = TextField::new(locale.text("link_text"), cx);
            field.set_popover_style(cx);
            field.set_text(&text_value, cx);
            field
        });
        let href = cx.new(|cx| {
            let mut field = TextField::new("https://example.com", cx);
            field.set_popover_style(cx);
            field.set_text(&href_value, cx);
            field
        });
        let title_events = cx.subscribe(&text, |this, _, event, cx| this.on_field_event(event, false, cx));
        let href_events = cx.subscribe(&href, |this, _, event, cx| this.on_field_event(event, true, cx));
        Self {
            focus: cx.focus_handle(), content: PopoverContent::Link { text, href, can_remove },
            error: None, trigger_bounds: None, _subscriptions: vec![title_events, href_events],
        }
    }

    pub fn new_table(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: PopoverContent::Table { rows: 3, columns: 4, keyboard_cell: None },
            error: None, trigger_bounds: None, _subscriptions: Vec::new(),
        }
    }

    pub fn kind(&self) -> EditorPopoverKind {
        match self.content { PopoverContent::Link { .. } => EditorPopoverKind::Link, PopoverContent::Table { .. } => EditorPopoverKind::Table }
    }

    pub fn set_trigger_bounds(&mut self, bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>) {
        self.trigger_bounds = Some(bounds);
    }

    /// 宿主创建后自行聚焦；链接的初始标题内容已全选。
    pub fn initial_focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content { PopoverContent::Link { text, .. } => text.read(cx).focus_handle(cx), PopoverContent::Table { .. } => self.focus.clone() }
    }

    pub fn set_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.error = error.filter(|value| !value.is_empty());
        if let PopoverContent::Link { href, .. } = &self.content {
            href.update(cx, |field, cx| field.set_invalid(self.error.is_some(), cx));
        }
        cx.notify();
    }

    fn is_composing(&self, cx: &App) -> bool {
        match &self.content {
            PopoverContent::Link { text, href, .. } => text.read(cx).is_composing() || href.read(cx).is_composing(),
            PopoverContent::Table { .. } => false,
        }
    }

    fn on_field_event(&mut self, event: &FieldEvent, address: bool, cx: &mut Context<Self>) {
        match event {
            FieldEvent::Changed => {
                // 主分支只在地址输入变化时清除地址错误，修改标题保留错误。
                if address && self.error.take().is_some() {
                    if let PopoverContent::Link { href, .. } = &self.content {
                        href.update(cx, |field, cx| field.set_invalid(false, cx));
                    }
                    cx.notify();
                }
            }
            FieldEvent::Submit { .. } => self.apply_link(cx),
            FieldEvent::Cancel if !self.is_composing(cx) => cx.emit(EditorPopoverEvent::Dismiss),
            FieldEvent::Cancel => {},
        }
    }

    fn apply_link(&self, cx: &mut Context<Self>) {
        if self.is_composing(cx) { return; }
        if let PopoverContent::Link { text, href, .. } = &self.content {
            // 保留用户原始输入。协议校验、文档定位及成功后关闭均由宿主执行。
            cx.emit(EditorPopoverEvent::ApplyLink {
                text: text.read(cx).content.clone(), href: href.read(cx).content.clone(),
            });
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_composing(cx) { return; }
        match &mut self.content {
            PopoverContent::Link { text, href, .. } => {
                if event.keystroke.key == "tab" {
                    let title_focus = text.read(cx).focus_handle(cx);
                    let next = if title_focus.is_focused(window) { href.read(cx).focus_handle(cx) } else { title_focus };
                    window.focus(&next);
                    window.dispatch_action(Box::new(FieldSelectAll), cx);
                    cx.stop_propagation();
                }
                // Enter/Escape 交给 TextField 的既有动作与订阅，避免一次按键重复提交。
            }
            PopoverContent::Table { rows, columns, keyboard_cell } => {
                match event.keystroke.key.as_str() {
                    "escape" => cx.emit(EditorPopoverEvent::Dismiss),
                    "enter" | "space" => cx.emit(EditorPopoverEvent::InsertTable { rows: *rows, columns: *columns }),
                    "tab" => {
                        let count = TABLE_ROWS * TABLE_COLUMNS;
                        let next = if event.keystroke.modifiers.shift {
                            keyboard_cell.map_or(count - 1, |cell| (cell + count - 1) % count)
                        } else { keyboard_cell.map_or(0, |cell| (cell + 1) % count) };
                        *keyboard_cell = Some(next); *rows = next / TABLE_COLUMNS + 1; *columns = next % TABLE_COLUMNS + 1;
                        cx.notify();
                    }
                    "left" | "right" | "up" | "down" => {
                        match event.keystroke.key.as_str() {
                            "left" => *columns = columns.saturating_sub(1).max(1),
                            "right" => *columns = (*columns + 1).min(TABLE_COLUMNS),
                            "up" => *rows = rows.saturating_sub(1).max(1),
                            _ => *rows = (*rows + 1).min(TABLE_ROWS),
                        }
                        *keyboard_cell = Some((*rows - 1) * TABLE_COLUMNS + *columns - 1);
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }
        }
    }

    fn button(&self, id: &'static str, name: Option<IconName>, primary: bool, theme: &Theme) -> Stateful<Div> {
        div().id(id).w(px(32.0)).h(px(28.0)).flex_none().flex().items_center().justify_center()
            .rounded(px(4.0)).border_1().border_color(if primary { theme.accent } else { theme.border })
            .bg(if primary { theme.accent } else { theme.rail_bg })
            .text_color(if primary { theme.on_accent } else { theme.muted }).cursor_pointer()
            .hover(|style| style.border_color(theme.accent).text_color(if primary { theme.on_accent } else { theme.accent_strong }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(name.map(|name| gpui::svg().path(name.path()).size(px(16.0)).flex_none()))
    }

    fn shell(&self, id: &'static str, theme: &Theme, animate_theme: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        div().id(id).relative().track_focus(&self.focus).key_context("EditorPopover")
            .children(animate_theme.then(crate::ui::motion::theme_frame))
            .font_family(ui_font_family()).text_size(px(14.0)).text_color(theme.fg)
            .flex().flex_col().flex_none().rounded(px(8.0)).border_1().occlude()
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|popover, event: &gpui::MouseDownEvent, _, cx| {
                if !popover.trigger_bounds.as_ref().and_then(|bounds| bounds.get()).is_some_and(|bounds| bounds.contains(&event.position)) {
                    cx.emit(EditorPopoverEvent::Dismiss);
                }
            }))
    }

    fn error_text(&self, _: &Theme) -> Option<AnyElement> {
        self.error.as_ref().map(|message| div().mx(px(2.0)).text_size(px(12.0)).line_height(px(16.8))
            .text_color(gpui::rgb(0xef4444))
            .child(message.clone()).into_any_element())
    }
}

impl Render for EditorPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let animate_theme = Theme::ui_transition_active(cx);
        let theme = Theme::ui(cx);
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        match &self.content {
            PopoverContent::Link { text, href, can_remove } => {
                text.update(cx, |field, cx| field.set_placeholder(locale.text("link_text"), cx));
                let bg = theme.surface.blend(theme.accent.opacity(0.04));
                let border = theme.border.blend(theme.accent.opacity(0.28));
                let mut url_row = div().flex().items_center().gap(px(6.0))
                    .child(div().flex_1().min_w(px(0.0)).child(href.clone()))
                    .child(self.button("link-apply", Some(IconName::Check), true, &theme).tooltip(tooltip(locale.text("link_apply")))
                        .on_click(cx.listener(|this, _, _, cx| this.apply_link(cx))));
                if *can_remove {
                    url_row = url_row.child(self.button("link-remove", None, false, &theme).child(trash_icon()).tooltip(tooltip(locale.text("link_remove")))
                        .on_click(cx.listener(|this, _, _, cx| { if !this.is_composing(cx) { cx.emit(EditorPopoverEvent::RemoveLink); } })));
                }
                url_row = url_row.child(self.button("link-dismiss", Some(IconName::Close), false, &theme).tooltip(tooltip(locale.text("close_link")))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorPopoverEvent::Dismiss))));
                self.shell("link-popover", &theme, animate_theme, cx)
                    .w(px(LINK_POPOVER_WIDTH).min((window.viewport_size().width - px(24.0)).max(px(0.0))))
                    .p(px(8.0)).gap(px(7.0)).bg(bg).border_color(border)
                    .shadow(vec![BoxShadow { color: rgba(if theme.dark { 0x0000006b } else { 0x00000038 }).into(), offset: point(px(0.0), px(20.0)), blur_radius: px(60.0), spread_radius: px(0.0) }])
                    .child(link_pointer(bg, border))
                    .child(div().flex().flex_col().gap(px(6.0)).child(div().w_full().child(text.clone())).child(url_row))
                    .children(self.error_text(&theme)).into_any_element()
            }
            PopoverContent::Table { rows, columns, keyboard_cell } => {
                let (preview_rows, preview_columns, selected) = (*rows, *columns, *keyboard_cell);
                let mut grid = div().flex().flex_col().gap(px(4.0));
                for row in 1..=TABLE_ROWS {
                    let mut grid_row = div().flex().gap(px(4.0));
                    for column in 1..=TABLE_COLUMNS {
                        let active = row <= preview_rows && column <= preview_columns;
                        let keyboard_active = selected == Some((row - 1) * TABLE_COLUMNS + column - 1);
                        grid_row = grid_row.child(div().id(("table-size", row * TABLE_COLUMNS + column))
                            .w(px(32.0)).h(px(28.0)).flex_none().rounded(px(4.0)).border_1()
                            .border_color(if active { theme.accent } else { theme.border })
                            .bg(if active { theme.selection } else { theme.rail_bg }).cursor_pointer()
                            .when(keyboard_active, |el| el.border_2().border_color(theme.accent_strong))
                            .on_hover(cx.listener(move |this, hover, _, cx| {
                                if *hover { if let PopoverContent::Table { rows, columns, keyboard_cell } = &mut this.content {
                                    *rows = row; *columns = column; *keyboard_cell = None; cx.notify();
                                } }
                            }))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(EditorPopoverEvent::InsertTable { rows: row, columns: column }))));
                    }
                    grid = grid.child(grid_row);
                }
                self.shell("table-popover", &theme, animate_theme, cx).w(px(TABLE_POPOVER_WIDTH)).p(px(10.0))
                    .bg(theme.surface).border_color(theme.border)
                    .shadow(vec![BoxShadow { color: rgba(if theme.dark { 0x00000052 } else { 0x16202c14 }).into(), offset: point(px(0.0), px(if theme.dark { 16.0 } else { 14.0 })), blur_radius: px(if theme.dark { 44.0 } else { 34.0 }), spread_radius: px(0.0) }])
                    .child(div().flex().items_center().justify_between().min_w(px(214.0)).mb(px(8.0))
                        .text_size(px(13.0)).line_height(px(19.5)).font_weight(FontWeight::BOLD).text_color(theme.muted)
                        .child(locale.text("table_title")).child(div().font_family(font_for(Family::Mono, true, false).family).text_color(theme.accent_strong)
                            .child(format!("{preview_rows} × {preview_columns}"))))
                    .child(grid).children(self.error_text(&theme).map(|error| div().mt(px(7.0)).child(error))).into_any_element()
            }
        }
    }
}

fn link_pointer(bg: Hsla, border: Hsla) -> AnyElement {
    div().absolute().left(px(16.0)).top(px(-8.0)).w(px(14.0)).h(px(14.0)).child(
        gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let at = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
            let mut fill = gpui::PathBuilder::fill();
            fill.move_to(at(7.0, 0.0)); fill.line_to(at(14.0, 7.0)); fill.line_to(at(7.0, 14.0)); fill.line_to(at(0.0, 7.0)); fill.close();
            if let Ok(path) = fill.build() { window.paint_path(path, bg); }
            let mut stroke = gpui::PathBuilder::stroke(px(1.0));
            stroke.move_to(at(0.0, 7.0)); stroke.line_to(at(7.0, 0.0)); stroke.line_to(at(14.0, 7.0));
            if let Ok(path) = stroke.build() { window.paint_path(path, border); }
        }).size_full()
    ).into_any_element()
}

fn trash_icon() -> AnyElement {
    gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
        let at = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
        let mut path = gpui::PathBuilder::stroke(px(1.25));
        for points in [
            &[(2.0, 4.0), (13.0, 4.0)][..], &[(5.5, 4.0), (5.5, 2.0), (9.5, 2.0), (9.5, 4.0)][..],
            &[(3.5, 4.0), (4.0, 13.0), (11.0, 13.0), (11.5, 4.0)][..],
            &[(6.0, 6.0), (6.0, 11.0)][..], &[(9.0, 6.0), (9.0, 11.0)][..],
        ] {
            path.move_to(at(points[0].0, points[0].1));
            for (x, y) in &points[1..] { path.line_to(at(*x, *y)); }
        }
        if let Ok(path) = path.build() { window.paint_path(path, window.text_style().color); }
    }).size(px(15.0)).flex_none().into_any_element()
}
