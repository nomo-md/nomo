//! 对齐原版 ContextMenu.svelte 的层级、尺寸、焦点与窗口边缘定位。
use super::*;
use super::context_menu::Item;
use gpui::{Animation, AnimationExt, AnyElement, AvailableSpace, Div, Stateful, ScrollHandle};
use std::time::Duration;

pub(super) fn panel(theme: &Theme, id: &'static str) -> Stateful<Div> {
    div().id(id).min_w(px(210.0)).max_w(px(300.0)).p(px(4.0)).flex().flex_col()
        .rounded(px(8.0)).border_1().border_color(theme.border).bg(theme.surface).text_color(theme.fg)
        .text_size(px(13.0)).line_height(px(20.0)).font_family(crate::theme::ui_font_family())
        .shadow(vec![gpui::BoxShadow { inset: false, color: gpui::rgba(if theme.dark { 0x00000052 } else { 0x16202c14 }).into(),
            offset: point(px(0.0), px(if theme.dark { 16.0 } else { 14.0 })), blur_radius: px(if theme.dark { 44.0 } else { 34.0 }), spread_radius: px(0.0) }])
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

/// 右键菜单直接锚定鼠标坐标，子菜单与实际选中行相隔 4px；两者均按实测大小避让。
pub(super) fn positioned(
    id: &'static str, position: Point<Pixels>, row: Option<Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,
    panel: Stateful<Div>, reduced: bool,
) -> AnyElement {
    let nested = row.is_some();
    gpui::deferred(gpui::canvas(move |_, window, cx| {
        let viewport = window.viewport_size();
        let margin = px(8.0) + window.client_inset().unwrap_or(px(0.0));
        let max_height = (viewport.height - margin * 2.0).max(px(1.0));
        let panel = panel.max_h(if nested { max_height.min(px(520.0)) } else { max_height }).overflow_y_scroll();
        let mut panel = if reduced { panel.into_any_element() } else {
            panel.with_animation(id, Animation::new(Duration::from_millis(120)).with_easing(|t| 1.0 - (1.0 - t).powi(3)),
                |panel, t| panel.relative().top(px(-4.0 * (1.0 - t))).opacity(t)).into_any_element()
        };
        let measured = panel.layout_as_root(size(AvailableSpace::MaxContent, AvailableSpace::MaxContent), window, cx);
        let origin = if let Some(bounds) = row.and_then(|r| r.get()) {
            point(if bounds.right() + px(4.0) + measured.width > viewport.width - margin { bounds.left() - px(4.0) - measured.width }
                else { bounds.right() + px(4.0) }, bounds.top() - px(4.0))
        } else { position };
        let origin = point(origin.x.clamp(margin, (viewport.width - margin - measured.width).max(margin)).round(),
            origin.y.clamp(margin, (viewport.height - margin - measured.height).max(margin)).round());
        panel.prepaint_at(origin, window, cx); panel
    }, |_, mut panel: AnyElement, window, cx| window.with_content_mask(None, |window| panel.paint(window, cx)))
        .absolute().inset_0()).with_priority(if nested { 9 } else { 8 }).into_any_element()
}

impl NomoMark {
    pub(super) fn context_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.context_items(cx);
        let Some(menu) = &mut self.context_menu else { return; };
        menu.keyboard = true;
        let nested = menu.submenu.is_some() && menu.subselected.is_some();
        let entries = if nested { &items[menu.submenu.unwrap()].children } else { &items };
        let indexes: Vec<_> = entries.iter().enumerate().filter(|(_, i)| i.enabled && !i.separator).map(|(i, _)| i).collect();
        let selected = if nested { menu.subselected } else { menu.selected.or_else(|| indexes.first().copied()) };
        let next = match key {
            "down" | "up" if !indexes.is_empty() => {
                let current = selected.and_then(|s| indexes.iter().position(|i| *i == s));
                Some(indexes[match current { Some(i) if key == "down" => (i + 1) % indexes.len(),
                    Some(i) => (i + indexes.len() - 1) % indexes.len(), None if key == "down" => 0, None => indexes.len() - 1 }])
            },
            "home" => indexes.first().copied(), "end" => indexes.last().copied(),
            "escape" | "left" if menu.submenu.is_some() => { menu.submenu = None; menu.subselected = None; cx.notify(); return; },
            "escape" | "tab" => { self.dismiss_context_menu(window, cx); return; },
            "enter" | "space" | "right" => {
                if let Some(index) = selected.or_else(|| indexes.first().copied()) {
                    if key != "right" || !entries[index].children.is_empty() { self.run_context_item(index, nested, window, cx); }
                }
                return;
            }, _ => None,
        };
        if let Some(index) = next {
            if nested { menu.subselected = Some(index); menu.subscroll.scroll_to_item(index); }
            else { menu.selected = Some(index); menu.submenu = None; menu.subselected = None; menu.scroll.scroll_to_item(index); }
        }
        cx.notify();
    }

    fn context_panel(&self, items: &[Item], nested: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let menu = self.context_menu.as_ref().unwrap();
        let theme = Theme::ui(cx);
        let selected = if nested { menu.subselected } else { menu.selected.or_else(|| items.iter().position(|i| i.enabled && !i.separator)) };
        let scroll: &ScrollHandle = if nested { &menu.subscroll } else { &menu.scroll };
        let mut panel = panel(&theme, if nested { "editor-context-submenu" } else { "editor-context-menu" }).track_scroll(scroll);
        for (index, item) in items.iter().enumerate() {
            if item.separator { panel = panel.child(div().h(px(1.0)).my(px(4.0)).mx(px(7.0)).flex_none().bg(theme.border)); continue; }
            let active = selected == Some(index);
            let color = if item.danger { theme.danger } else { theme.fg };
            let background = if item.danger { theme.danger.opacity(0.09) } else { theme.hover };
            let opens = !item.children.is_empty() && item.enabled;
            let mut row = div().id((if nested { "editor-context-child" } else { "editor-context-root" }, index))
                .relative().min_h(px(32.0)).px(px(9.0)).py(px(5.0)).flex().flex_none().items_center().gap(px(8.0))
                .rounded(px(4.0)).text_color(color).whitespace_nowrap()
                .when(item.enabled, |el| el.cursor_pointer())
                .when(!item.enabled, |el| el.opacity(0.46).cursor(CursorStyle::Arrow))
                .when(item.checked || active, |el| el.bg(background))
                .when(active && menu.keyboard && item.enabled, |el| el.border_2().border_color(theme.accent.opacity(0.68)).px(px(7.0)).py(px(3.0)))
                .on_mouse_move(cx.listener(move |view, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some(menu) = &mut view.context_menu {
                        // 布局/键盘重绘也会重新命中鼠标；静止指针不能抢回键盘焦点或重开子菜单。
                        if menu.pointer == event.position { return; }
                        menu.pointer = event.position;
                        menu.keyboard = false;
                        if nested { menu.subselected = Some(index); }
                        else {
                            menu.selected = Some(index); menu.subselected = None;
                            let submenu = opens.then_some(index);
                            if menu.submenu != submenu { menu.submenu = submenu; menu.subscroll.set_offset(point(px(0.0), px(0.0))); }
                        }
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |view, _, window, cx| { cx.stop_propagation(); view.run_context_item(index, nested, window, cx); }));
            let glyph = if item.checked { "check" } else { item.icon };
            row = row.child(div().w(px(16.0)).h(px(16.0)).flex_none().flex().items_center().justify_center()
                .when(!glyph.is_empty(), |el| el.child(gpui::svg().path(format!("icons/{glyph}.svg")).size(px(15.0)).text_color(color))))
                .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(item.label.clone()));
            if !item.shortcut.is_empty() { row = row.child(div().pl(px(20.0)).flex_none().text_color(theme.muted).child(item.shortcut.clone())); }
            if !item.children.is_empty() { row = row.child(crate::ui::components::icon(crate::ui::components::IconName::ChevronRight, 14.0, theme.muted)); }
            if !nested && menu.submenu == Some(index) {
                let bounds = menu.row_bounds.clone();
                row = row.child(gpui::canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| ()).absolute().inset_0());
            }
            panel = panel.child(row);
        }
        panel
    }

    pub(super) fn context_menu_overlay(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        if menu.identity != self.host.parsed.identity || menu.selection != self.host.selection { return None; }
        let viewport = window.viewport_size();
        let items = self.context_items(cx);
        let root = positioned("editor-context-enter", menu.position, None, self.context_panel(&items, false, cx), crate::ui::popup::reduced_motion(cx));
        let nested = menu.submenu.and_then(|i| items.get(i)).filter(|item| !item.children.is_empty()).map(|item|
            positioned("editor-context-sub-enter", menu.position, Some(menu.row_bounds.clone()), self.context_panel(&item.children, true, cx), crate::ui::popup::reduced_motion(cx)));
        let shield = gpui::deferred(gpui::anchored().position(point(px(0.0), px(0.0))).child(
            div().id("editor-context-shield").w(viewport.width).h(viewport.height).cursor(CursorStyle::Arrow)
                // 外部点击仍交给命中的正文窗口；跨窗口右键可一次关闭旧菜单并打开新菜单。
                .on_mouse_down(MouseButton::Left, cx.listener(|v, _, w, cx| v.dismiss_context_menu(w, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|v, _, w, cx| v.dismiss_context_menu(w, cx)))
                .on_scroll_wheel(cx.listener(|v, _, w, cx| v.dismiss_context_menu(w, cx)))
        )).with_priority(7);
        // 焦点属于稳定的覆盖层。测量画布中的面板会被移到正文布局之外，不能由它持有键盘分发节点。
        Some(div().id("editor-context-overlay").absolute().inset_0().track_focus(&menu.focus).key_context("EditorContextMenu")
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                view.context_key(&event.keystroke.key, window, cx); cx.stop_propagation();
            }))
            .on_action(cx.listener(|v, _: &editor::MoveUp, w, cx| v.context_key("up", w, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveDown, w, cx| v.context_key("down", w, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveLeft, w, cx| v.context_key("left", w, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveRight, w, cx| v.context_key("right", w, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveLineStart, w, cx| v.context_key("home", w, cx)))
            .on_action(cx.listener(|v, _: &editor::MoveLineEnd, w, cx| v.context_key("end", w, cx)))
            .on_action(cx.listener(|v, _: &editor::Newline, w, cx| v.context_key("enter", w, cx)))
            .on_action(cx.listener(|v, _: &Dismiss, w, cx| v.context_key("escape", w, cx)))
            .on_action(cx.listener(|v, _: &editor::Indent, w, cx| v.context_key("tab", w, cx)))
            .child(shield).child(root).children(nested).into_any_element())
    }
}
