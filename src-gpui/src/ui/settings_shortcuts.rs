//! 快捷键草稿只归设置窗口所有；Enter 提交、Escape 恢复，输入过程中不重建键表。
use super::*;
use crate::ui::shortcuts::{self, SHORTCUTS};

impl SettingsWindow {
    fn shortcut_field(&mut self, id: &'static str, cx: &mut Context<Self>) -> Entity<TextField> {
        if let Some(field) = self.fields.get(id) { return field.clone(); }
        let current = shortcuts::current(id, cx);
        let field = cx.new(|cx| {
            let mut field = TextField::new("Ctrl+Shift+K", cx);
            field.set_settings_style(); field.set_settings_monospace(); field.content = current; field
        });
        self._subscriptions.push(cx.subscribe(&field, move |this, field, event, cx| {
            if field.read(cx).is_composing() { return; }
            if matches!(event, FieldEvent::Cancel) {
                let current = shortcuts::current(id, cx);
                field.update(cx, |field, cx| field.set_text(&current, cx));
                this.shortcut_errors.remove(id); cx.notify(); return;
            }
            let value = field.read(cx).content.clone();
            match shortcuts::validate(id, &value, cx) {
                Ok(normalized) => {
                    this.shortcut_errors.remove(id);
                    field.update(cx, |field, cx| field.set_invalid(false, cx));
                    if matches!(event, FieldEvent::Submit { .. }) {
                        Settings::set(cx, &format!("shortcutPreferences.{id}"), json!(normalized));
                        field.update(cx, |field, cx| field.set_text(&normalized, cx));
                    }
                }
                Err(error) => { this.shortcut_errors.insert(id, error); field.update(cx, |field, cx| field.set_invalid(true, cx)); }
            }
            cx.notify();
        }));
        self.fields.insert(id, field.clone()); field
    }

    pub(super) fn render_shortcuts(&mut self, theme: &Theme, viewport: gpui::Size<gpui::Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let compact = f32::from(viewport.width) <= COMPACT_WIDTH;
        let rows: Vec<_> = SHORTCUTS.iter().map(|entry| {
            let field = self.shortcut_field(entry.id, cx);
            let id = entry.id;
            let error = self.shortcut_errors.get(id).cloned();
            div().min_h(px(54.0)).py(px(10.0)).border_t_1().border_color(theme.border.opacity(0.72))
                .flex().justify_between().gap(px(12.0))
                .when(compact, |el| el.flex_col().items_start()).when(!compact, |el| el.items_center())
                .child(div().min_w(px(0.0)).flex_1().text_size(px(14.0)).text_color(theme.heading).child(entry.label))
                .child(div().w(px(control_width(viewport).max(220.0))).flex_none().flex().flex_col().gap(px(4.0))
                    .child(div().flex().gap(px(6.0)).child(div().flex_1().child(field))
                        .child(button(eid("shortcut-reset", id), "恢复", None, ButtonKind::Secondary, theme)
                            .tooltip(tooltip(format!("恢复为 {}", entry.default)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match shortcuts::validate(id, entry.default, cx) {
                                    Ok(value) => {
                                        Settings::set(cx, &format!("shortcutPreferences.{id}"), json!(value));
                                        if let Some(field) = this.fields.get(id) { field.update(cx, |field, cx| field.set_text(&value, cx)); }
                                        this.shortcut_errors.remove(id);
                                    }
                                    Err(error) => { this.shortcut_errors.insert(id, error); }
                                }
                                cx.notify();
                            }))))
                    .children(error.map(|error| div().text_size(px(12.0)).text_color(theme.danger).child(error))))
                .into_any_element()
        }).collect();
        div().id("settings-shortcuts").flex_none().flex().flex_col()
            .child(div().py(px(10.0)).text_size(px(13.0)).text_color(theme.muted)
                .child("输入组合后按 Enter 应用；Escape 放弃修改。macOS 的 Ctrl 对应 Command。"))
            .children(shortcuts::warnings(cx).map(|message| div().text_size(px(12.0)).text_color(theme.warning).child(message)))
            .children(rows).into_any_element()
    }
}
