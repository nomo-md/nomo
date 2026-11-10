//! 右下角缩放、统计、格式检查与编辑锁；仅保存交互状态，文档锁由会话持有。
use super::*;
use crate::ui::{
    components::{STATUS_SLIDER_THUMB_RADIUS, status_slider},
    popup::status_popup,
    toolbar_i18n::ToolbarLocale,
};
use gpui::{BoxShadow, Bounds, Div, EntityId, KeyDownEvent, Pixels, Stateful, point, relative, rgba};
#[path = "markdown_lint.rs"]
mod markdown_lint;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { Zoom, Stats, Lint }
impl Panel {
    fn index(self) -> usize { match self { Self::Zoom => 0, Self::Stats => 1, Self::Lint => 2 } }
}

pub(super) struct StatusUi {
    open: Option<Panel>,
    opened: Instant,
    generation: usize,
    document: Option<EntityId>,
    locale: &'static str,
    anchors: [Rc<Cell<Option<Bounds<Pixels>>>>; 3],
    triggers: [FocusHandle; 3],
    edit_lock_focus: FocusHandle,
    focus: FocusHandle,
    hover: [bool; 3],
    motion: [ScalarMotion; 3],
    metric_hover: [bool; 4],
    metric_motion: [ScalarMotion; 4],
    step_hover: [bool; 2],
    step_motion: [ScalarMotion; 2],
    slider_motion: ScalarMotion,
    selected: usize,
    keyboard_selection: bool,
    dragging: bool,
    slider_hover: bool,
    last_zoom: Option<i64>,
    pulse: Option<Instant>,
    stats: StatsTransition,
    lint_hint: bool,
}

impl StatusUi {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        Self {
            open: None, opened: Instant::now(), generation: 0, document: None, locale: "",
            anchors: std::array::from_fn(|_| Rc::new(Cell::new(None))),
            triggers: std::array::from_fn(|_| cx.focus_handle()), edit_lock_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            hover: [false; 3], motion: [ScalarMotion::default(); 3],
            metric_hover: [false; 4], metric_motion: [ScalarMotion::default(); 4],
            step_hover: [false; 2], step_motion: [ScalarMotion::default(); 2],
            slider_motion: ScalarMotion::default(),
            selected: 0, keyboard_selection: false, dragging: false, slider_hover: false,
            last_zoom: None, pulse: None,
            stats: StatsTransition::default(),
            lint_hint: false,
        }
    }
}

/// 只暂存两个计数；位数按当前文档和统计类型的最大值预留，选区变短不挤动缩放按钮。
#[derive(Default)]
struct StatsTransition {
    metric: Option<usize>,
    value: Option<u64>,
    selected: bool,
    digits: usize,
    outgoing: Option<(u64, Instant)>,
}

impl StatsTransition {
    fn update(&mut self, metric: usize, value: Option<u64>, full: Option<u64>, selected: bool, reduced: bool) {
        if self.metric != Some(metric) {
            *self = Self { metric: Some(metric), ..Self::default() };
        }
        if reduced || value.is_none() || self.outgoing.is_some_and(|(_, started)| started.elapsed() >= Duration::from_millis(160)) {
            self.outgoing = None;
        }
        // 同一选区范围内的刷新只替换目标值，不重启动画；反向切换覆盖旧过渡，不排队。
        if self.selected != selected {
            self.outgoing = if reduced || self.value == value { None }
                else { self.value.zip(value).map(|(previous, _)| (previous, Instant::now())) };
        }
        self.value = value;
        self.selected = selected;
        for count in [value, full].into_iter().flatten() {
            self.digits = self.digits.max(count.checked_ilog10().unwrap_or(0) as usize + 1);
        }
    }

