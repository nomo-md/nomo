//! 文档大纲浮层：界面只消费结构索引的有限页，不持有正文。
use super::*;
use crate::editor_control::document_host::OutlineSnapshot;

const OUTLINE_ROW_HEIGHT: f32 = 32.0;
const OUTLINE_ROW_CONTENT_HEIGHT: f32 = 24.0;
const OUTLINE_PADDING: f32 = 14.0;
const MAX_ANIMATED_ROWS: usize = 64;

#[derive(Clone)]
struct OutlineVisualRow {
    key: String,
    position: usize,
    level: u8,
    text: String,
    collapsed: bool,
    has_children: bool,
    active: bool,
}

#[derive(Clone)]
struct ProjectionBefore {
    revision: u64,
    generation: u64,
    rows: Rc<Vec<OutlineVisualRow>>,
    offset_y: f32,
    list_height: f32,
}

#[derive(Clone)]
struct ProjectionMotion {
    before: ProjectionBefore,
    started: Instant,
    duration: Duration,
}

impl ProjectionMotion {
    fn progress(&self) -> f32 {
        motion::ease((self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0))
    }
    fn running(&self) -> bool { self.started.elapsed() < self.duration }
}

struct RowHover { key: String, motion: ScalarMotion }

#[derive(Clone)]
struct ActiveMotion { from: Option<u64>, to: Option<u64>, started: Instant, duration: Duration }
impl ActiveMotion {
    fn progress(&self) -> f32 { motion::ease((self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0)) }
    fn running(&self) -> bool { self.started.elapsed() < self.duration }
    fn weight(&self, byte: u64) -> f32 {
        if self.to == Some(byte) { self.progress() }
        else if self.from == Some(byte) { 1.0 - self.progress() }
        else { 0.0 }
    }
}

struct PanelDrag { start: gpui::Point<gpui::Pixels>, origin: (f32, f32), current: (f32, f32), moved: bool }
struct OutlineMenu { position: gpui::Point<gpui::Pixels>, heading: Option<Heading>, revision: u64 }
struct SectionDrag {
    start: gpui::Point<gpui::Pixels>, pointer: gpui::Point<gpui::Pixels>, source: Heading,
    target: Option<Heading>, placement: crate::text_document::markdown_session::OutlinePlacement,
    revision: u64, moving: bool, valid: bool, generation: u64,
}
pub(super) struct OutlineUi {
    pub bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    focus: FocusHandle,
    keyboard_focus_visible: bool,
    selected: usize,
    panel_height: f32,
    list_height: f32,
    document_id: Option<String>,
    showing: bool,
    visibility: ScalarMotion,
    exit_snapshot: Option<OutlineSnapshot>,
    pending_projection: Option<ProjectionBefore>,
    projection_motion: Option<ProjectionMotion>,
    active_target: Option<u64>,
    active_motion: Option<ActiveMotion>,
    row_hovers: Vec<RowHover>,
    bulk_hover: ScalarMotion,
    panel_drag: Option<PanelDrag>,
    menu: Option<OutlineMenu>,
    section_drag: Option<SectionDrag>,
    auto_scroll: Option<Task<()>>,
    suppress_click: bool,
    drag_generation: u64,
    _activation: Subscription,
}
impl OutlineUi {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if !window.is_window_active() { workspace.outline_ui.panel_drag = None; workspace.outline_ui.section_drag = None; workspace.outline_ui.auto_scroll = None; workspace.outline_ui.menu = None; cx.notify(); }
        });
        Self { bounds: Rc::new(Cell::new(None)), focus: cx.focus_handle(), keyboard_focus_visible: true, selected: 0, panel_height: 92.0, list_height: 24.0, document_id: None,
            showing: false, visibility: ScalarMotion::default(), exit_snapshot: None, pending_projection: None,
            projection_motion: None, active_target: None, active_motion: None, row_hovers: Vec::new(), bulk_hover: ScalarMotion::default(),
            panel_drag: None, menu: None, section_drag: None, auto_scroll: None, suppress_click: false, drag_generation: 0, _activation: activation }
    }

    pub(super) fn clear_document(&mut self) {
        self.document_id = None; self.showing = false; self.visibility.reset();
        self.exit_snapshot = None; self.pending_projection = None; self.projection_motion = None;
        self.active_target = None; self.active_motion = None; self.row_hovers.clear(); self.bulk_hover.reset();
        self.panel_drag = None; self.section_drag = None; self.auto_scroll = None; self.menu = None;
    }

    pub(super) fn set_reduced_motion(&mut self, reduced: bool) {
        self.visibility.retime(reduced, Duration::from_millis(160));
        self.bulk_hover.retime(reduced, Duration::from_millis(140));
        for entry in &mut self.row_hovers { entry.motion.retime(reduced, Duration::from_millis(160)); }
        if reduced {
            if let Some(animation) = &mut self.projection_motion { animation.duration = animation.duration.min(Duration::from_millis(90)); }
            if let Some(animation) = &mut self.active_motion { animation.duration = animation.duration.min(Duration::from_millis(90)); }
        }
    }
}

fn split_number(text: &str) -> (&str, &str) {
    let bytes = text.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) { return ("", text); }
    let mut end = 0;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') { end += 1; }
    while end < bytes.len() && bytes[end].is_ascii_whitespace() { end += 1; }
    (&text[..end], &text[end..])
}

