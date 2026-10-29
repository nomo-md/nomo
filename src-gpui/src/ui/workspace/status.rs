//! 编辑区顶部信息行（保存状态 · 统计 · 缩放），与正文列左对齐、不随文档滚动；
//! 仅保存浮层交互，不持有文档、正文或统计任务。
use super::*;
use crate::ui::{components::status_slider, popup::status_popup, toolbar_i18n::ToolbarLocale};
use gpui::{BoxShadow, Bounds, Div, EntityId, KeyDownEvent, Pixels, Stateful, point, relative, rgba};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { Zoom, Stats }
impl Panel {
    fn index(self) -> usize { if self == Self::Zoom { 0 } else { 1 } }
}

pub(super) struct StatusUi {
    open: Option<Panel>,
    opened: Instant,
    generation: usize,
    document: Option<EntityId>,
    locale: &'static str,
    anchors: [Rc<Cell<Option<Bounds<Pixels>>>>; 2],
    triggers: [FocusHandle; 2],
    focus: FocusHandle,
    hover: [bool; 2],
    motion: [ScalarMotion; 2],
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
}

impl StatusUi {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        Self {
            open: None, opened: Instant::now(), generation: 0, document: None, locale: "",
            anchors: std::array::from_fn(|_| Rc::new(Cell::new(None))),
            triggers: std::array::from_fn(|_| cx.focus_handle()), focus: cx.focus_handle(),
            hover: [false; 2], motion: [ScalarMotion::default(); 2],
            metric_hover: [false; 4], metric_motion: [ScalarMotion::default(); 4],
            step_hover: [false; 2], step_motion: [ScalarMotion::default(); 2],
            slider_motion: ScalarMotion::default(),
            selected: 0, keyboard_selection: false, dragging: false, slider_hover: false,
            last_zoom: None, pulse: None,
        }
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
            self.status_ui.selected = METRICS.iter().position(|m| m.0 == cx.global::<Settings>().stats_metric).unwrap_or(1);
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

    /// `left` 为正文列相对编辑区的左边距（px），信息行文字与正文首字对齐。
    pub(super) fn render_status_controls(&mut self, theme: &Theme, left: f32, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(document) = self.document() { document.update(cx, |host, cx| host.sync_stats_visibility(cx)); }
        let document = self.document().map(|doc| doc.entity_id());
        let visible = document.is_some() && cx.global::<Settings>().stats_visible;
        let locale = ToolbarLocale::current(cx);
        let language = locale.text("status_stats");
        if !visible || document != self.status_ui.document || language != self.status_ui.locale {
            let focused = self.status_ui.focus.contains_focused(window, cx)
                || self.status_ui.triggers.iter().any(|focus| focus.contains_focused(window, cx));
            self.close_status(cx);
            self.status_ui.document = document;
            self.status_ui.locale = language;
            self.status_ui.hover = [false; 2];
            self.status_ui.motion = [ScalarMotion::default(); 2];
            self.status_ui.last_zoom = None;
            self.status_ui.pulse = None;
            for anchor in &self.status_ui.anchors { anchor.set(None); }
            if focused {
                if let Some(document) = self.document() { window.focus(&document.focus_handle(cx), cx); }
                else { window.focus(&self.focus, cx); }
            }
        }
        if !visible { return None; }
        let settings = cx.global::<Settings>();
        let percent = (settings.zoom * 100.0).round() as i64;
        let metric = METRICS.iter().position(|m| m.0 == settings.stats_metric).unwrap_or(1);
        let doc = self.document()?;
        let host = doc.read(cx);
        let label = if host.stats_failed() && host.stats().is_none() { locale.text("status_unavailable").to_owned() }
            else {
                let value = host.stats().map(|stats| [stats.lines, stats.words, stats.visible_chars, stats.chars][metric].to_string())
                    .unwrap_or_else(|| "—".into());
                format!("{value}{}", locale.text(METRICS[metric].3))
            };
        let reduced = self.explorer.reduce_motion;
        if self.status_ui.last_zoom.is_some_and(|previous| previous != percent) {
            self.status_ui.pulse = Some(Instant::now());
        }
        self.status_ui.last_zoom = Some(percent);
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
            let active = self.status_ui.hover[index] || self.status_ui.open.is_some_and(|panel| panel.index() == index);
            self.status_ui.motion[index].set(active, reduced, Duration::from_millis(140));
            let enabled = if index == 0 { percent > 80 } else { percent < 160 };
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
            || (!reduced && self.status_ui.open.is_some() && self.status_ui.opened.elapsed() < Duration::from_millis(140));
        let dirty = doc.read(cx).is_dirty(cx);
        let separator = || div().flex_none().opacity(0.6).child("·");
        // 触发器自带 8px 内边距，整体左移同样距离使文字与正文首字对齐。
        let controls = div().w_full().h(px(32.0)).flex_none().flex().items_center().gap(px(2.0))
            .pl(px((left - 8.0).max(8.0))).text_size(px(12.0)).text_color(theme.muted)
            .children(animate.then(motion::theme_frame))
            .child(div().flex_none().px(px(8.0)).flex().items_center().gap(px(6.0))
                .children(dirty.then(|| div().size(px(6.0)).rounded_full().bg(theme.accent)))
                .child(if dirty { "有未保存的修改" } else { "已保存" }))
            .child(separator())
            .child(self.status_trigger(Panel::Stats, label, 1.0, theme, cx))
            .child(separator())
            .child(self.status_trigger(Panel::Zoom, format!("{percent}%"), scale, theme, cx));
        Some(controls.into_any_element())
    }

