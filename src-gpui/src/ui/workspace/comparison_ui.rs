use super::*;
use std::collections::HashMap;

pub(in crate::ui::workspace) struct PaneFocus {
    keys: [u64; 2],
    handles: [FocusHandle; 2],
    _subscriptions: [Subscription; 2],
}
pub(super) struct TabMenu {
    item: TabItem,
    at: gpui::Point<gpui::Pixels>,
    page: MenuPage,
    focus: FocusHandle,
    selected: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum MenuPage {
    Main,
    Combine,
    Copy,
    Reveal,
}
#[derive(Clone, Copy)]
enum Choice {
    Page(MenuPage),
    Combine(u64),
    Swap,
    Split,
    Move,
    Close,
    Others,
    Right,
    All,
    Path(u64, bool),
}

pub(in crate::ui::workspace) struct TabRow {
    pub item: TabItem,
    pub tab: Option<usize>,
    pub title: String,
    pub tooltip: String,
    pub dirty: bool,
}

impl Workspace {
    pub(in crate::ui::workspace) fn tab_rows(&self, cx: &App) -> Vec<TabRow> {
        let indices: HashMap<_, _> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| (tab.key, index))
            .collect();
        self.comparison
            .items
            .iter()
            .map(|&item| {
                let mut titles = Vec::new();
                let mut paths = Vec::new();
                let mut dirty = false;
                for key in item.keys() {
                    if let Some(&index) = indices.get(&key) {
                        let doc = self.tabs[index].document.read(cx);
                        let title = doc.display_title(cx);
                        paths.push(
                            doc.path()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| title.clone()),
                        );
                        titles.push(title);
                        dirty |= doc.is_dirty(cx);
                    } else if let Some(saved) = self.comparison.layout.pending.get(&key) {
                        let title = saved.display_title(cx);
                        paths.push(
                            saved
                                .path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| title.clone()),
                        );
                        titles.push(title);
                        dirty |= saved.dirty;
                    }
                }
                TabRow {
                    item,
                    tab: indices.get(&item.first()).copied(),
                    title: titles.join("｜"),
                    tooltip: paths.join("\n"),
                    dirty,
                }
            })
            .collect()
    }
    pub(in crate::ui::workspace) fn saved_tab_button(
        &self,
        item: TabItem,
        title: String,
        index: usize,
        marker: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("saved-tab", index))
            .relative()
            .px_3()
            .h(px(30.))
            .max_w(px(180.))
            .truncate()
            .cursor_pointer()
            .child(title)
            .children(marker)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |w, e: &gpui::MouseDownEvent, win, cx| {
                    w.open_tab_context(item, e.position, win, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |w, e: &gpui::MouseDownEvent, win, cx| {
                    w.arm_tab_drag(item, None, e.position, win, cx)
                }),
            )
            .on_click(cx.listener(move |w, _, win, cx| {
                if !w.drag_suppresses_click() {
                    w.activate_item(item, win, cx);
                }
            }))
            .into_any_element()
    }
    pub(in crate::ui::workspace) fn render_comparison(
        &mut self,
        pair: Pair,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let keys = [pair.left, pair.right];
        if self
            .comparison
            .pane_focus
            .as_ref()
            .is_none_or(|focus| focus.keys != keys)
        {
            let handles = [cx.focus_handle(), cx.focus_handle()];
            let subscriptions = std::array::from_fn(|index| {
                let key = keys[index];
                cx.on_focus_in(&handles[index], window, move |w, window, cx| {
                    if w.active_pair()
                        .is_some_and(|p| p.contains_key(key) && p.focused != key)
                    {
                        let child = window.focused(cx);
                        w.activate_key(key, window, cx);
                        if w.tab().is_some_and(|tab| tab.key == key) {
                            if let Some(child) = child {
                                window.focus(&child, cx);
                            }
                        } else if let Some(doc) = w.document() {
                            window.focus(&doc.focus_handle(cx), cx);
                        }
                    }
                })
            });
            self.comparison.pane_focus = Some(PaneFocus {
                keys,
                handles,
                _subscriptions: subscriptions,
            });
        }
        let handles = self.comparison.pane_focus.as_ref().unwrap().handles.clone();
        let mode = self
            .tab()
            .map(|tab| tab.mode)
            .unwrap_or(EditorMode::Semantic);
        let focus = self.comparison.divider_focus.clone();
        let mut panes = Vec::with_capacity(2);
        for (side, key) in keys.into_iter().enumerate() {
            let Some(document) = self.host_for(key) else {
                continue;
            };
            document.update(cx, |host, _| {
                host.set_save_directory(self.tree.as_ref().map(|tree| tree.root.as_path()))
            });
            let focused = key == pair.focused;
            let title = document.read(cx).display_title(cx);
            document.update(cx, |host, cx| host.set_workspace_focus(focused, cx));
            let mut body = div().relative().flex_1().min_h_0().child(document);
            if focused {
                let measured = self.outline_ui.bounds.clone();
                let weak = cx.entity().downgrade();
                body = body.child(
                    gpui::canvas(
                        move |bounds, win, _| {
                            if measured.replace(Some(bounds)) != Some(bounds) {
                                let weak = weak.clone();
                                win.on_next_frame(move |_, cx| {
                                    let _ = weak.update(cx, |_, cx| cx.notify());
                                });
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                );
                body = body.children(self.render_outline_panel(theme, mode, window, cx));
            }
            let header = div()
                .id(("comparison-member", key))
                .h(px(29.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .bg(if focused { theme.hover } else { theme.surface })
                .border_b_1()
                .border_color(theme.border)
                .text_size(px(12.))
                .text_color(if focused { theme.heading } else { theme.muted })
                .cursor_grab()
                .child("⠿")
                .child(icon(IconName::FileText, 13., theme.muted))
                .child(div().truncate().child(title))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |w, event: &gpui::MouseDownEvent, win, cx| {
                        w.arm_tab_drag(TabItem::Pair(pair), Some(key), event.position, win, cx);
                    }),
                );
            let pane = div()
                .id(("comparison-pane", key))
                .track_focus(&handles[side])
                .flex()
                .flex_col()
                .h_full()
                .min_w_0()
                .min_h_0()
                .when(key == pair.left, |el| el.w(gpui::relative(pair.ratio)))
                .when(key != pair.left, |el| el.flex_1())
                .capture_any_mouse_down(cx.listener(move |w, _, win, cx| {
                    if w.tab().is_some_and(|tab| tab.key != key) {
                        w.activate_key(key, win, cx);
                        if w.tab().is_none_or(|tab| tab.key != key) {
                            cx.stop_propagation();
                        }
                    }
                }))
                .child(header)
                .child(body);
            panes.push(pane.into_any_element());
        }
        let bounds = self.comparison.bounds.clone();
        let divider = div()
            .id("comparison-divider")
            .w(px(8.))
            .h_full()
            .flex_none()
            .bg(theme.border)
            .hover(|s| s.bg(theme.accent))
            .cursor_col_resize()
            .track_focus(&focus)
            .tab_index(0)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|w, _, win, cx| {
                    w.comparison.resizing = true;
                    win.focus(&w.comparison.divider_focus, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|w, e: &gpui::KeyDownEvent, _, cx| {
                let Some(pair) = w.active_pair() else {
                    return;
                };
                let step = if e.keystroke.modifiers.shift {
                    0.10
                } else {
                    0.02
                };
                let ratio = match e.keystroke.key.as_str() {
                    "left" => pair.ratio - step,
                    "right" => pair.ratio + step,
                    "home" => 0.20,
                    "end" => 0.80,
                    _ => return,
                };
                w.resize_comparison(pair.focused, ratio, true, cx);
                cx.stop_propagation();
            }));
        let mut panes = panes.into_iter();
        div()
            .relative()
            .size_full()
            .min_w_0()
            .flex()
            .child(
                gpui::canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(panes.next())
            .child(divider)
            .children(panes.next())
            .into_any_element()
    }
    fn resize_comparison(&mut self, key: u64, ratio: f32, persist: bool, cx: &mut Context<Self>) {
        if let Some(TabItem::Pair(pair)) = self.comparison.item_mut(key) {
            pair.ratio = ratio.clamp(0.2, 0.8);
            if persist {
                self.record_layout(cx);
            }
            cx.notify();
        }
    }
    pub(in crate::ui::workspace) fn comparison_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.comparison.resizing {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_comparison_resize(cx);
            return;
        }
        if let (Some(pair), Some(bounds)) = (self.active_pair(), self.comparison.bounds.get()) {
            let width = f32::from(bounds.size.width).max(1.);
            self.resize_comparison(
                pair.focused,
                f32::from(event.position.x - bounds.left()) / width,
                false,
                cx,
            );
        }
    }
    pub(in crate::ui::workspace) fn finish_comparison_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.comparison.resizing) {
            self.record_layout(cx);
            cx.notify();
        }
    }
    pub(in crate::ui::workspace) fn open_tab_context(
        &mut self,
        item: TabItem,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_interaction_busy(window) {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.comparison.menu = Some(TabMenu {
            item,
            at,
            page: MenuPage::Main,
            focus,
            selected: 0,
        });
        cx.stop_propagation();
        cx.notify();
    }
    fn tab_choices(&self, menu: &TabMenu, cx: &App) -> Vec<(String, Choice, bool)> {
        use Choice::*;
        let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
        let item = menu.item;
        let key = item.focused();
        match menu.page {
            MenuPage::Main => {
                let mut choices = Vec::new();
                if item.pair().is_some() {
                    choices.push((locale.text("tab_swap").into(), Swap, true));
                    choices.push((locale.text("tab_ungroup").into(), Split, true));
                } else if self.markdown_tab(key, cx) {
                    choices.push((
                        format!("{}  ›", locale.text("tab_combine")),
                        Page(MenuPage::Combine),
                        self.comparison
                            .items
                            .iter()
                            .any(|i| self.can_combine(key, i.first(), cx)),
                    ));
                }
                if self.markdown_tab(key, cx) {
                    choices.push((locale.text("tab_move_window").into(), Move, true));
                }
                choices.push((locale.text("tab_close").into(), Close, true));
                choices.push((
                    locale.text("tab_close_others").into(),
                    Others,
                    self.comparison.items.len() > 1,
                ));
                choices.push((
                    locale.text("tab_close_right").into(),
                    Right,
                    self.comparison
                        .position(key)
                        .is_some_and(|p| p + 1 < self.comparison.items.len()),
                ));
                choices.push((locale.text("tab_close_all").into(), All, true));
                if item.keys().iter().any(|k| {
                    self.host_for(*k)
                        .is_some_and(|doc| doc.read(cx).path().is_some())
                }) {
                    choices.push((
                        locale.text("tab_copy_path").into(),
                        if item.pair().is_some() {
                            Page(MenuPage::Copy)
                        } else {
                            Path(key, false)
                        },
                        true,
                    ));
                    choices.push((
                        locale.text("tab_reveal").into(),
                        if item.pair().is_some() {
                            Page(MenuPage::Reveal)
                        } else {
                            Path(key, true)
                        },
                        true,
                    ));
                }
                choices
            }
            MenuPage::Combine => self
                .comparison
                .items
                .iter()
                .filter_map(|item| {
                    let other = item.first();
                    self.can_combine(key, other, cx).then(|| {
                        (
                            self.host_for(other)
                                .map(|doc| doc.read(cx).display_title(cx))
                                .or_else(|| {
                                    self.comparison
                                        .layout
                                        .pending
                                        .get(&other)
                                        .map(|doc| doc.display_title(cx))
                                })
                                .unwrap_or_default(),
                            Combine(other),
                            true,
                        )
                    })
                })
                .collect(),
            page => item
                .keys()
                .into_iter()
                .filter_map(|k| {
                    let doc = self.host_for(k)?;
                    doc.read(cx).path().map(|_| {
                        (
                            doc.read(cx).display_title(cx),
                            Path(k, page == MenuPage::Reveal),
                            true,
                        )
                    })
                })
                .collect(),
        }
    }
    fn choose_tab_action(&mut self, action: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.comparison.menu.as_mut() else {
            return;
        };
        if let Choice::Page(page) = action {
            menu.page = page;
            menu.selected = 0;
            cx.notify();
            return;
        }
        let item = menu.item;
        self.dismiss_tab_menu(window, cx);
        if self.comparison.item(item.first()) != Some(item) {
            return;
        }
        match action {
            Choice::Combine(k) => self.group_tabs(item.first(), k, window, cx),
            Choice::Swap | Choice::Split => {
                self.split_or_swap(item.focused(), matches!(action, Choice::Swap), window, cx)
            }
            Choice::Move => self.transfer_item(item, None, None, drag::Placement::End, window, cx),
            Choice::Path(key, reveal) => {
                if let Some(path) = self.host_for(key).and_then(|doc| doc.read(cx).path()) {
                    if reveal {
                        cx.reveal_path(&path);
                    } else {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                            path.to_string_lossy().into_owned(),
                        ));
                    }
                }
            }
            Choice::Close => self.close_item(item, window, cx),
            Choice::Others | Choice::Right | Choice::All => {
                let pos = self.comparison.position(item.first()).unwrap_or(0);
                self.comparison.batch = self
                    .comparison
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| match action {
                        Choice::Others => *i != pos,
                        Choice::Right => *i > pos,
                        _ => true,
                    })
                    .map(|(_, item)| item.first())
                    .collect();
                self.next_batch_close(window, cx);
            }
            Choice::Page(_) => {}
        }
    }
    fn dismiss_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comparison.menu.take().is_some() {
            if let Some(doc) = self.document() {
                window.focus(&doc.focus_handle(cx), cx);
            }
            cx.notify();
        }
    }
    pub(in crate::ui::workspace) fn render_tab_context(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.comparison.menu.as_ref()?;
        let choices = self.tab_choices(menu, cx);
        let at = menu.at;
        let selected = menu.selected;
        let page = menu.page;
        let panel = div()
            .id("comparison-tab-context")
            .track_focus(&menu.focus)
            .w(px(218.))
            .max_h(px(360.))
            .p_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .occlude()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|w, _, win, cx| w.dismiss_tab_menu(win, cx)))
            .on_key_down(cx.listener(|w, e: &gpui::KeyDownEvent, win, cx| {
                let Some(menu) = w.comparison.menu.as_ref() else {
                    return;
                };
                let choices = w.tab_choices(menu, cx);
                match e.keystroke.key.as_str() {
                    "escape" => w.dismiss_tab_menu(win, cx),
                    "left" => {
                        w.choose_tab_action(Choice::Page(MenuPage::Main), win, cx);
                    }
                    "enter" | "right" => {
                        if let Some((_, action, true)) = choices.get(menu.selected) {
                            w.choose_tab_action(*action, win, cx);
                        }
                    }
                    "up" | "down" if !choices.is_empty() => {
                        let menu = w.comparison.menu.as_mut().unwrap();
                        menu.selected = (menu.selected
                            + if e.keystroke.key == "up" {
                                choices.len() - 1
                            } else {
                                1
                            })
                            % choices.len();
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .when(page != MenuPage::Main, |p| {
                p.child(
                    div()
                        .id("comparison-menu-back")
                        .p_1()
                        .child(format!(
                            "‹ {}",
                            crate::ui::toolbar_i18n::ToolbarLocale::current(cx)
                                .text("tab_menu_back")
                        ))
                        .cursor_pointer()
                        .on_click(cx.listener(|w, _, win, cx| {
                            w.choose_tab_action(Choice::Page(MenuPage::Main), win, cx)
                        })),
                )
            })
            .children(
                choices
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, action, enabled))| {
                        div()
                            .id(("comparison-action", i))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .truncate()
                            .text_size(px(13.))
                            .when(i == selected, |row| row.bg(theme.hover))
                            .when(!enabled, |row| row.opacity(0.45))
                            .when(enabled, |row| {
                                row.cursor_pointer().hover(|s| s.bg(theme.hover)).on_click(
                                    cx.listener(move |w, _, win, cx| {
                                        w.choose_tab_action(action, win, cx)
                                    }),
                                )
                            })
                            .child(label)
                    }),
            );
        Some(
            gpui::deferred(popup(
                "comparison-menu-popup",
                PopupAnchor::Bounds(gpui::Bounds::new(at, gpui::size(px(0.), px(0.)))),
                PopupPlacement::BelowLeft,
                panel,
                self.explorer.reduce_motion,
            ))
            .with_priority(6)
            .into_any_element(),
        )
    }
}