fn fading_outline_row(row: &OutlineVisualRow, theme: &Theme) -> AnyElement {
    let color = if row.active { theme.accent_strong } else { theme.muted };
    let arrow = if row.has_children {
        icon(IconName::ChevronDown, 13.0, color).with_transformation(Transformation::rotate(radians(
            if row.collapsed { -std::f32::consts::FRAC_PI_2 } else { 0.0 }
        ))).into_any_element()
    } else { div().into_any_element() };
    let (number, rest) = split_number(&row.text);
    div().w_full().h(px(OUTLINE_ROW_CONTENT_HEIGHT)).pl(px((row.level.saturating_sub(1)) as f32 * 16.0))
        .flex().items_center().gap(px(7.0)).rounded(px(4.0)).overflow_hidden()
        .text_size(px(14.0)).text_color(color)
        .when(row.active, |el| el.bg(theme.accent.opacity(0.07)).font_weight(FontWeight::SEMIBOLD))
        .child(div().w(px(16.0)).h(px(24.0)).flex_none().flex().items_center().justify_center().child(arrow))
        .child(div().flex().flex_1().min_w_0().overflow_hidden().whitespace_nowrap()
            .child(div().flex_none().max_w(px(55.0)).truncate().opacity(0.45).child(number.to_string()))
            .child(div().flex_1().min_w_0().truncate().child(rest.to_string())))
        .into_any_element()
}

impl Workspace {
    fn begin_outline_projection_change(&mut self, cx: &mut Context<Self>) -> Option<Entity<DocumentHost>> {
        if !self.outline_ui.showing { return None; }
        let document = self.document()?;
        let snapshot = document.read(cx).outline_snapshot();
        if snapshot.pending || snapshot.error.is_some() { return None; }
        let scroll = self.outline_scroll.0.borrow().base_handle.clone();
        let top = scroll.logical_scroll_top().0;
        let count = ((self.outline_ui.list_height / OUTLINE_ROW_HEIGHT).ceil() as usize + 3).min(MAX_ANIMATED_ROWS);
        let from = top.saturating_sub(1);
        let rows = (from..from.saturating_add(count).min(snapshot.count)).filter_map(|index| snapshot.row(index)).map(|heading| OutlineVisualRow {
            key: heading.key(), position: heading.position, level: heading.level,
            text: heading.text.chars().take(256).collect(), collapsed: heading.collapsed,
            has_children: heading.has_children, active: snapshot.active_byte == Some(heading.byte),
        }).collect();
        self.outline_ui.pending_projection = Some(ProjectionBefore { revision: snapshot.revision, generation: snapshot.view_generation,
            rows: Rc::new(rows), offset_y: f32::from(scroll.offset().y), list_height: self.outline_ui.list_height });
        Some(document)
    }

    fn toggle_outline_heading(&mut self, heading: &Heading, cx: &mut Context<Self>) {
        if let Some(document) = self.begin_outline_projection_change(cx) {
            document.update(cx, |host, cx| host.set_outline_fold(heading, cx));
        }
    }

    fn set_outline_level_animated(&mut self, level: u8, cx: &mut Context<Self>) {
        if let Some(document) = self.begin_outline_projection_change(cx) {
            document.update(cx, |host, cx| host.set_outline_level(level, cx));
        }
    }

    fn set_outline_row_hover(&mut self, key: String, hovered: bool, cx: &mut Context<Self>) {
        let reduced = self.explorer.reduce_motion;
        let duration = Duration::from_millis(160);
        if hovered {
            for entry in &mut self.outline_ui.row_hovers {
                if entry.key != key { entry.motion.set(false, reduced, duration); }
            }
        }
        if let Some(entry) = self.outline_ui.row_hovers.iter_mut().find(|entry| entry.key == key) {
            entry.motion.set(hovered, reduced, duration);
        } else if hovered {
            if self.outline_ui.row_hovers.len() >= 4 { self.outline_ui.row_hovers.remove(0); }
            let mut state = ScalarMotion::default(); state.set(true, reduced, duration);
            self.outline_ui.row_hovers.push(RowHover { key, motion: state });
        }
        cx.notify();
    }