    fn render(&self, label: String, unit: &'static str) -> Div {
        let Some(value) = self.value else { return div().child(label); };
        let mut number = div().relative()
            .child(div().opacity(0.0).child("0".repeat(self.digits)));
        let mut opacity = 1.0;
        let mut offset = 0.0;
        if let Some((previous, started)) = self.outgoing {
            let progress = (started.elapsed().as_secs_f32() / 0.16).min(1.0);
            let incoming = motion::ease(((progress - 0.15) / 0.85).clamp(0.0, 1.0));
            let outgoing = motion::ease((progress / 0.72).min(1.0));
            let distance = if self.selected { 3.0 } else { -3.0 };
            opacity = incoming;
            offset = distance * (1.0 - incoming);
            number = number.child(div().absolute().right_0().top(px(-distance * outgoing))
                .opacity(1.0 - outgoing).child(previous.to_string()));
        }
        div().flex().items_center()
            .child(number.child(div().absolute().right_0().top(px(offset)).opacity(opacity).child(value.to_string())))
            .child(unit)
    }
}

const METRICS: [(StatsMetric, &str, &str, &str); 4] = [
    (StatsMetric::Lines, "lines", "status_lines", "status_line_unit"),
    (StatsMetric::Words, "words", "status_words", "status_word_unit"),
    (StatsMetric::VisibleChars, "visibleChars", "status_visible_chars", "status_visible_char_unit"),
    (StatsMetric::Chars, "chars", "status_chars", "status_char_unit"),
];

fn shadow(theme: &Theme, hover: f32, dialog: bool) -> BoxShadow {
    BoxShadow {
        inset: false,
        color: if dialog { rgba(if theme.dark { 0x0000006b } else { 0x00000038 }).into() }
            else { rgba(if theme.dark { 0x0000002e } else { 0x16202c14 }).into() },
        offset: point(px(0.0), px(if dialog { 20.0 } else { 1.0 + hover * if theme.dark { 15.0 } else { 13.0 } })),
        blur_radius: px(if dialog { 60.0 } else { 4.0 + hover * if theme.dark { 40.0 } else { 30.0 } }),
        spread_radius: px(0.0),
    }
}

impl Workspace {
    pub(super) fn close_status(&mut self, cx: &mut Context<Self>) {
        // Esc、切换标签或关闭统计也必须提交最后一次滑杆值。
        if self.status_ui.dragging {
            let percent = (cx.global::<Settings>().zoom * 100.0).round() as i64;
            Settings::set_with(cx, "zoomPercent", serde_json::json!(percent), true);
        }
        self.status_ui.open = None;
        self.status_ui.lint_hint = false;
        self.status_ui.dragging = false;
        self.status_ui.slider_hover = false;
        self.status_ui.metric_hover = [false; 4];
        self.status_ui.metric_motion = [ScalarMotion::default(); 4];
        self.status_ui.step_hover = [false; 2];
        self.status_ui.step_motion = [ScalarMotion::default(); 2];
        self.status_ui.slider_motion.reset();
    }

    fn toggle_status(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        let close = self.status_ui.open == Some(panel);
        self.close_status(cx);
        if close {
            window.focus(&self.status_ui.triggers[panel.index()], cx);
        } else {
            if panel == Panel::Stats {
                if let Some(document) = self.document() { document.update(cx, |host, cx| host.retry_stats(cx)); }
            }
            self.status_ui.open = Some(panel);
            self.status_ui.opened = Instant::now();
            self.status_ui.generation = self.status_ui.generation.wrapping_add(1);
            self.status_ui.selected = if panel == Panel::Lint { 0 } else { METRICS.iter().position(|m| m.0 == cx.global::<Settings>().stats_metric).unwrap_or(1) };
            self.status_ui.keyboard_selection = false;
            window.focus(&self.status_ui.focus, cx);
        }
        cx.notify();
    }

