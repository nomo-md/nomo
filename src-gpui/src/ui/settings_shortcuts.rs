//! 轻量命令列表；同一时刻只挂载一个编辑／帮助浮层，草稿离开浮层即释放。
use super::*;
use crate::ui::shortcuts::{self, SHORTCUTS};
use crate::ui::popup::{PopupAnchor, PopupPlacement, popup, reduced_motion};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ShortcutPopup { Editor(&'static str), Menu, Reference }
pub(super) struct Recording { id: &'static str, slot: usize, draft: Option<String> }

// GPUI 的可聚焦点击控件共用鼠标、Enter 和空格激活路径。
fn shortcut_button(
    id: impl Into<gpui::ElementId>, label: impl Into<gpui::SharedString>, kind: ButtonKind, theme: &Theme,
    cx: &mut Context<SettingsWindow>, action: impl Fn(&mut SettingsWindow, &mut Window, &mut Context<SettingsWindow>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, None, kind, theme).tab_index(0).flex_none()
        .focus(|style| style.bg(theme.hover).border_1().border_color(theme.accent))
        .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); action(this, window, cx); }))
}

fn keycap(value: &str, theme: &Theme) -> gpui::Div {
    div().px(px(7.0)).h(px(24.0)).flex().items_center().flex_none()
        .rounded(px(5.0)).border_1().border_color(theme.border.opacity(0.8))
        .bg(theme.app_bg).text_size(px(12.0)).text_color(theme.fg).whitespace_nowrap()
        .child(shortcuts::display(value))
}