    fn status_trigger(&self, panel: Panel, label: String, scale: f32, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let index = panel.index();
        let hover = self.status_ui.motion[index].value();
        let locale = ToolbarLocale::current(cx);
        let mono = crate::theme::font_for(crate::theme::Family::Mono, false, false).family;
        let bounds = self.status_ui.anchors[index].clone();
        // 外层保留稳定命中区域，内层缩放不会挤动相邻按钮或重新定位弹层。
        div().id(("status-trigger", index)).relative()
            .role(gpui::Role::Button).aria_label(locale.text(if index == 0 { "status_zoom" } else { "status_word_count" }))
            .aria_expanded(self.status_ui.open == Some(panel))
            .h(px(24.0)).px(px(8.0)).flex_none().rounded(px(4.0)).cursor_pointer()
            .font_family(mono).text_size(px(12.0))
            .track_focus(&self.status_ui.triggers[index]).tab_index(0)
            .focus_visible(|style| style.shadow(vec![BoxShadow { inset: false,
                color: theme.accent.opacity(0.45), offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0), spread_radius: px(3.0) }]))
            .tooltip(tooltip(locale.text(if index == 0 { "status_zoom" } else { "status_word_count" })))
            .child(div().opacity(0.0).whitespace_nowrap().child(label.clone()))
            .child(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center()
                .child(div().w(relative(scale)).h(px(24.0 * scale)).flex().items_center().justify_center()
                    .rounded(px(4.0)).bg(theme.hover.opacity(hover)).text_size(px(12.0 * scale))
                    .text_color(motion::color(theme.muted, theme.fg, hover)).whitespace_nowrap().child(label)))
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
            .on_mouse_down_out(cx.listener(|workspace, event: &gpui::MouseDownEvent, _, cx| {
                if !workspace.status_ui.dragging && !workspace.status_ui.anchors.iter()
                    .any(|anchor| anchor.get().is_some_and(|bounds| bounds.contains(&event.position))) {
                    workspace.close_status(cx); cx.notify();
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
                .child(div().p(sp(12.0)).flex_none()
                    .child(status_slider("status-zoom-slider", (value - 80) as f32 / 80.0,
                        self.status_ui.dragging, 154.0 * scale, theme, on_change, self.status_ui.slider_motion.value())
                        .role(gpui::Role::Slider).aria_label(locale.text("status_zoom"))
                        .aria_numeric_value(value as f64).aria_min_numeric_value(80.0).aria_max_numeric_value(160.0).aria_numeric_value_step(5.0)
                        .tab_index(0).tooltip(tooltip(locale.text("status_zoom")))
                        .focus_visible(|style| style.bg(theme.accent.opacity(0.07)))
                        .on_hover(cx.listener(|workspace, hovered, _, cx| {
                            workspace.status_ui.slider_hover = *hovered; cx.notify();
                        }))));
        } else {
            let document = self.document()?;
            let stats = document.read(cx).stats().cloned();
            let active = cx.global::<Settings>().stats_metric;
            panel = panel.child(div().flex_none().pt(sp(11.0)).pb(sp(9.0)).px(sp(12.0))
                .border_b_1().border_color(theme.border).text_size(sp(14.0)).font_weight(FontWeight::SEMIBOLD)
                .child(locale.text("status_stats")));
            if document.read(cx).stats_failed() {
                panel = panel.child(div().px(sp(12.0)).py(sp(6.0)).text_color(theme.muted)
                    .child(locale.text("status_unavailable")));
            }
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