    pub(super) fn set_status_zoom(&mut self, percent: i64, persist: bool, cx: &mut Context<Self>) {
        let percent = percent.clamp(80, 160);
        let current = (cx.global::<Settings>().zoom * 100.0).round() as i64;
        if percent != current {
            if let Some(document) = self.document() {
                document.update(cx, |host, cx| host.prepare_zoom(None, cx));
            }
        }
        Settings::set_with(cx, "zoomPercent", serde_json::json!(percent), persist);
        cx.notify();
    }

    fn choose_status_metric(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        Settings::set(cx, "writingStatsMetric", serde_json::json!(METRICS[index].1));
        self.close_status(cx);
        window.focus(&self.status_ui.triggers[Panel::Stats.index()], cx);
        cx.notify();
    }

    pub(super) fn toggle_editing_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.document() else { return; };
        let locked = !document.read(cx).editing_locked();
        if locked {
            let target = document.downgrade();
            if !self.prepare_document_transition(&document, false, window, cx, move |workspace, window, cx| {
                if let Some(document) = target.upgrade() {
                    workspace.set_document_editing_locked(&document, true, window, cx);
                }
            }) {
                return;
            }
        }
        self.set_document_editing_locked(&document, locked, window, cx);
    }

    fn set_document_editing_locked(&mut self, document: &Entity<DocumentHost>, locked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.document().as_ref() != Some(document) { return; }
        if let Err(error) = document.update(cx, |host, cx| host.set_editing_locked(locked, cx)) {
            self.failure_notice = Some(ToolbarLocale::current(cx).error(&error).into());
            cx.notify();
            return;
        }
        self.close_status(cx);
        if locked {
            self.popover = None;
            self.toolbar_popup = None;
            self.cancel_outline_drag(cx);
            if let Some(search) = &mut self.search {
                search.show_replace = false;
            }
        }
        window.focus(&document.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn render_status_controls(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let doc = self.document();
        if let Some(document) = &doc { document.update(cx, |host, cx| { host.sync_stats_visibility(cx); host.sync_lint(cx); }); }
        let document = doc.as_ref().map(|doc| doc.entity_id());
        let settings = cx.global::<Settings>();
        let zoom_visible = settings.zoom_control_visible;
        let stats_visible = settings.stats_visible;
        let lint_visible = settings.lint_indicator_visible;
        let edit_lock_visible = settings.edit_lock_button_visible;
        let panels = [zoom_visible, stats_visible, lint_visible];
        let locale = ToolbarLocale::current(cx);
        let language = locale.text("status_stats");
        let context_changed = document != self.status_ui.document || language != self.status_ui.locale;
        let panel_hidden = self.status_ui.open.is_some_and(|panel| !panels[panel.index()]);
        let dismiss_panel = context_changed || panel_hidden;
        let focus_lost = (dismiss_panel && self.status_ui.focus.contains_focused(window, cx))
            || self.status_ui.triggers.iter().zip(panels).any(|(focus, visible)| (context_changed || !visible) && focus.is_focused(window))
            || ((!edit_lock_visible || document.is_none()) && self.status_ui.edit_lock_focus.is_focused(window));
        if dismiss_panel {
            self.close_status(cx);
            self.status_ui.document = document;
            self.status_ui.locale = language;
            self.status_ui.hover = [false; 3];
            self.status_ui.motion = [ScalarMotion::default(); 3];
            self.status_ui.last_zoom = None;
            self.status_ui.pulse = None;
            self.status_ui.stats = StatsTransition::default();
            for anchor in &self.status_ui.anchors { anchor.set(None); }
        }
        if focus_lost {
            if let Some(document) = &doc { window.focus(&document.focus_handle(cx), cx); }
            else { window.focus(&self.focus, cx); }
        }
        let doc = doc?;
        let reduced = self.explorer.reduce_motion;
        for (index, visible) in panels.into_iter().enumerate() {
            if visible {
                let active = self.status_ui.hover[index] || self.status_ui.open.is_some_and(|panel| panel.index() == index);
                self.status_ui.motion[index].set(active, reduced, Duration::from_millis(140));
            } else {
                self.status_ui.anchors[index].set(None);
                self.status_ui.hover[index] = false;
                self.status_ui.motion[index].reset();
            }
        }
        if !zoom_visible {
            self.status_ui.last_zoom = None;
            self.status_ui.pulse = None;
        }
        if !stats_visible { self.status_ui.stats = StatsTransition::default(); }
        let edit_lock = edit_lock_visible.then(|| self.edit_lock_button(doc.read(cx).editing_locked(), theme, cx));
        let controls = div().absolute().right(px(16.0)).bottom(px(14.0)).flex().items_center().gap(px(8.0));
        if !zoom_visible && !stats_visible && !lint_visible {
            return edit_lock.map(|button| gpui::deferred(controls.child(button)).with_priority(3).into_any_element());
        }
        let settings = cx.global::<Settings>();
        let percent = (settings.zoom * 100.0).round() as i64;
        let metric = METRICS.iter().position(|m| m.0 == settings.stats_metric).unwrap_or(1);
        let stats_label = stats_visible.then(|| {
            let host = doc.read(cx);
            let metric_value = |stats: &crate::text_document::DocumentStats| [stats.lines, stats.words, stats.visible_chars, stats.chars][metric];
            let value = host.stats().map(metric_value);
            self.status_ui.stats.update(metric, value, host.full_stats().map(metric_value), host.stats_are_selected(), reduced);
            format!("{}{}", value.map(|value| value.to_string()).unwrap_or_else(|| "—".into()), locale.text(METRICS[metric].3))
        });
        if zoom_visible && self.status_ui.last_zoom.is_some_and(|previous| previous != percent) {
            self.status_ui.pulse = Some(Instant::now());
        }
        self.status_ui.last_zoom = zoom_visible.then_some(percent);
        let mut scale = 1.0;
        if let Some(started) = self.status_ui.pulse {
            let progress = (started.elapsed().as_secs_f32() / if reduced { 0.08 } else { 0.12 }).min(1.0);
            if progress >= 1.0 { self.status_ui.pulse = None; }
            else {
                let t = progress - 1.0;
                let eased = 1.0 + 2.70158 * t * t * t + 1.70158 * t * t;
                let amount = if reduced { 0.02 } else { 0.06 };
                scale = 1.0 - amount + amount * eased;
            }
        }
        for index in 0..2 {
            let enabled = zoom_visible && if index == 0 { percent > 80 } else { percent < 160 };
            self.status_ui.step_motion[index].set(enabled && self.status_ui.step_hover[index], reduced, Duration::from_millis(120));
        }
        for index in 0..4 {
            self.status_ui.metric_motion[index].set(self.status_ui.open == Some(Panel::Stats)
                && (metric == index || self.status_ui.metric_hover[index]), reduced, Duration::from_millis(140));
        }
        self.status_ui.slider_motion.set(self.status_ui.slider_hover, reduced, Duration::from_millis(120));
        let animate = self.status_ui.motion.iter().any(ScalarMotion::running)
            || self.status_ui.metric_motion.iter().any(ScalarMotion::running)
            || self.status_ui.step_motion.iter().any(ScalarMotion::running)
            || self.status_ui.slider_motion.running()
            || self.status_ui.pulse.is_some()
            || self.status_ui.stats.outgoing.is_some()
            || (!reduced && self.status_ui.open.is_some() && self.status_ui.opened.elapsed() < Duration::from_millis(140));
        let controls = controls.children(animate.then(motion::theme_frame))
            .when(zoom_visible, |controls| controls.child(self.status_trigger(Panel::Zoom, format!("{percent}%"), scale, theme, cx)))
            .children(stats_label.map(|label| self.status_trigger(Panel::Stats, label, 1.0, theme, cx)))
            .when(lint_visible, |controls| controls.child(self.status_trigger(Panel::Lint, self.lint_label(cx), 1.0, theme, cx)))
            .children(edit_lock);
        Some(gpui::deferred(controls).with_priority(3).into_any_element())
    }

    fn edit_lock_button(&self, locked: bool, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let locale = ToolbarLocale::current(cx);
        let title = locale.app_text(if locked { "documentLock.unlock" } else { "documentLock.lock" });
        let shortcut = crate::ui::shortcuts::current("toggle-edit-lock", cx);
        let icon = if locked { IconName::Lock } else { IconName::LockOpen };
        small_icon_button("status-edit-lock", icon, locked, theme)
            .size(px(28.0)).rounded(px(8.0)).border_1().border_color(theme.border.opacity(0.82))
            .when(!locked, |button| button.bg(motion::color(theme.surface, theme.rail_bg, 0.06)))
            .shadow(vec![shadow(theme, 0.0, false)])
            .role(gpui::Role::Button).aria_label(title)
            .track_focus(&self.status_ui.edit_lock_focus).tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .tooltip(tooltip(format!("{title} ({shortcut})")))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|workspace, _, window, cx| {
                workspace.toggle_editing_lock(window, cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|workspace, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    if !event.is_held { workspace.toggle_editing_lock(window, cx); }
                    cx.stop_propagation();
                }
            }))
    }

    fn status_trigger(&self, panel: Panel, label: String, scale: f32, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let index = panel.index();
        let hover = self.status_ui.motion[index].value();
        let locale = ToolbarLocale::current(cx);
        let title = if panel == Panel::Lint { markdown_lint::text(locale, "title") }
            else { locale.text(if index == 0 { "status_zoom" } else { "status_word_count" }) };
        let mono = crate::theme::font_for(crate::theme::Family::Mono, false, false).family;
        let bounds = self.status_ui.anchors[index].clone();
        let background = motion::color(motion::color(theme.surface, theme.rail_bg, 0.06), theme.surface, hover);
        let border = motion::color(theme.border.opacity(0.82), motion::color(theme.border, theme.accent, 0.3), hover);
        let unit = locale.text(METRICS[self.status_ui.stats.metric.unwrap_or(1)].3);
        let reserved = if panel == Panel::Stats && self.status_ui.stats.value.is_some() {
            format!("{}{unit}", "0".repeat(self.status_ui.stats.digits))
        } else { label.clone() };
        let content = if panel == Panel::Stats { self.status_ui.stats.render(label, unit) }
            else { div().child(label) };
        // 外层保留稳定命中区域，内层缩放不会挤动相邻按钮或重新定位弹层。
        div().id(("status-trigger", index)).relative().min_w(px(if index == 0 { 52.0 } else { 58.0 }))
            .role(gpui::Role::Button).aria_label(title)
            .aria_expanded(self.status_ui.open == Some(panel))
            .h(px(28.0)).px(px(10.0)).flex_none().rounded(px(8.0)).cursor_pointer()
            .font_family(mono).font_weight(FontWeight::MEDIUM).text_size(px(13.0))
            .track_focus(&self.status_ui.triggers[index]).tab_index(0)
            .focus_visible(|style| style.shadow(vec![BoxShadow { inset: false,
                color: theme.accent.opacity(0.45), offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0), spread_radius: px(3.0) }]))
            .tooltip(tooltip(title))
            .child(div().opacity(0.0).whitespace_nowrap().child(reserved))
            .child(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center()
                .child(div().relative().top(px(if self.explorer.reduce_motion { 0.0 } else { -hover }))
                    .w(relative(scale)).h(px(28.0 * scale)).flex().items_center().justify_center()
                    .rounded(px(8.0 * scale)).border_1().border_color(border).bg(background)
                    .shadow(vec![shadow(theme, hover, false)]).text_size(px(13.0 * scale))
                    .text_color(motion::color(theme.heading, theme.fg, hover)).whitespace_nowrap().child(content)))
            .child(gpui::canvas(move |value, _, _| { bounds.set(Some(value)); }, |_, _, _, _| {})
                .absolute().top_0().left_0().size_full())
            .on_hover(cx.listener(move |workspace, hovered, _, cx| {
                workspace.status_ui.hover[index] = *hovered; cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |workspace, _, window, cx| {
                workspace.toggle_status(panel, window, cx); cx.stop_propagation();
            }))
            .on_key_down(cx.listener(move |workspace, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "up" | "down") {
                    workspace.toggle_status(panel, window, cx); cx.stop_propagation();
                }
            }))
    }

    pub(super) fn render_status_popup(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.status_ui.open?;
        if kind == Panel::Lint { return self.render_lint_popup(theme, cx); }
        let locale = ToolbarLocale::current(cx);
        let reduced = self.explorer.reduce_motion;
        let progress = if reduced { 1.0 } else { (self.status_ui.opened.elapsed().as_secs_f32() / 0.14).min(1.0) };
        let scale = 0.98 + 0.02 * (1.0 - (1.0 - progress).powi(3));
        let sp = |value| px(value * scale);
        let mut panel = div().id("status-popup").track_focus(&self.status_ui.focus)
            .role(gpui::Role::Dialog).aria_label(locale.text(if kind == Panel::Zoom { "status_zoom" } else { "status_stats" }))
            .w(sp(if kind == Panel::Zoom { 180.0 } else { 148.0 })).flex().flex_col()
            .rounded(sp(8.0)).border_1().border_color(theme.border.opacity(0.82))
            .bg(motion::color(theme.surface, theme.rail_bg, 0.03))
            .shadow(vec![shadow(theme, 0.0, true)]).text_color(theme.fg).text_size(sp(13.0)).occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|workspace, event: &gpui::MouseDownEvent, window, cx| {
                if !workspace.status_ui.dragging && !workspace.status_ui.anchors.iter()
                    .any(|anchor| anchor.get().is_some_and(|bounds| bounds.contains(&event.position))) {
                    let restore_focus = workspace.status_ui.focus.contains_focused(window, cx);
                    workspace.close_status(cx);
                    if restore_focus && let Some(document) = workspace.document() {
                        window.focus(&document.focus_handle(cx), cx);
                    }
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |workspace, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        workspace.close_status(cx);
                        window.focus(&workspace.status_ui.triggers[kind.index()], cx);
                    }
                    "up" | "down" if kind == Panel::Stats => {
                        workspace.status_ui.selected = (workspace.status_ui.selected + if event.keystroke.key == "up" { 3 } else { 1 }) % 4;
                        workspace.status_ui.keyboard_selection = true;
                    }
                    "enter" | "space" if kind == Panel::Stats => {
                        workspace.choose_status_metric(workspace.status_ui.selected, window, cx);
                    }
                    "left" | "right" | "up" | "down" | "home" | "end" if kind == Panel::Zoom => {
                        let current = (cx.global::<Settings>().zoom * 100.0).round() as i64;
                        let next = match event.keystroke.key.as_str() {
                            "left" | "down" => current - 5, "right" | "up" => current + 5,
                            "home" => 80, _ => 160,
                        };
                        workspace.set_status_zoom(next, true, cx);
                    }
                    _ => return,
                }
                cx.stop_propagation(); cx.notify();
            }));
        if kind == Panel::Zoom {
            let value = (cx.global::<Settings>().zoom * 100.0).round() as i64;
            let slider_width = 154.0 * scale;
            let slider_fraction = |percent: i64| (percent - 80) as f32 / 80.0;
            let track_width = (slider_width - 2.0 * STATUS_SLIDER_THUMB_RADIUS).max(1.0);
            let reset_center = STATUS_SLIDER_THUMB_RADIUS + track_width * slider_fraction(100);
            let reset_width = 44.0 * scale;
            let reset_title = format!("{} (100%)", locale.app_text("menu.zoomReset"));
            let reset_marker = div().id("status-zoom-reset")
                .ml(px(reset_center - reset_width / 2.0))
                .w(px(reset_width)).h(sp(22.0))
                .flex().items_center().justify_center()
                .rounded(sp(5.0)).border_1().border_color(theme.border.opacity(0.7))
                .text_size(sp(11.0)).font_weight(FontWeight::MEDIUM).text_color(theme.heading)
                .role(gpui::Role::Button).aria_label(reset_title.clone())
                .tab_index(0).cursor_pointer().tooltip(tooltip(reset_title))
                .hover(|style| style.bg(theme.accent.opacity(0.10)))
                .focus_visible(|style| style.border_color(theme.accent))
                .child("100%")
                .on_click(cx.listener(|workspace, _, _, cx| {
                    workspace.zoom(0.0, cx);
                    cx.stop_propagation();
                }));
            let weak = cx.entity().downgrade();
            let on_change = Rc::new(move |fraction: f32, dragging: bool, _: &mut Window, cx: &mut App| {
                let percent = 80 + (fraction * 16.0).round() as i64 * 5;
                let _ = weak.update(cx, |workspace, cx| {
                    // 关闭后到达的旧鼠标回调不重新创建拖动状态。
                    if workspace.status_ui.open != Some(Panel::Zoom) { return; }
                    workspace.status_ui.dragging = dragging;
                    workspace.set_status_zoom(percent, !dragging, cx);
                });
            });
            panel = panel.child(div().flex().flex_none().items_center().justify_between().px(sp(12.0)).py(sp(10.0))
                .border_b_1().border_color(theme.border)
                .child(self.status_zoom_step(-5, value > 80, scale, theme, cx))
                .child(div().text_size(sp(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(theme.heading)
                    .font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family).child(format!("{value}%")))
                .child(self.status_zoom_step(5, value < 160, scale, theme, cx)))
                .child(div().p(sp(12.0)).flex_none().flex().flex_col().items_start().gap(sp(2.0))
                    .child(status_slider("status-zoom-slider", slider_fraction(value),
                        self.status_ui.dragging, slider_width, theme, on_change, self.status_ui.slider_motion.value())
                        .role(gpui::Role::Slider).aria_label(locale.text("status_zoom"))
                        .aria_numeric_value(value as f64).aria_min_numeric_value(80.0).aria_max_numeric_value(160.0).aria_numeric_value_step(5.0)
                        .tab_index(0).tooltip(tooltip(locale.text("status_zoom")))
                        .focus_visible(|style| style.bg(theme.accent.opacity(0.07)))
                        .on_hover(cx.listener(|workspace, hovered, _, cx| {
                            workspace.status_ui.slider_hover = *hovered; cx.notify();
                        })))
                    .child(reset_marker));
        } else {
            let document = self.document()?;
            let stats = document.read(cx).stats().cloned();
            let active = cx.global::<Settings>().stats_metric;
            panel = panel.child(div().flex_none().pt(sp(11.0)).pb(sp(9.0)).px(sp(12.0))
                .border_b_1().border_color(theme.border).text_size(sp(14.0)).font_weight(FontWeight::SEMIBOLD)
                .child(locale.text("status_stats")));
            if cx.global::<Settings>().reading_time {
                if let Some(stats) = &stats {
                    panel = panel.child(div().flex_none().mx(sp(6.0)).mt(sp(6.0)).py(sp(7.0)).px(sp(8.0))
                        .border_1().border_color(theme.border).rounded(sp(4.0)).bg(theme.accent.opacity(0.07))
                        .text_color(theme.muted).child(locale.text("status_reading").replace("{minutes}", &stats.reading_minutes.to_string())));
                }
            }
            let mut options = div().flex().flex_col().gap(sp(2.0)).p(sp(6.0));
            for (index, (metric, _, label, _)) in METRICS.iter().enumerate() {
                let value = stats.as_ref().map(|s| [s.lines, s.words, s.visible_chars, s.chars][index].to_string()).unwrap_or_else(|| "—".into());
                options = options.child(div().id(("status-metric", index)).tab_index(0)
                    .role(gpui::Role::Button).aria_label(locale.text(label)).aria_selected(*metric == active)
                    .flex_none().min_h(sp(30.0)).px(sp(8.0)).flex().items_center().justify_between().gap(sp(10.0))
                    .rounded(sp(8.0)).cursor_pointer().text_color(theme.muted)
                    .bg(theme.accent.opacity(0.07 * self.status_ui.metric_motion[index].value()))
                    .when(self.status_ui.keyboard_selection && self.status_ui.selected == index,
                        |el| el.bg(theme.accent.opacity(0.12)))
                    .focus_visible(|style| style.bg(theme.accent.opacity(0.12)))
                    .child(div().min_w_0().text_ellipsis().child(locale.text(label)))
                    .child(div().text_size(sp(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(theme.heading)
                        .font_family(crate::theme::font_for(crate::theme::Family::Mono, true, false).family).child(value))
                    .on_click(cx.listener(move |workspace, _, window, cx| workspace.choose_status_metric(index, window, cx)))
                    .on_hover(cx.listener(move |workspace, hovered, _, cx| {
                        workspace.status_ui.metric_hover[index] = *hovered; cx.notify();
                    }))
                    .on_key_down(cx.listener(move |workspace, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            workspace.choose_status_metric(index, window, cx); cx.stop_propagation();
                        }
                    })));
            }
            panel = panel.child(options);
        }
        let anchor = PopupAnchor::Tracked(self.status_ui.anchors[kind.index()].clone(), point(px(16.0), px(14.0)));
        Some(gpui::deferred(status_popup(("status-popup-open", self.status_ui.generation), anchor, panel, reduced))
            .with_priority(6).into_any_element())
    }

    fn status_zoom_step(&self, delta: i64, enabled: bool, scale: f32, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let locale = ToolbarLocale::current(cx);
        let index = usize::from(delta > 0);
        let hover = self.status_ui.step_motion[index].value();
        div().id(if delta < 0 { "status-zoom-out" } else { "status-zoom-in" })
            .role(gpui::Role::Button).aria_label(locale.text(if delta < 0 { "status_zoom_out" } else { "status_zoom_in" }))
            .size(px(28.0 * scale)).flex_none().flex().items_center().justify_center()
            .rounded(px(8.0 * scale)).border_1()
            .border_color(motion::color(theme.border.opacity(0.7), motion::color(theme.border, theme.accent, 0.35), hover))
            .bg(motion::color(theme.surface, theme.rail_bg, 0.1 * (1.0 - hover)))
            .text_color(motion::color(theme.heading, theme.fg, hover))
            .text_size(px(16.0 * scale)).font_weight(FontWeight::MEDIUM)
            .when(enabled, |el| el.tab_index(0).cursor_pointer())
            .when(!enabled, |el| el.opacity(0.35).cursor(gpui::CursorStyle::OperationNotAllowed))
            .focus_visible(|style| style.border_color(theme.accent))
            .tooltip(tooltip(locale.text(if delta < 0 { "status_zoom_out" } else { "status_zoom_in" })))
            .child(if delta < 0 { "−" } else { "+" })
            .on_hover(cx.listener(move |workspace, hovered, _, cx| {
                workspace.status_ui.step_hover[index] = *hovered; cx.notify();
            }))
            .on_click(cx.listener(move |workspace, _, _, cx| {
                if enabled { workspace.zoom(delta as f32 / 100.0, cx); }
            }))
            .on_key_down(cx.listener(move |workspace, event: &KeyDownEvent, _, cx| {
                if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    workspace.zoom(delta as f32 / 100.0, cx); cx.stop_propagation();
                }
            }))
    }
}