impl SettingsWindow {
    fn shortcut_search(&mut self, cx: &mut Context<Self>) -> Entity<TextField> {
        const KEY: &str = "shortcut-search";
        if let Some(field) = self.fields.get(KEY) { return field.clone(); }
        let field = cx.new(|cx| {
            let mut field = TextField::new(ToolbarLocale::current(cx).app_text("shortcuts.search"), cx);
            field.set_settings_style(); field.set_tab_stop(); field
        });
        self._subscriptions.push(cx.subscribe(&field, |this, _, event, cx| {
            if matches!(event, FieldEvent::Changed) {
                this.shortcut_popup = None; this.shortcut_recording = None;
                this.shortcut_return_focus = None; this.shortcut_errors.clear();
                this.scroll.set_offset(point(px(0.0), px(0.0))); cx.notify();
            }
        }));
        self.fields.insert(KEY, field.clone()); field
    }
    fn open_shortcut_popup(&mut self, popup: ShortcutPopup, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_return_focus = window.focused(cx);
        self.shortcut_popup = Some(popup); self.shortcut_recording = None;
        self.shortcut_errors.clear(); self.shortcut_reset_confirm = false;
        window.focus(&self.shortcut_popup_focus, cx); cx.notify();
    }
    pub(super) fn close_shortcut_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_popup = None; self.shortcut_recording = None;
        self.shortcut_errors.clear(); self.shortcut_reset_confirm = false;
        window.focus(&self.shortcut_return_focus.take().unwrap_or_else(|| self.focus.clone()), cx);
        cx.notify();
    }
    fn start_recording(&mut self, id: &'static str, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_recording = Some(Recording { id, slot, draft: None });
        self.shortcut_errors.remove(id);
        window.focus(&self.shortcut_popup_focus, cx); cx.notify();
    }
    fn apply_recording(&mut self, cx: &mut Context<Self>) {
        let Some(recording) = &self.shortcut_recording else { return; };
        let Some(draft) = &recording.draft else { return; };
        let (id, slot) = (recording.id, recording.slot);
        let mut values = shortcuts::current_keys(id, cx);
        if slot < values.len() { values[slot] = draft.clone(); } else { values.push(draft.clone()); }
        self.save_shortcut(id, values, cx);
    }
    fn save_shortcut(&mut self, id: &'static str, values: Vec<String>, cx: &mut Context<Self>) {
        match shortcuts::validate_keys(id, &values, cx) {
            Ok(values) => {
                shortcuts::save(id, &values, cx);
                self.shortcut_errors.remove(id); self.shortcut_recording = None;
            }
            Err(error) => { self.shortcut_errors.insert(id, error); }
        }
        cx.notify();
    }
    pub(super) fn record_shortcut(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Tab 在浮层中访问控件，移出浮层时关闭并丢弃草稿；Ctrl+Tab 仍可录制。
        let modifiers = event.keystroke.modifiers;
        if matches!(CATEGORIES[self.category].page, Page::Shortcuts) && event.keystroke.key == "tab"
            && !modifiers.control && !modifiers.platform && !modifiers.alt && !modifiers.function {
            if modifiers.shift { window.focus_prev(cx); } else { window.focus_next(cx); }
            if self.shortcut_popup.is_some() && !self.shortcut_popup_focus.contains_focused(window, cx) {
                let next = window.focused(cx);
                self.close_shortcut_popup(window, cx);
                if let Some(next) = next { window.focus(&next, cx); }
            }
            window.prevent_default(); cx.stop_propagation(); return;
        }
        if !self.shortcut_popup_focus.is_focused(window) { return; }
        let Some(recording) = &mut self.shortcut_recording else { return; };
        window.prevent_default(); cx.stop_propagation();
        if event.is_held { return; }
        let key = &event.keystroke;
        if key.key == "escape" {
            self.shortcut_errors.remove(recording.id); self.shortcut_recording = None;
        } else if key.key == "enter" && key.modifiers == gpui::Modifiers::default() {
            self.apply_recording(cx);
        } else if !matches!(key.key.as_str(), "shift" | "control" | "alt" | "cmd" | "fn") {
            match shortcuts::record(key) {
                Ok(value) => {
                    let mut values = shortcuts::current_keys(recording.id, cx);
                    if recording.slot < values.len() { values[recording.slot] = value.clone(); } else { values.push(value.clone()); }
                    recording.draft = Some(value);
                    match shortcuts::validate_keys(recording.id, &values, cx) {
                        Ok(_) => { self.shortcut_errors.remove(recording.id); }
                        Err(error) => { self.shortcut_errors.insert(recording.id, error); }
                    }
                }
                Err(error) => { recording.draft = None; self.shortcut_errors.insert(recording.id, error); }
            }
        }
        cx.notify();
    }
    fn shortcut_overlay(&mut self, panel: gpui::Stateful<gpui::Div>, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> gpui::Div {
        div().absolute().size_full().top_0().left_0()
            .child(gpui::deferred(gpui::anchored().position(point(px(0.0), px(0.0))).child(
                div().id("shortcut-dismiss").w(viewport.width).h(viewport.height).occlude()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| {
                        cx.stop_propagation(); this.close_shortcut_popup(window, cx);
                    }))
                    .on_scroll(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation(); this.close_shortcut_popup(window, cx);
                    }))
            )).with_priority(1))
            .child(gpui::deferred(popup("shortcut-popup-enter", PopupAnchor::Parent, PopupPlacement::BelowRight,
                panel.track_focus(&self.shortcut_popup_focus).tab_group().tab_index(0).tab_stop(false)
                    .p(px(16.0)).flex().flex_col().gap(px(12.0)).rounded(px(10.0))
                    .bg(theme.dropdown_bg).border_1().border_color(theme.dropdown_border)
                    .shadow(vec![theme.floating_shadow.clone()]).occlude()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation()), reduced_motion(cx))).with_priority(2))
    }
    fn shortcut_editor(&mut self, id: &'static str, theme: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let entry = SHORTCUTS.iter().find(|entry| entry.id == id).expect("known shortcut");
        let locale = ToolbarLocale::current(cx);
        let keys = shortcuts::current_keys(id, cx);
        let mut panel = div().id("shortcut-editor").w(px(360.0))
            .child(div().flex().items_center().justify_between().gap(px(8.0))
                .child(div().flex_1().min_w(px(0.0)).text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(locale.app_text(entry.label)))
                .child(shortcut_button("shortcut-editor-close", "×", ButtonKind::Ghost, theme, cx, |this, window, cx| this.close_shortcut_popup(window, cx))
                    .tooltip(tooltip(locale.text("close")))))
            .child(div().text_size(px(12.0)).text_color(theme.muted).child(locale.app_text(entry.scope.label())));
        for (slot, key) in keys.iter().enumerate() {
            panel = panel.child(div().flex().items_center().justify_between().gap(px(8.0))
                .child(shortcut_button(("shortcut-edit-key", slot), "", ButtonKind::Ghost, theme, cx,
                    move |this, window, cx| this.start_recording(id, slot, window, cx))
                    .px(px(0.0)).child(keycap(key, theme)).tooltip(tooltip(locale.app_text("shortcuts.record"))))
                .child(shortcut_button(("shortcut-remove", slot), "×", ButtonKind::Ghost, theme, cx, move |this, _, cx| {
                    let mut values = shortcuts::current_keys(id, cx);
                    if slot < values.len() { values.remove(slot); this.shortcut_recording = None; this.save_shortcut(id, values, cx); }
                }).tooltip(tooltip(locale.app_text("shortcuts.remove")))));
        }
        if keys.is_empty() {
            panel = panel.child(div().text_size(px(13.0)).text_color(theme.muted).child(locale.app_text("shortcuts.unassigned")));
        }
        if let Some(recording) = &self.shortcut_recording {
            let draft = recording.draft.clone();
            let can_apply = draft.is_some() && !self.shortcut_errors.contains_key(id);
            panel = panel.child(div().p(px(12.0)).rounded(px(6.0)).border_1().border_color(theme.accent.opacity(0.6))
                .flex().flex_col().gap(px(10.0)).bg(theme.app_bg)
                .children(draft.as_ref().map(|key| keycap(key, theme)))
                .child(div().text_size(px(12.0)).text_color(theme.muted).child(locale.app_text("shortcuts.pressKeys")))
                .child(div().flex().gap(px(8.0))
                    .child(shortcut_button("shortcut-apply", locale.app_text("shortcuts.apply"), if can_apply { ButtonKind::Primary } else { ButtonKind::Disabled }, theme, cx,
                        move |this, _, cx| { if can_apply { this.apply_recording(cx); } }))
                    .child(shortcut_button("shortcut-cancel", locale.app_text("shortcuts.cancel"), ButtonKind::Ghost, theme, cx,
                        move |this, window, cx| { this.shortcut_recording = None; this.shortcut_errors.remove(id); window.focus(&this.shortcut_popup_focus, cx); cx.notify(); }))));
        } else if keys.len() < shortcuts::MAX_BINDINGS {
            let slot = keys.len();
            panel = panel.child(shortcut_button("shortcut-add", locale.app_text("shortcuts.addBinding"), ButtonKind::Secondary, theme, cx,
                move |this, window, cx| this.start_recording(id, slot, window, cx)));
        }
        if let Some(error) = self.shortcut_errors.get(id) {
            panel = panel.child(div().text_size(px(12.0)).text_color(theme.danger).child(error.render(locale)));
        }
        let defaults = entry.defaults();
        let default_label = if defaults.is_empty() { locale.app_text("shortcuts.unassigned").to_owned() }
            else { defaults.iter().map(|key| shortcuts::display(key)).collect::<Vec<_>>().join(" / ") };
        let modified = keys != defaults;
        panel.child(div().pt(px(10.0)).border_t_1().border_color(theme.border.opacity(0.6)).flex().flex_col().gap(px(6.0))
            .child(div().text_size(px(12.0)).text_color(theme.muted).child(format!("{} · {default_label}", locale.app_text("shortcuts.default"))))
            .when(modified, |el| el.child(shortcut_button("shortcut-reset", locale.app_text("shortcuts.reset"), ButtonKind::Ghost, theme, cx,
                move |this, _, cx| { this.shortcut_recording = None; this.save_shortcut(id, entry.defaults(), cx); }))))
    }
    pub(super) fn render_shortcuts(&mut self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let locale = ToolbarLocale::current(cx);
        let padding = if f32::from(viewport.width) <= COMPACT_WIDTH { 16.0 } else { 24.0 };
        let search = self.shortcut_search(cx);
        search.update(cx, |field, cx| field.set_placeholder(locale.app_text("shortcuts.search"), cx));
        let query = search.read(cx).content.trim().to_lowercase();
        let mut more = div().relative().flex_none().child(shortcut_button("shortcuts-more", "···", ButtonKind::Ghost, theme, cx,
            |this, window, cx| this.open_shortcut_popup(ShortcutPopup::Menu, window, cx))
            .tooltip(tooltip(locale.app_text("shortcuts.more"))));
        if self.shortcut_popup == Some(ShortcutPopup::Menu) {
            let mut panel = div().id("shortcut-menu").w(px(300.0));
            if self.shortcut_reset_confirm {
                panel = panel.child(div().text_size(px(13.0)).child(locale.app_text("shortcuts.resetConfirm")))
                    .child(shortcut_button("shortcuts-confirm-reset", locale.app_text("shortcuts.resetAll"), ButtonKind::Secondary, theme, cx,
                        |this, window, cx| { shortcuts::reset_all(cx); this.close_shortcut_popup(window, cx); }))
                    .child(shortcut_button("shortcuts-cancel-reset", locale.app_text("shortcuts.cancel"), ButtonKind::Ghost, theme, cx,
                        |this, window, cx| this.close_shortcut_popup(window, cx)));
            } else {
                panel = panel.child(shortcut_button("shortcuts-reset-all", locale.app_text("shortcuts.resetAll"), ButtonKind::Ghost, theme, cx,
                    |this, _, cx| { this.shortcut_reset_confirm = true; cx.notify(); }));
            }
            more = more.child(self.shortcut_overlay(panel, theme, viewport, cx));
        }
        let toolbar = div().flex_none().px(px(padding)).pt(px(18.0)).pb(px(12.0))
            .child(div().w_full().max_w(px(740.0)).flex().flex_col().gap(px(10.0))
                .child(div().flex().items_center().gap(px(8.0)).child(div().flex_1().min_w(px(0.0)).child(search)).child(more))
                .child(div().flex().items_center().gap(px(4.0)).children(
                    ["shortcuts.all", "shortcuts.modified", "shortcuts.unassigned"].into_iter().enumerate().map(|(index, label)| {
                        shortcut_button(("shortcut-filter", index), locale.app_text(label), ButtonKind::Ghost, theme, cx,
                            move |this, _, cx| { this.shortcut_filter = index; this.scroll.set_offset(point(px(0.0), px(0.0))); cx.notify(); })
                            .h(px(28.0)).px(px(10.0)).text_size(px(12.0))
                            .when(self.shortcut_filter == index, |el| el.bg(theme.sidebar_active).text_color(theme.accent_strong))
                    })
                )));
        let mut content = div().w_full().max_w(px(740.0)).flex().flex_col()
            .children(shortcuts::warnings(cx).map(|message| div().pb(px(10.0)).text_size(px(12.0)).text_color(theme.warning).child(message)));
        let groups = [("files", "shortcuts.group.files"), ("editing", "shortcuts.group.editing"), ("search", "shortcuts.group.search"),
            ("format", "shortcuts.group.format"), ("insert", "shortcuts.group.insert"), ("view", "shortcuts.group.view")];
        let mut count = 0;
        for (group, title) in groups {
            let mut rows = Vec::new();
            for entry in SHORTCUTS.iter().filter(|entry| entry.group == group) {
                let id = entry.id;
                let keys = shortcuts::current_keys(id, cx);
                let label = locale.app_text(entry.label);
                let modified = keys != entry.defaults();
                let open = self.shortcut_popup == Some(ShortcutPopup::Editor(id));
                let searchable = format!("{label} {id} {} {}", keys.join(" "), shortcuts::current(id, cx)).to_lowercase();
                // 编辑中暂留当前行，避免清除／恢复后被筛选移除，丢失浮层和焦点。
                if !open && ((!query.is_empty() && !searchable.contains(&query))
                    || (self.shortcut_filter == 1 && !modified) || (self.shortcut_filter == 2 && !keys.is_empty())) { continue; }
                count += 1;
                let trigger = shortcut_button(eid("shortcut-edit", id), "", ButtonKind::Ghost, theme, cx,
                    move |this, window, cx| this.open_shortcut_popup(ShortcutPopup::Editor(id), window, cx))
                    .h_auto().min_h(px(38.0)).px(px(4.0)).py(px(4.0)).max_w(px(content_width(viewport) * 0.62))
                    .justify_end().gap(px(5.0)).tooltip(tooltip(locale.app_text("shortcuts.edit")))
                    .child(div().min_w(px(0.0)).flex().flex_wrap().justify_end().gap(px(5.0))
                        .children(keys.iter().map(|key| keycap(key, theme)))
                        .when(keys.is_empty(), |el| el.child(div().text_size(px(12.0)).text_color(theme.muted).child(locale.app_text("shortcuts.unassigned")))))
                    .child(div().text_size(px(14.0)).text_color(theme.muted).child("···"));
                let mut controls = div().relative().flex_none().child(trigger);
                if open {
                    let editor = self.shortcut_editor(id, theme, cx);
                    controls = controls.child(self.shortcut_overlay(editor, theme, viewport, cx));
                }
                rows.push(div().min_h(px(46.0)).flex_none().flex().items_center().justify_between().gap(px(12.0))
                    .border_b_1().border_color(theme.border.opacity(0.4))
                    .child(div().flex_1().min_w(px(0.0)).flex().items_center().gap(px(7.0))
                        .child(div().min_w(px(0.0)).text_size(px(13.0)).text_color(theme.heading).child(label))
                        .when(modified, |el| el.child(div().id(eid("shortcut-modified", id)).size(px(5.0)).flex_none().rounded_full().bg(theme.accent)
                            .tooltip(tooltip(locale.app_text("shortcuts.modified"))))))
                    .child(controls));
            }
            if !rows.is_empty() {
                content = content.child(div().pt(px(18.0)).pb(px(6.0)).text_size(px(12.0)).text_color(theme.muted)
                    .font_weight(FontWeight::SEMIBOLD).child(locale.app_text(title))).children(rows);
            }
        }
        if count == 0 { content = content.child(div().py(px(36.0)).text_size(px(13.0)).text_color(theme.muted).child(locale.app_text("shortcuts.noResults"))); }
        let mut reference = div().relative().flex_none().child(shortcut_button("shortcut-reference", locale.app_text("shortcuts.reference"), ButtonKind::Ghost, theme, cx,
            |this, window, cx| this.open_shortcut_popup(ShortcutPopup::Reference, window, cx)).px(px(0.0)).text_size(px(12.0)));
        if self.shortcut_popup == Some(ShortcutPopup::Reference) {
            let panel = div().id("shortcut-reference-panel").w(px(380.0))
                .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child(locale.app_text("shortcuts.reference")))
                .child(div().text_size(px(12.0)).text_color(theme.muted).child(locale.app_text(if cfg!(target_os="macos") { "shortcuts.helpMac" } else { "shortcuts.helpPc" })))
                .child(div().text_size(px(13.0)).child(locale.app_text(if cfg!(target_os="macos") { "shortcuts.referenceMac" } else { "shortcuts.referencePc" })))
                .child(shortcut_button("shortcut-reference-close", locale.text("close"), ButtonKind::Ghost, theme, cx,
                    |this, window, cx| this.close_shortcut_popup(window, cx)));
            reference = reference.child(self.shortcut_overlay(panel, theme, viewport, cx));
        }
        div().id("settings-shortcuts").tab_group().flex_1().min_h(px(0.0)).flex().flex_col().child(toolbar)
            .child(div().id("shortcut-list").flex_1().min_h(px(0.0)).overflow_y_scroll().track_scroll(&self.scroll)
                .px(px(padding)).pb(px(20.0)).child(content))
            .child(div().flex_none().px(px(padding)).py(px(8.0)).border_t_1().border_color(theme.border.opacity(0.5))
                .child(div().w_full().max_w(px(740.0)).flex().items_center().justify_end()
                    .child(reference)))
            .into_any_element()
    }
}
