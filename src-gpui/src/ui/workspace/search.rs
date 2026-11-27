//! 当前文档的查找与替换面板。窗口只保留最后的搜索词及手动位置。
use super::*;
use gpui::{Bounds, Pixels, Point, point};

pub(crate) struct SearchPanel {
    document: gpui::WeakEntity<DocumentHost>,
    pub(crate) query: Entity<TextField>,
    pub(crate) replacement: Entity<TextField>,
    pub(super) show_replace: bool,
    options: bool,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    size: gpui::Size<Pixels>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    case_sensitive: bool,
    whole_word: bool,
    backwards: bool,
    wrap_around: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub(crate) fn open_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.popover=None;
        let Some(buffer) = self.document() else { return };
        if replace && buffer.read(cx).editing_locked() { return; }
        let selected = if self.search.is_none() { buffer.read(cx).selected_text(cx) } else { String::new() };
        let was_open = self.search.is_some();
        if self.search.is_none() {
            buffer.update(cx, |host, cx| host.begin_find(cx));
            let locale = crate::ui::toolbar_i18n::ToolbarLocale::current(cx);
            let query = cx.new(|cx| { let mut field = TextField::new(locale.text("find"), cx); field.set_tab_stop(); field });
            let replacement = cx.new(|cx| { let mut field = TextField::new(locale.text("replace_with"), cx); field.set_tab_stop(); field });
            let subscriptions = vec![
                cx.subscribe_in(&query, window, |workspace, _, event, window, cx| workspace.on_query_event(event, window, cx)),
                cx.subscribe_in(&replacement, window, |workspace, _, event, window, cx| match event {
                    FieldEvent::Submit { .. } => workspace.replace_one(window, cx),
                    FieldEvent::Cancel => workspace.close_search(window, cx),
                    FieldEvent::Changed => cx.notify()
                }),
            ];
            self.search = Some(SearchPanel { document: buffer.downgrade(), query, replacement, options: false, drag: None, size: Default::default(), bounds: Default::default(), show_replace: replace, case_sensitive: false, whole_word: false, backwards: false, wrap_around: true, _subscriptions: subscriptions });
        }
        let search = self.search.as_mut().expect("created");
        search.show_replace = replace;
        let query = search.query.clone();
        if !was_open {
            let text = if !selected.is_empty() && !selected.contains(['\r', '\n']) && selected.len() < 200 { &selected } else { &self.last_search };
            query.update(cx, |field, cx| field.set_text(text, cx));
        } else {
            query.update(cx, |field, cx| field.select_all_text(cx));
        }
        window.focus(&query.focus_handle(cx), cx);
        cx.notify();
    }

    fn on_query_event(&mut self, event: &FieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            FieldEvent::Changed => self.run_search(cx),
            FieldEvent::Submit { backwards } => { let reverse=self.search.as_ref().is_some_and(|search| search.backwards); self.step_match(!(*backwards ^ reverse), window, cx); },
            FieldEvent::Cancel => self.close_search(window, cx),
        }
    }

    pub(super) fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &self.search else { return };
        let Some(buffer) = self.search_target() else { return };
        let query = search.query.read(cx).content.clone();
        let case = search.case_sensitive;
        let whole_word = search.whole_word;
        buffer.update(cx, |buffer, cx| buffer.set_search(query, case, whole_word, cx));
    }

    fn search_target(&self) -> Option<Entity<DocumentHost>> {
        let target = self.search.as_ref()?.document.upgrade()?;
        (self.document().as_ref() == Some(&target)).then_some(target)
    }

    pub(crate) fn step_match(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(panel), Some(document)) = (&self.search, self.search_target()) {
            document.update(cx, |host, cx| host.step_match(forward, panel.wrap_around, cx));
        }
    }
    pub(super) fn replace_one(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(search),Some(document))=(&self.search,self.search_target()) else {return;};
        let replacement=search.replacement.read(cx).content.clone();
        let wrap = search.wrap_around;
        document.update(cx,|host,cx|host.replace_current(&replacement,wrap,cx));
    }
    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let (Some(search),Some(document))=(&self.search,self.search_target()) else {return;};
        let replacement=search.replacement.read(cx).content.clone();
        document.update(cx,|host,cx|host.replace_all(replacement,cx));
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.search_target().map(|doc| doc.update(cx, |host, cx| host.focus_after_find(cx)));
        self.dismiss_search(cx);
        if let Some(focus) = focus { window.focus(&focus, cx); }
        cx.notify();
    }

    pub(super) fn dismiss_search(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.search.take() else { return; };
        let query = &panel.query.read(cx).content;
        // 单个窗口只记一条可复用的查询，长粘贴不会在关闭后常驻。
        self.last_search = if query.len() <= 8192 { query.clone() } else { String::new() };
        if let Some(doc) = panel.document.upgrade() { doc.update(cx, |host, cx| host.end_find(cx)); }
        cx.notify();
    }

    fn drag_search_to(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(panel) = self.search.as_ref() else { return };
        let (Some((start, origin)), Some(area)) = (panel.drag, self.outline_ui.bounds.get()) else { return };
        let p = origin + (pointer - start);
        self.search_position = Some(point(
            p.x.clamp(px(0.), (area.size.width - panel.size.width).max(px(0.))),
            p.y.clamp(px(0.), (area.size.height - panel.size.height).max(px(0.))),
        ));
        cx.notify();
    }

    fn focus_search_on_mouse_down(&mut self, _: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &self.search {
            let query = panel.query.focus_handle(cx);
            if !query.is_focused(window) && !panel.replacement.focus_handle(cx).is_focused(window) {
                window.focus(&query, cx);
            }
        }
        cx.stop_propagation();
    }

    pub(super) fn render_search(&self, outline: Option<Bounds<Pixels>>, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self.search.as_ref()?;
        let doc = self.search_target()?;
        let host = doc.read(cx);
        let state = host.search();
        let locale = ToolbarLocale::current(cx);
        let source_hint = host.search_source_hint(cx);
        let editable = !host.editing_locked();
        let expanded = editable && panel.show_replace;
        let area = self.outline_ui.bounds.get()?;
        let width = (area.size.width - px(12.)).clamp(px(0.), px(480.));
        let height = panel.size.height.max(px(44.)).min(area.size.height);
        let right = (area.size.width - width - px(12.)).max(px(0.));
        let top = px(10.);
        let default = Bounds::new(area.origin + point(right, top), gpui::size(width, height));
        let mut pos = point(right, top);
        if let Some(obstacle) = outline.filter(|r| !r.intersect(&default).is_empty()) {
            let left = obstacle.left() - area.left() - width - px(8.);
            if left >= px(0.) { pos.x = left; }
            else if obstacle.bottom() - area.top() + height + px(8.) <= area.size.height { pos.y = obstacle.bottom() - area.top() + px(8.); }
        }
        pos = self.search_position.unwrap_or(pos);
        pos.x = pos.x.clamp(px(0.), (area.size.width - width).max(px(0.)));
        let panel_left = area.left() + pos.x;
        let safe_height = self.status_ui.controls_bounds(cx.global::<Settings>())
            .filter(|r| !self.mini.active() && r.left() < panel_left + width && panel_left < r.right())
            .map_or(area.size.height, |r| (r.top() - area.top() - px(6.)).clamp(px(0.), area.size.height));
        pos.y = pos.y.clamp(px(0.), safe_height);

        let count = if state.query.is_empty() { String::new() } else if state.pending { "…".into() }
            else if state.total == 0 { locale.text("no_results").into() }
            else if let Some(current) = state.current { format!("{}/{}", current + 1, state.total) }
            else { locale.text("search_count").replace("{}", &state.total.to_string()) };
        let mut feedback = state.error.clone().or_else(|| state.feedback.clone());
        if let Some(hint) = source_hint { feedback = Some(locale.text(hint).into()); }
        let can_one = state.can_replace();
        let can_all = state.can_replace_all();
        let deletion = panel.replacement.read(cx).content.is_empty();
        let total = state.total;
        let header_button = |id, name, active| {
            small_icon_button(id, name, active, theme)
                .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_search_on_mouse_down))
        };
        let nav = |forward, id, name, key| {
            let enabled = state.can_step(forward, panel.wrap_around);
            header_button(id, name, false).tooltip(tooltip(format!("{}  {}", locale.text(key),
                crate::ui::shortcuts::current(if forward { "find-next" } else { "find-previous" }, cx))))
                .when(!enabled, |el| el.opacity(0.38).cursor_default())
                .when(enabled, |el| el.on_click(cx.listener(move |w, _, win, cx| w.step_match(forward, win, cx))))
        };
        let prev = nav(false, "prev", IconName::ArrowUp, "previous");
        let next = nav(true, "next", IconName::ArrowDown, "next");
        let grip = div().id("search-grip").w(px(18.)).h(px(26.)).flex_none().flex().items_center().justify_center()
            .child(div().flex().gap(px(2.)).children((0..2).map(|_| div().flex().flex_col().gap(px(2.))
                .children((0..3).map(|_| div().size(px(2.5)).rounded_full().bg(theme.muted))))))
            .tooltip(tooltip(locale.text("search_move")));
        let input = div().flex().flex_1().min_w_0().items_center().gap(px(4.)).child(grip)
            .when(editable, |el| el.child(header_button("toggle-replace",
                if expanded { IconName::ChevronDown } else { IconName::ChevronRight }, false)
                .tooltip(tooltip(locale.text("replace"))).on_click(cx.listener(|w, _, window, cx| {
                    if let Some(panel) = &mut w.search {
                        panel.show_replace = !panel.show_replace;
                        if !panel.show_replace && panel.replacement.focus_handle(cx).is_focused(window) { window.focus(&panel.query.focus_handle(cx), cx); }
                    }
                    cx.notify();
                }))))
            .child(div().flex().flex_1().min_w_0().child(panel.query.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()));
        let controls = div().flex().flex_none().items_center().justify_end().gap(px(4.))
            .child(header_button("case", IconName::CaseSensitive, panel.case_sensitive).tooltip(tooltip(locale.text("match_case")))
                .on_click(cx.listener(|w, _, _, cx| { if let Some(s) = &mut w.search { s.case_sensitive = !s.case_sensitive; } w.run_search(cx); cx.notify(); })))
            .child(div().id("search-count").w(px(if width < px(330.) {42.} else {62.}))
                .truncate().text_center().text_size(px(12.)).text_color(theme.muted)
                .when(!count.is_empty(), |el| el.child(count.clone()).tooltip(tooltip(count))))
            .child(prev).child(next)
            .child(header_button("search-options", IconName::SlidersHorizontal, panel.options || panel.whole_word || panel.backwards || !panel.wrap_around)
                .tooltip(tooltip(locale.text("search_options"))).on_click(cx.listener(|w, _, _, cx| {
                    if let Some(s) = &mut w.search { s.options = !s.options; } cx.notify();
                })))
            .child(header_button("close-search", IconName::Close, false).tooltip(tooltip(format!("{}  Esc", locale.text("close"))))
                .on_click(cx.listener(|w, _, win, cx| w.close_search(win, cx))));
        let compact = width < px(320.);
        // 顶部留白属于拖动区域，和卡片另外三边共用同一边距。
        let inset = px(12.);
        let row = div().flex().flex_none().px(inset).pt(inset).gap(px(4.)).cursor_grab()
            .when(compact, |el| el.flex_col()).when(!compact, |el| el.items_center())
            .on_mouse_down(MouseButton::Left, cx.listener(move |w, event: &gpui::MouseDownEvent, _, cx| {
                if let Some(panel) = &mut w.search {
                    let origin = panel.bounds.get().map_or(pos, |r| r.origin - area.origin);
                    panel.drag = Some((event.position, origin));
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(input).child(controls);
        let mut card = floating_card("search-panel", theme).relative().flex_none().occlude().w_full()
            .max_h(safe_height).overflow_y_scroll().pb(inset).flex().flex_col().gap(px(6.))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_search_on_mouse_down)).child(row);
        if expanded {
            let kind = |enabled| if enabled { ButtonKind::Secondary } else { ButtonKind::Disabled };
            let one = button("replace-one", locale.text("replace_current"), None, kind(can_one), theme).h(px(26.)).text_size(px(12.)).px(px(6.))
                .tooltip(tooltip(locale.text(if deletion { "search_delete_one" } else { "replace_current" })))
                .when(can_one, |el| el.on_click(cx.listener(|w, _, win, cx| w.replace_one(win, cx))));
            let all = button("replace-all", locale.text("replace_all"), None, kind(can_all), theme).h(px(26.)).text_size(px(12.)).px(px(6.))
                .tooltip(tooltip(locale.text(if deletion { "search_delete_all" } else { "search_replace_all" }).replace("{}", &total.to_string())))
                .when(can_all, |el| el.on_click(cx.listener(|w, _, _, cx| w.replace_all(cx))));
            let buttons = div().flex().items_center().gap(px(4.)).child(one).child(all);
            let row = div().flex().flex_none().mx(inset).gap(px(4.)).when(width < px(330.), |el| el.flex_col()).child(panel.replacement.clone()).child(buttons);
            card = card.child(row);
        }
        if panel.options {
            let mut options = div().flex().flex_none().mx(inset).flex_wrap().items_center().gap(px(4.));
            for (key, active) in [("whole_word", panel.whole_word), ("reverse", panel.backwards), ("wrap", panel.wrap_around)] {
                options = options.child(button(key, locale.text(key), active.then_some(IconName::Check), ButtonKind::Ghost, theme)
                    .text_size(px(12.)).px(px(6.)).when(active, |el| el.bg(theme.sidebar_active)).on_click(cx.listener(move |w, _, _, cx| {
                        if let Some(s) = &mut w.search {
                            match key { "whole_word" => s.whole_word = !s.whole_word, "reverse" => s.backwards = !s.backwards, _ => s.wrap_around = !s.wrap_around }
                        }
                        if key == "whole_word" { w.run_search(cx); }
                        cx.notify();
                    })));
            }
            card = card.child(options.child(small_icon_button("search-reset", IconName::RotateCcw, false, theme)
                .tooltip(tooltip(locale.text("search_reset"))).on_click(cx.listener(|w, _, _, cx| {
                    w.search_position = None; if let Some(s) = &mut w.search { s.options = false; } cx.notify();
                }))));
        } else if panel.whole_word || panel.backwards {
            let flags = [(panel.whole_word,"whole_word"),(panel.backwards,"reverse")].into_iter()
                .filter_map(|(on,key)| on.then(|| locale.text(key))).collect::<Vec<_>>().join(" · ");
            card = card.child(div().flex_none().mx(inset).text_size(px(11.)).text_color(theme.muted).child(flags));
        }
        if let Some(feedback) = feedback {
            card = card.child(div().flex().flex_none().mx(inset).items_center().gap(px(6.)).text_size(px(12.)).text_color(theme.muted).child(feedback)
                .when(source_hint.is_some(), |el| el.child(button("search-source", locale.text("search_view_source"), None, ButtonKind::Ghost, theme)
                    .on_click(cx.listener(|w, _, win, cx| w.set_mode(EditorMode::Source, win, cx))))));
        }
        panel.query.update(cx, |field, cx| field.set_placeholder(locale.text("find"), cx));
        panel.replacement.update(cx, |field, cx| field.set_placeholder(locale.text("replace_with"), cx));
        // 让上方留白先收缩，内容按本帧真实高度布局。展开替换或选项时仍在可见范围内。
        let last = panel.bounds.clone();
        let weak = cx.entity().downgrade();
        Some(div().absolute().top_0().left(pos.x).w(width).h(safe_height).flex().flex_col()
            .on_children_prepainted(move |children, window, _| {
                let Some(bounds) = children.last().copied() else { return };
                if last.replace(Some(bounds)) == Some(bounds) { return; }
                let weak = weak.clone();
                let last = last.clone();
                window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |w, cx| {
                    let Some(panel) = &mut w.search else { return };
                    if !Rc::ptr_eq(&panel.bounds, &last) { return; }
                    if panel.size != bounds.size { panel.size = bounds.size; cx.notify(); }
                    if let Some(doc) = panel.document.upgrade() { doc.update(cx, |host, _| host.set_search_panel_bounds(bounds)); }
                }); });
            })
            .child(div().h(pos.y).min_h_0())
            .child(card.child(self.search_drag_events(panel, cx))).into_any_element())
    }

    fn search_drag_events(&self, panel: &SearchPanel, cx: &Context<Self>) -> impl IntoElement + use<> {
        let held = panel.drag.is_some();
        let weak = cx.entity().downgrade();
        gpui::canvas(|_, _, _| (), move |_, _, window, _| {
            if held { window.set_window_cursor_style(gpui::CursorStyle::ClosedHand); }
            let moving = weak.clone();
            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _, cx| {
                if phase != gpui::DispatchPhase::Capture { return; }
                if let Some(w) = moving.upgrade().filter(|w| w.read(cx).search.as_ref().is_some_and(|s| s.drag.is_some())) {
                    w.update(cx, |w, cx| {
                        if e.pressed_button == Some(MouseButton::Left) { w.drag_search_to(e.position, cx); }
                        else if let Some(s) = &mut w.search { s.drag = None; cx.notify(); }
                    });
                    cx.stop_propagation();
                }
            });
            let release = weak.clone();
            window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _, cx| {
                if phase != gpui::DispatchPhase::Capture || e.button != MouseButton::Left { return; }
                if let Some(w) = release.upgrade().filter(|w| w.read(cx).search.as_ref().is_some_and(|s| s.drag.is_some())) {
                    w.update(cx, |w, cx| { w.drag_search_to(e.position, cx); if let Some(s) = &mut w.search { s.drag = None; } cx.notify(); });
                    cx.stop_propagation();
                }
            });
        }).absolute().inset_0().size_full()
    }
}