    pub(super) fn outline_position(&self, window: &Window, cx: &App, height: f32) -> (f32, f32) {
        let bounds = self.outline_ui.bounds.get();
        let width = bounds.map_or(f32::from(window.viewport_size().width), |b| f32::from(b.size.width));
        let area_height = bounds.map_or(f32::from(window.viewport_size().height), |b| f32::from(b.size.height));
        let top = bounds.map_or(168.0, |b| 168.0 - f32::from(b.origin.y));
        let saved = cx.global::<Settings>().value("outlinePanelPosition");
        let stored = if saved.get("positioned").and_then(|v| v.as_bool()).unwrap_or(false) {
            saved.get("left").and_then(|v| v.as_f64()).zip(saved.get("top").and_then(|v| v.as_f64()))
                .map(|(x,y)| (x as f32,y as f32)).filter(|(x,y)| x.is_finite() && y.is_finite())
        } else { None };
        let (left, top) = self.outline_ui.panel_drag.as_ref().filter(|d| d.moved).map(|d| d.current)
            .or(stored).unwrap_or((width - 220.0 - (width * 0.035).clamp(32.0, 160.0), top));
        (left.clamp(8.0, (width - 228.0).max(8.0)), top.clamp(8.0, (area_height - height - 8.0).max(8.0)))
    }
    pub(super) fn render_outline_panel(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let document = self.document()?;
        let document_id = document.read(cx).status().document_id.clone();
        if self.outline_ui.document_id.as_ref() != Some(&document_id) {
            self.outline_ui.document_id = Some(document_id);
            self.outline_ui.selected = 0;
            self.outline_ui.panel_drag = None;
            self.outline_ui.section_drag = None;
            self.outline_ui.auto_scroll = None;
            self.outline_ui.menu = None;
            self.outline_ui.showing = false;
            self.outline_ui.visibility.reset();
            self.outline_ui.exit_snapshot = None;
            self.outline_ui.pending_projection = None;
            self.outline_ui.projection_motion = None;
            self.outline_ui.active_target = None;
            self.outline_ui.active_motion = None;
            self.outline_ui.row_hovers.clear();
            self.outline_ui.bulk_hover.reset();
            self.outline_scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        let settings = cx.global::<Settings>().clone();
        let width = self.outline_ui.bounds.get().map_or(f32::from(window.viewport_size().width), |b| f32::from(b.size.width));
        let visible = settings.show_outline && width > 900.0;
        let reduced = self.explorer.reduce_motion;
        if visible != self.outline_ui.showing {
            self.outline_ui.exit_snapshot = if !visible && width > 900.0 { Some(document.read(cx).outline_snapshot()) } else { None };
            self.outline_ui.showing = visible;
            if width <= 900.0 { self.outline_ui.visibility.reset(); }
            else { self.outline_ui.visibility.set(visible, reduced, Duration::from_millis(160)); }
            if !visible {
                self.outline_ui.panel_drag = None; self.outline_ui.section_drag = None;
                self.outline_ui.auto_scroll = None; self.outline_ui.menu = None;
                self.outline_ui.pending_projection = None; self.outline_ui.projection_motion = None; self.outline_ui.active_motion = None;
                self.outline_ui.row_hovers.clear(); self.outline_ui.bulk_hover.reset();
            }
        }
        if width <= 900.0 { self.outline_ui.visibility.reset(); self.outline_ui.exit_snapshot = None; }
        document.update(cx, |host, cx| host.set_outline_enabled(visible, settings.outline_level, cx));
        if !visible && !self.outline_ui.visibility.running() {
            self.outline_ui.exit_snapshot = None;
            return None;
        }
        let snapshot = if visible { document.read(cx).outline_snapshot() } else { self.outline_ui.exit_snapshot.clone()? };
        if !snapshot.has_expandable { self.outline_ui.bulk_hover.reset(); }
        if let Some(before) = self.outline_ui.pending_projection.take() {
            if snapshot.pending && before.revision == snapshot.revision {
                self.outline_ui.pending_projection = Some(before);
            } else if before.revision == snapshot.revision && before.generation != snapshot.view_generation && snapshot.error.is_none() {
                self.outline_ui.projection_motion = Some(ProjectionMotion { before, started: Instant::now(),
                    duration: Duration::from_millis(if reduced { 90 } else { 180 }) });
            }
        }
        if self.outline_ui.projection_motion.as_ref().is_some_and(|animation| !animation.running() || animation.before.revision != snapshot.revision) {
            self.outline_ui.projection_motion = None;
        }
        if visible && self.outline_ui.active_target != snapshot.active_byte {
            self.outline_ui.active_motion = Some(ActiveMotion { from: self.outline_ui.active_target, to: snapshot.active_byte,
                started: Instant::now(), duration: Duration::from_millis(if reduced { 90 } else { 160 }) });
            self.outline_ui.active_target = snapshot.active_byte;
        }
        if self.outline_ui.active_motion.as_ref().is_some_and(|animation| !animation.running()) { self.outline_ui.active_motion = None; }
        self.outline_ui.row_hovers.retain(|entry| entry.motion.running() || entry.motion.value() > 0.001);
        if self.outline_ui.visibility.running() || self.outline_ui.projection_motion.is_some() || self.outline_ui.active_motion.is_some()
            || self.outline_ui.bulk_hover.running() || self.outline_ui.row_hovers.iter().any(|entry| entry.motion.running()) {
            window.request_animation_frame();
        }
        self.outline_ui.selected = self.outline_ui.selected.min(snapshot.count.saturating_sub(1));
        let scroll = self.outline_scroll.0.borrow().base_handle.clone();
        let top_index = scroll.logical_scroll_top().0;
        let scroll_y = f32::from(scroll.offset().y);
        if visible { document.update(cx, |host, cx| host.request_outline_page(top_index, cx)); }
        let count = snapshot.count + usize::from(!snapshot.complete && snapshot.count > 0);
        let area_height = self.outline_ui.bounds.get().map_or(f32::from(window.viewport_size().height), |b| f32::from(b.size.height));
        let notice_height = if snapshot.error.is_some() || snapshot.incomplete { 40.0 } else { 0.0 };
        let positioned = settings.value("outlinePanelPosition").get("positioned").and_then(|v| v.as_bool()).unwrap_or(false);
        let max_panel_height = if positioned { area_height - 16.0 } else { f32::from(window.viewport_size().height) - 260.0 };
        // 虚拟列表的每格固定 32px；末行自带 8px 空隙，避免不足一格造成多余滚动。
        let bottom_padding = if count == 0 { OUTLINE_PADDING } else { OUTLINE_PADDING - 8.0 };
        let max_list_height = (max_panel_height.min(area_height - 16.0) - 54.0 - bottom_padding - notice_height).max(24.0);
        let target_list_height = (count as f32 * OUTLINE_ROW_HEIGHT).clamp(24.0, max_list_height);
        let animation = self.outline_ui.projection_motion.clone();
        let progress = animation.as_ref().map_or(1.0, ProjectionMotion::progress);
        let list_height = animation.as_ref().map_or(target_list_height, |animation|
            (animation.before.list_height + (target_list_height - animation.before.list_height) * progress).clamp(24.0, max_list_height));
        self.outline_ui.list_height = list_height;
        let height = list_height + 54.0 + bottom_padding + notice_height;
        self.outline_ui.panel_height = height;
        let (left, mut top) = self.outline_position(window, cx, height);
        let visibility = self.outline_ui.visibility.value();
        if !reduced { top -= (1.0 - visibility) * 4.0; }
        let active = snapshot.active_byte;
        let active_motion = self.outline_ui.active_motion.clone();
        let revision = snapshot.revision;
        let focused = self.outline_ui.focus.is_focused(window);
        if !focused { self.outline_ui.keyboard_focus_visible = true; }
        let selected = (focused && self.outline_ui.keyboard_focus_visible).then_some(self.outline_ui.selected);
        let drag_visual = self.outline_ui.section_drag.as_ref().filter(|d|d.moving).map(|d| (d.source.position,d.target.as_ref().map(|h|h.position),d.placement,d.valid));
        let row_hovers = Rc::new(self.outline_ui.row_hovers.iter().map(|entry| (entry.key.clone(), entry.motion.value())).collect::<Vec<_>>());
        let page = snapshot.clone();
        let theme = theme.clone();
        let rows: AnyElement = if count == 0 {
            let text: String = if snapshot.error.is_some() { "大纲暂不可用".into() } else if snapshot.pending { "正在加载大纲…".into() } else if snapshot.incomplete { "结构索引不完整，无法确认标题".into() } else { "当前文档还没有标题".into() };
            div().w_full().h(px(list_height)).px(px(OUTLINE_PADDING)).text_size(px(13.0)).text_color(theme.muted).child(text).into_any_element()
        } else {
            let ghosts: Vec<AnyElement> = animation.as_ref().map(|animation| {
                let end = top_index.saturating_add(((list_height / OUTLINE_ROW_HEIGHT).ceil() as usize + 3).min(MAX_ANIMATED_ROWS)).min(snapshot.count);
                let start = top_index.saturating_sub(1).min(end);
                if (start..end).any(|index| snapshot.row(index).is_none()) { return Vec::new(); }
                let keys: Vec<String> = (start..end).filter_map(|index| snapshot.row(index)).map(Heading::key).collect();
                animation.before.rows.iter().filter(|row| !keys.contains(&row.key)).filter_map(|row| {
                    let y = row.position as f32 * OUTLINE_ROW_HEIGHT + animation.before.offset_y - 3.0 * progress;
                    (y > -OUTLINE_ROW_HEIGHT && y < list_height).then(|| div().absolute().left_0().right_0().top(px(y))
                        .opacity(1.0 - progress).child(fading_outline_row(row, &theme)).into_any_element())
                }).collect()
            }).unwrap_or_default();
            let list = uniform_list("outline", count, {
                let theme = theme.clone();
                let animation = animation.clone();
                let active_motion = active_motion.clone();
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range.map(|index| {
                        let Some(heading) = page.row(index).cloned() else {
                            return div().w_full().h(px(OUTLINE_ROW_HEIGHT)).text_color(theme.muted).child("加载中…").into_any_element();
                        };
                        let active = active == Some(heading.byte);
                        let active_weight = active_motion.as_ref().map_or(if active { 1.0 } else { 0.0 }, |animation| animation.weight(heading.byte));
                        let key = heading.key();
                        let old = animation.as_ref().and_then(|animation| animation.before.rows.iter().find(|row| row.key == key));
                        let (offset, opacity) = if let Some(animation) = &animation {
                            if let Some(old) = old {
                                let delta = (old.position as f32 - index as f32) * OUTLINE_ROW_HEIGHT + animation.before.offset_y - scroll_y;
                                (delta.clamp(-list_height, list_height) * (1.0 - progress), 1.0)
                            } else { (-3.0 * (1.0 - progress), progress) }
                        } else { (0.0, 1.0) };
                        let hover = row_hovers.iter().find(|(hovered,_)| hovered == &key).map_or(0.0, |(_, value)| *value);
                        let color = motion::color(motion::color(theme.muted, theme.accent_strong, hover), theme.accent_strong, active_weight);
                        let title = if heading.text.is_empty() { "（空标题）".to_string() } else { heading.text.clone() };
                        let title_hint = format!("{} · 第 {} 行", title, heading.line);
                        let (number, rest) = split_number(&title);
                        let number = number.to_string(); let rest = rest.to_string();
                        let fold_heading = heading.clone(); let goto_heading = heading.clone(); let menu_heading = heading.clone();
                        let drag_heading = heading.clone();
                        let hover_key = key.clone();
                        let drop = drag_visual.filter(|(_, target, _, _)| *target == Some(index));
                        let toggle = if heading.has_children {
                            let target = if heading.collapsed { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
                            let angle = old.map_or(target, |old| {
                                let from = if old.collapsed { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
                                let arrow_progress = animation.as_ref().map_or(1.0, |animation| {
                                    let duration = if animation.duration <= Duration::from_millis(90) { 90.0 } else { 150.0 };
                                    motion::ease((animation.started.elapsed().as_secs_f32() * 1000.0 / duration).min(1.0))
                                });
                                from + (target - from) * arrow_progress
                            });
                            div().id((if heading.collapsed { "outline-fold-collapsed" } else { "outline-fold-expanded" }, index)).w(px(16.0)).h(px(24.0)).flex_none().flex().items_center().justify_center()
                                .cursor_pointer().tooltip(tooltip(if heading.collapsed { "展开标题" } else { "收起标题" }))
                                .child(icon(IconName::ChevronDown, 13.0, color).with_transformation(Transformation::rotate(radians(angle))))
                                .on_mouse_down(MouseButton::Left, cx.listener(|workspace, _, _, cx| { workspace.outline_ui.keyboard_focus_visible = false; cx.stop_propagation(); cx.notify(); }))
                                .on_click(cx.listener(move |workspace, _, _, cx| { cx.stop_propagation(); workspace.toggle_outline_heading(&fold_heading, cx); }))
                                .into_any_element()
                        } else { div().w(px(16.0)).h(px(24.0)).flex_none().into_any_element() };
                        let row = div().id(("outline-row", index)).relative().top(px(offset)).w_full().h(px(OUTLINE_ROW_CONTENT_HEIGHT))
                            .pl(px((heading.level.saturating_sub(1)) as f32 * 16.0))
                            .flex().items_center().gap(px(7.0)).rounded(px(4.0)).overflow_hidden().cursor_pointer()
                            .text_size(px(14.0)).text_color(color).opacity(opacity)
                            .when(active_weight > 0.0, |el| el.bg(theme.accent.opacity(0.07 * active_weight)))
                            .when(active, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .when(selected == Some(index), |el| el.border_1().border_color(theme.accent))
                            .when(drag_visual.is_some_and(|(source,_,_,_)| source == index), |el| el.opacity(0.38))
                            .when(drop.is_some_and(|(_,_,_,valid)| !valid), |el| el.bg(theme.callout_caution.opacity(0.12)).border_1().border_color(theme.callout_caution))
                            .when(drop.is_some_and(|(_,_,place,valid)| valid && matches!(place,crate::text_document::markdown_session::OutlinePlacement::Inside)), |el| el.bg(theme.accent.opacity(0.15)).border_1().border_color(theme.accent))
                            .child(toggle).child(div().id(("outline-title",index)).flex().items_center().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().tooltip(tooltip(title_hint))
                                .child(div().flex_none().max_w(px(55.0)).truncate().opacity(0.45).child(number)).child(div().flex_1().min_w_0().truncate().child(rest)))
                            .on_hover(cx.listener(move |workspace, hovered: &bool, _, cx| workspace.set_outline_row_hover(hover_key.clone(), *hovered, cx)))
                            .on_click(cx.listener(move |workspace, _, _, cx| {
                                if !workspace.outline_ui.showing { return; }
                                if workspace.outline_ui.suppress_click { workspace.outline_ui.suppress_click=false; return; }
                                workspace.outline_ui.selected = index;
                                if let Some(doc) = workspace.document() { doc.update(cx, |host, cx| host.reveal_outline_heading(goto_heading.byte, revision, cx)); }
                            }))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |workspace, event: &gpui::MouseDownEvent, window, _cx| {
                                if !workspace.outline_ui.showing { return; }
                                workspace.outline_ui.keyboard_focus_visible = false;
                                window.focus(&workspace.outline_ui.focus);
                                workspace.outline_ui.drag_generation += 1;
                                workspace.outline_ui.section_drag = Some(SectionDrag { start:event.position, pointer:event.position, source:drag_heading.clone(), target:None,
                                    placement:crate::text_document::markdown_session::OutlinePlacement::Before, revision, moving:false, valid:false, generation:workspace.outline_ui.drag_generation });
                            }))
                            .on_mouse_down(MouseButton::Right, cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| {
                                if !workspace.outline_ui.showing { return; }
                                workspace.outline_ui.keyboard_focus_visible = false;
                                cx.stop_propagation(); workspace.outline_ui.menu = Some(OutlineMenu { position: event.position, heading: Some(menu_heading.clone()), revision }); cx.notify();
                            }));
                        div().relative().w_full().h(px(OUTLINE_ROW_HEIGHT)).child(row)
                            .when(drop.is_some_and(|(_,_,place,valid)| valid && matches!(place,crate::text_document::markdown_session::OutlinePlacement::Before)), |el|
                                el.child(div().absolute().left(px(3.0)).right(px(3.0)).top(px(offset - 5.0)).h(px(2.0)).rounded_full().bg(theme.accent)))
                            .when(drop.is_some_and(|(_,_,place,valid)| valid && matches!(place,crate::text_document::markdown_session::OutlinePlacement::After)), |el|
                                el.child(div().absolute().left(px(3.0)).right(px(3.0)).top(px(offset + OUTLINE_ROW_CONTENT_HEIGHT + 3.0)).h(px(2.0)).rounded_full().bg(theme.accent)))
                            .into_any_element()
                    }).collect()
                })
            }).track_scroll(self.outline_scroll.clone()).w_full().h(px(list_height))
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()));
            div().w_full().px(px(OUTLINE_PADDING)).child(div().relative().w_full().h(px(list_height)).overflow_hidden()
                .child(list).children(ghosts)).into_any_element()
        };
        let bulk_expand = snapshot.has_collapsed;
        let bulk_hover = self.outline_ui.bulk_hover.value();
        let bulk_border = motion::color(theme.border, motion::color(theme.border, theme.accent, 0.38), bulk_hover);
        let bulk = div().id(if bulk_expand { "outline-bulk-expand" } else { "outline-bulk-default" }).size(px(28.0)).flex().items_center().justify_center().rounded(px(4.0))
            .border_1().border_color(bulk_border).bg(theme.accent.opacity(0.08 * bulk_hover)).cursor_pointer()
            .tooltip(tooltip(if bulk_expand { "全部展开" } else { "恢复默认展开层级" }))
            .child(icon(if bulk_expand { IconName::ChevronsUpDown } else { IconName::ChevronsDownUp }, 15.0,
                motion::color(theme.muted, theme.accent_strong, bulk_hover)))
            .on_hover(cx.listener(|workspace, hovered: &bool, _, cx| {
                workspace.outline_ui.bulk_hover.set(*hovered, workspace.explorer.reduce_motion, Duration::from_millis(140)); cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|workspace, _, _, cx| { workspace.outline_ui.keyboard_focus_visible = false; cx.stop_propagation(); cx.notify(); }))
            .on_click(cx.listener(move |workspace, _, _, cx| { cx.stop_propagation(); let level = if bulk_expand { 6 } else { cx.global::<Settings>().outline_level }; workspace.set_outline_level_animated(level,cx); }));
        let header = div().h(px(54.0)).px(px(14.0)).flex().items_center().justify_between()
            .child(div().text_size(px(14.0)).font_weight(FontWeight::EXTRA_BOLD).text_color(theme.heading).child("文档大纲"))
            .when(snapshot.has_expandable, |el|el.child(bulk))
            .on_mouse_down(MouseButton::Left, cx.listener(move |workspace, event: &gpui::MouseDownEvent, window, cx| {
                if !workspace.outline_ui.showing { return; }
                workspace.outline_ui.keyboard_focus_visible = false;
                cx.stop_propagation(); window.focus(&workspace.outline_ui.focus); let origin = workspace.outline_position(window, cx, height);
                workspace.outline_ui.panel_drag = Some(PanelDrag { start: event.position, origin, current: origin, moved: false });
            }));
        let notice = if let Some(error) = snapshot.error.clone() {
            Some(div().px(px(14.0)).pb(px(6.0)).text_size(px(11.0)).text_color(theme.callout_caution)
                .child(error).child(div().id("outline-retry").cursor_pointer().text_color(theme.accent_strong).child("重试")
                    .on_click(cx.listener(|workspace, _, _, cx| { if let Some(doc) = workspace.document() { doc.update(cx, |host, cx| host.retry_outline(cx)); } }))))
        } else { snapshot.incomplete.then(|| div().px(px(14.0)).pb(px(6.0)).text_size(px(11.0)).text_color(theme.muted).child("部分结构尚未识别")) };
        let shadows = vec![
            gpui::BoxShadow { color: gpui::rgba(if theme.dark { 0x00000033 } else { 0x16202c0f }).into(),
                offset: gpui::point(px(0.0), px(0.0)), blur_radius: px(24.0), spread_radius: px(0.0) },
            gpui::BoxShadow { color: gpui::rgba(if theme.dark { 0x00000029 } else { 0x16202c0d }).into(),
                offset: gpui::point(px(0.0), px(6.0)), blur_radius: px(16.0), spread_radius: px(0.0) },
        ];
        Some(div().id("outline-card").key_context("Outline").absolute().left(px(left)).top(px(top)).w(px(220.0)).rounded(px(8.0))
            .opacity(visibility).bg(motion::color(theme.surface, theme.app_bg, 0.04)).border_1().border_color(theme.border.opacity(0.82))
            .shadow(shadows).occlude().track_focus(&self.outline_ui.focus).tab_index(0)
            .on_mouse_move(cx.listener(|workspace, event: &gpui::MouseMoveEvent, _, cx| workspace.outline_mouse_move(event, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |workspace, event: &gpui::MouseDownEvent, _, cx| { if !workspace.outline_ui.showing { return; } workspace.outline_ui.keyboard_focus_visible = false; cx.stop_propagation(); workspace.outline_ui.menu = Some(OutlineMenu { position: event.position, heading: None, revision }); cx.notify(); }))
            .on_key_down(cx.listener(|workspace, event: &gpui::KeyDownEvent, _, cx| workspace.outline_key(event, cx)))
            .child(header).children(notice).child(rows).pb(px(bottom_padding)).into_any_element())
    }

    fn outline_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" { self.outline_ui.panel_drag = None; self.outline_ui.section_drag = None; self.outline_ui.auto_scroll = None; self.outline_ui.menu = None; cx.notify(); return; }
        if !self.outline_ui.showing { return; }
        let Some(doc) = self.document() else { return; };
        let state = doc.read(cx).outline_snapshot();
        match event.keystroke.key.as_str() {
            "up" => self.outline_ui.selected = self.outline_ui.selected.saturating_sub(1),
            "down" => self.outline_ui.selected = (self.outline_ui.selected + 1).min(state.count.saturating_sub(1)),
            "enter" => { if let Some(row) = state.row(self.outline_ui.selected) { let row=row.clone(); doc.update(cx, |host,cx|host.reveal_outline_heading(row.byte,state.revision,cx)); } },
            "left" | "right" | "space" => { if let Some(row) = state.row(self.outline_ui.selected) { if row.has_children && (event.keystroke.key == "space" || row.collapsed == (event.keystroke.key == "right")) { let row=row.clone(); self.toggle_outline_heading(&row,cx); } } },
            _ => return,
        }
        self.outline_ui.keyboard_focus_visible = true;
        self.outline_scroll.scroll_to_item(self.outline_ui.selected, ScrollStrategy::Top);
        cx.stop_propagation(); cx.notify();
    }
    pub(super) fn outline_mouse_move(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(drag) = self.outline_ui.panel_drag.as_mut() {
            let dx = f32::from(event.position.x - drag.start.x); let dy = f32::from(event.position.y - drag.start.y);
            if !drag.moved && dx.hypot(dy) < 5.0 { return; }
            drag.moved = true; drag.current = (drag.origin.0 + dx, drag.origin.1 + dy); cx.notify(); return;
        }
        let Some(drag) = self.outline_ui.section_drag.as_mut() else { return; };
        let dx=f32::from(event.position.x-drag.start.x); let dy=f32::from(event.position.y-drag.start.y);
        if !drag.moving && dx.hypot(dy)<5.0 { return; }
        drag.moving=true; drag.pointer=event.position;
        let source=drag.source.clone(); let revision=drag.revision; let previous=drag.target.as_ref().map(|h|h.position);
        let scroll=self.outline_scroll.0.borrow().base_handle.clone();
        let bounds=scroll.bounds(); let offset=scroll.offset();
        let local=f32::from(event.position.y-bounds.origin.y-offset.y);
        let index=if bounds.contains(&event.position) && local>=0.0 { Some((local/32.0).floor() as usize) } else {None};
        let target=self.document().and_then(|doc| { let state=doc.read(cx).outline_snapshot(); if state.revision==revision {index.and_then(|i|state.row(i).cloned())} else {None} });
        let fraction=(local/32.0).fract();
        let placement=if fraction<0.3 {crate::text_document::markdown_session::OutlinePlacement::Before}
            else if fraction>0.7 {crate::text_document::markdown_session::OutlinePlacement::After}
            else {crate::text_document::markdown_session::OutlinePlacement::Inside};
        let valid=target.as_ref().is_some_and(|target| !(source.byte<=target.byte && target.byte<source.to_byte) && !(matches!(placement,crate::text_document::markdown_session::OutlinePlacement::Inside) && target.level==6));
        let target_index=target.as_ref().map(|h|h.position);
        if let Some(drag)=self.outline_ui.section_drag.as_mut() {drag.target=target.clone();drag.placement=placement;drag.valid=valid;}
        if target_index != previous {
            self.outline_ui.drag_generation+=1; let generation=self.outline_ui.drag_generation;
            if let Some(drag)=self.outline_ui.section_drag.as_mut() {drag.generation=generation;}
            if let Some(heading)=target.filter(|h| h.has_children && h.collapsed) {
                cx.spawn(async move |this,cx| {
                    cx.background_executor().timer(Duration::from_millis(500)).await;
                    let _=this.update(cx,|workspace,cx| {
                        if workspace.outline_ui.section_drag.as_ref().is_some_and(|drag|drag.moving && drag.generation==generation && drag.target.as_ref().is_some_and(|h|h.key()==heading.key())) {
                            workspace.toggle_outline_heading(&heading,cx);
                        }
                    });
                }).detach();
            }
        }
        let edge=36.0; let y=f32::from(event.position.y-bounds.origin.y); let height=f32::from(bounds.size.height);
        if bounds.contains(&event.position) && (y<edge || y>height-edge) && self.outline_ui.auto_scroll.is_none() {
            let task=cx.spawn(async move |this,cx| loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let keep=this.update(cx,|workspace,cx|workspace.outline_scroll_tick(cx)).unwrap_or(false);
                if !keep {let _=this.update(cx,|workspace,_|workspace.outline_ui.auto_scroll=None);break;}
            });
            self.outline_ui.auto_scroll=Some(task);
        }
        cx.notify();
    }
    fn outline_scroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer)=self.outline_ui.section_drag.as_ref().filter(|drag|drag.moving).map(|drag|drag.pointer) else {return false;};
        let scroll=self.outline_scroll.0.borrow().base_handle.clone(); let bounds=scroll.bounds();
        if !bounds.contains(&pointer) {return false;}
        let y=f32::from(pointer.y-bounds.origin.y); let height=f32::from(bounds.size.height); let edge=36.0;
        let delta=if y<edge {-(edge-y)/edge*12.0} else if y>height-edge {(y-(height-edge))/edge*12.0} else {return false;};
        let old=scroll.offset(); let max=scroll.max_offset().height.max(px(0.0));
        let next=(old.y-px(delta)).clamp(-max,px(0.0));
        if f32::from(next-old.y).abs()<0.5 {return false;}
        scroll.set_offset(gpui::point(old.x,next));
        self.outline_mouse_move(&gpui::MouseMoveEvent {position:pointer,pressed_button:Some(MouseButton::Left),..Default::default()},cx);
        true
    }
    pub(super) fn outline_mouse_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.outline_ui.auto_scroll=None;
        if let Some(drag)=self.outline_ui.section_drag.take() {
            if drag.moving {
                self.outline_ui.suppress_click=true;
                let weak=cx.entity().downgrade();
                window.on_next_frame(move |_,cx| {let _=weak.update(cx,|workspace,_|workspace.outline_ui.suppress_click=false);});
                if let Some(target)=drag.target {
                    if drag.valid {if let Some(doc)=self.document() {doc.update(cx,|host,cx|host.move_outline_section(&drag.source,&target,drag.placement,drag.revision,cx));}}
                    else {self.notice=Some("不能把章节移动到自身、子章节或 H6 内部。".into());}
                }
                cx.notify();
            }
        }
        if let Some(drag) = self.outline_ui.panel_drag.take() {
            if drag.moved {
                let bounds = self.outline_ui.bounds.get();
                let width = bounds.map_or(f32::from(window.viewport_size().width), |b| f32::from(b.size.width));
                let height = bounds.map_or(f32::from(window.viewport_size().height), |b| f32::from(b.size.height));
                let left = drag.current.0.clamp(8.0, (width - 228.0).max(8.0));
                let top = drag.current.1.clamp(8.0, (height - self.outline_ui.panel_height - 8.0).max(8.0));
                Settings::set(cx, "outlinePanelPosition", serde_json::json!({"positioned":true,"left":left,"top":top}));
            }
            cx.notify();
        }
    }
    pub(super) fn cancel_outline_drag(&mut self, cx: &mut Context<Self>) {
        self.outline_ui.panel_drag = None;
        self.outline_ui.section_drag = None;
        self.outline_ui.auto_scroll = None;
        cx.notify();
    }
    pub(super) fn outline_mouse_up_out(&mut self, event: &gpui::MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.outline_ui.panel_drag.is_some() {
            let size = window.viewport_size();
            if gpui::bounds(gpui::point(px(0.0), px(0.0)), size).contains(&event.position) {
                self.outline_mouse_up(window, cx);
            } else { self.cancel_outline_drag(cx); }
        } else if self.outline_ui.section_drag.is_some() {
            let origin = self.outline_ui.bounds.get().map_or(gpui::point(px(0.0), px(0.0)), |bounds| bounds.origin);
            let (left, top) = self.outline_position(window, cx, self.outline_ui.panel_height);
            let card = gpui::bounds(origin + gpui::point(px(left), px(top)), gpui::size(px(220.0), px(self.outline_ui.panel_height)));
            if card.contains(&event.position) { self.outline_mouse_up(window, cx); }
            else { self.cancel_outline_drag(cx); }
        }
    }
    pub(super) fn reset_outline_position(&mut self, cx: &mut Context<Self>) {
        self.outline_ui.panel_drag = None;
        Settings::set(cx, "outlinePanelPosition", serde_json::json!({"positioned":false}));
    }
    pub(super) fn render_outline_drag_preview(&self, theme: &Theme) -> Option<AnyElement> {
        let drag=self.outline_ui.section_drag.as_ref().filter(|drag|drag.moving)?;
        let point=drag.pointer + gpui::point(px(12.0),px(12.0));
        Some(gpui::deferred(gpui::anchored().position(point).child(div().px(px(8.0)).py(px(4.0)).w(px(180.0))
            .rounded(px(4.0)).bg(theme.surface).border_1().border_color(theme.accent).shadow_lg()
            .overflow_hidden().whitespace_nowrap().text_color(theme.heading).child(drag.source.text.clone()))).with_priority(5).into_any_element())
    }
    pub(super) fn render_outline_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.outline_ui.menu.as_ref()?; let heading = menu.heading.clone(); let revision = menu.revision;
        let mut items = Vec::new();
        if heading.is_some() { items.extend([("jump", "跳转到标题"), ("copy", "复制标题")]); }
        if heading.as_ref().is_some_and(|h| h.has_children) { items.push(("fold", "展开／收起")); }
        items.extend([("expand", "全部展开"), ("collapse", "全部收起"), ("default", "恢复默认展开层级"), ("reset", "恢复大纲默认位置"), ("hide", "隐藏大纲")]);
        let mut panel = div().id("outline-context-menu").w(px(190.0)).py(px(5.0)).flex().flex_col().rounded(px(8.0))
            .bg(theme.surface).border_1().border_color(theme.border).shadow_lg().occlude()
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| { workspace.outline_ui.menu=None; cx.notify(); }));
        for (index, (action, label)) in items.into_iter().enumerate() {
            let heading=heading.clone();
            panel = panel.child(div().id(("outline-menu-item",index)).h(px(29.0)).px(px(14.0)).flex().items_center().cursor_pointer()
                .text_size(px(13.0)).text_color(theme.fg).hover(|style|style.bg(theme.hover)).child(label)
                .on_click(cx.listener(move |workspace, _, _, cx| {
                    workspace.outline_ui.menu=None; cx.stop_propagation();
                    match action {
                        "copy" => if let Some(h)=&heading { cx.write_to_clipboard(gpui::ClipboardItem::new_string(h.text.clone())); },
                        "jump" => if let (Some(doc),Some(h))=(workspace.document(),&heading) { doc.update(cx,|host,cx|host.reveal_outline_heading(h.byte,revision,cx)); },
                        "fold" => if let Some(h)=&heading { if h.has_children { workspace.toggle_outline_heading(h,cx); } },
                        "expand" | "collapse" => workspace.set_outline_level_animated(if action=="expand" {6}else{1},cx),
                        "default" => { let level = cx.global::<Settings>().outline_level; workspace.set_outline_level_animated(level,cx); },
                        "reset" => workspace.reset_outline_position(cx),
                        "hide" => Settings::set(cx,"outlineVisible",serde_json::json!(false)),
                        _ => {},
                    }
                    cx.notify();
                })));
        }
        Some(gpui::deferred(gpui::anchored().position(menu.position).snap_to_window_with_margin(px(8.0)).child(panel)).with_priority(3).into_any_element())
    }
}
