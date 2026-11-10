//! 文件树和主页共用的路径拖拽；预览只在一次手势内持有可见缩略图。
use crate::{theme::Theme, ui::{components::{icon, IconName}, motion, popup::reduced_motion}};
use gpui::{prelude::*, div, px, point, size, AnyElement, AvailableSpace, Bounds, Context, Pixels, SharedString, Window};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::{Arc, Mutex}, time::Instant};

#[derive(Clone)]
pub(crate) struct PathDrag {
    pub path: PathBuf,
    pub root: PathBuf,
    pub directory: bool,
    pub title: SharedString,
    pub visual: Arc<Mutex<Option<DragPose>>>,
}

#[derive(Clone, Copy)]
pub(crate) struct DragPose { pub bounds: Bounds<Pixels>, pub lift: f32, pub row: bool, pub settled: bool }

/// 只缓存可见项的几何。预绘制时拿到真实槽位，再接过浮层，避免坐标估算和一帧跳变。
pub(crate) struct ItemMotion {
    rect: motion::Spring<4>,
    lift: motion::Spring<1>,
    glow: motion::Spring<1>,
    seed: Option<DragPose>,
    initialized: bool,
    last: Option<Instant>,
}

impl Default for ItemMotion {
    fn default() -> Self {
        Self { rect: motion::Spring::layout([0.0; 4]), lift: motion::Spring::new([0.0], 170.0, 26.0),
            glow: motion::Spring::new([0.0], 22.0, 9.4), seed: None, initialized: false, last: None }
    }
}

impl ItemMotion {
    pub fn seed(&mut self, pose: DragPose, landed: bool) {
        self.seed = Some(pose);
        self.lift.p = [pose.lift]; self.lift.v = [0.0];
        self.glow.p = [if landed { 1.0 } else { 0.0 }]; self.glow.v = [0.0];
        self.last = None;
    }
    pub(crate) fn reflow_from_current(&mut self, scroll_delta: f32, now: Instant) {
        // 滚动补偿后仍从上一帧的屏幕位置开始，下一帧才推进到新槽位。
        self.rect.p[1] += scroll_delta;
        self.rect.target[1] += scroll_delta;
        self.last = Some(now);
    }

    pub(crate) fn snap_distant_reflow(&mut self, target: [f32; 2], max_travel: [f32; 2]) -> bool {
        if !self.initialized || self.seed.is_some() {
            return false;
        }
        let [x, y, ..] = self.rect.p;
        if (target[0] - x).abs() <= max_travel[0] && (target[1] - y).abs() <= max_travel[1] {
            return false;
        }
        self.rect.p[..2].copy_from_slice(&target);
        self.rect.target[..2].copy_from_slice(&target);
        self.rect.v[..2].fill(0.0);
        true
    }

    pub fn running(&self) -> bool { self.seed.is_some() || self.rect.running() || self.lift.running() || self.glow.running() }
    pub fn y(&self) -> f32 { self.rect.p[1] }

    pub fn draw(state: Rc<RefCell<Self>>, target: [f32; 2], child: AnyElement, row: bool, now: Instant, reduced: bool, theme: &Theme) -> AnyElement {
        let theme = theme.clone();
        gpui::canvas(move |bounds, window, cx| {
            let origin = bounds.origin - point(px(target[0]), px(target[1]));
            let mut state = state.borrow_mut();
            state.rect.target = [target[0], target[1], f32::from(bounds.size.width), f32::from(bounds.size.height)];
            if let Some(seed) = state.seed.take() {
                state.rect.p = [f32::from(seed.bounds.left() - origin.x), f32::from(seed.bounds.top() - origin.y),
                    f32::from(seed.bounds.size.width), f32::from(seed.bounds.size.height)];
                state.rect.v = [0.0; 4]; state.initialized = true;
            } else if !state.initialized { state.rect.snap(); state.initialized = true; }
            let dt = state.last.replace(now).map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f32());
            if reduced { state.rect.snap(); state.lift.snap(); state.glow.snap(); }
            else { state.rect.step(dt); state.lift.step(dt); state.glow.step(dt); }
            let [x, y, width, height] = state.rect.p;
            let lift = state.lift.p[0].clamp(0.0, 1.0);
            let glow = state.glow.p[0].clamp(0.0, 1.0);
            let moving = state.running();
            drop(state);
            let mut element = div().relative().w(px(width.max(1.0))).h(px(height.max(1.0)))
                .when(row && lift > 0.001, |el| el.rounded(px(4.0 + 5.0 * lift)).bg(gpui::Hsla { a: lift, ..theme.surface })
                    .shadow(vec![drag_shadow(&theme, lift)]))
                .child(child)
                .when(row && glow > 0.005, |el| el.child(div().absolute().inset_0().rounded(px(4.0)).bg(gpui::Hsla { a: 0.16 * glow, ..theme.sidebar_active })))
                .into_any_element();
            if lift > 0.01 { element = gpui::deferred(element).with_priority(1).into_any_element(); }
            element.layout_as_root(size(AvailableSpace::Definite(px(width.max(1.0))), AvailableSpace::Definite(px(height.max(1.0)))), window, cx);
            element.prepaint_at(origin + point(px(x), px(y)), window, cx);
            if moving { window.request_animation_frame(); }
            element
        }, |_, mut element, window, cx| element.paint(window, cx)).size_full().into_any_element()
    }
}

fn drag_shadow(theme: &Theme, lift: f32) -> gpui::BoxShadow {
    gpui::BoxShadow { inset: false, color: gpui::Hsla { a: theme.workspace_shadow_soft.a * lift, ..theme.workspace_shadow_soft },
        offset: point(px(0.0), px(10.0)), blur_radius: px(24.0), spread_radius: px(2.0) }
}

pub(crate) struct PathDragPreview {
    pub item: PathDrag,
    pub document: Option<super::home::document_card::DocumentCover>,
    pub document_motion: Option<Rc<RefCell<super::home::document_card::DocumentMotion>>>,
    pub folder: Option<super::home::folder_card::FolderCover>,
    pub folder_fx: Option<Rc<RefCell<super::home::folder_card::FolderFx>>>,
    pub size: Option<(f32, f32)>,
    pub row: Option<(f32, f32)>,
    pub row_icon: IconName,
    pub lift: motion::Spring<1>,
    pub last: Instant,
}

impl Render for PathDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::ui(cx);
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last).as_secs_f32(); self.last = now;
        self.lift.target = [1.0];
        if reduced_motion(cx) { self.lift.snap(); } else { self.lift.step(dt); }
        let lift = self.lift.p[0].max(0.0);
        if self.lift.running() { window.request_animation_frame(); }
        let visual = self.item.visual.clone();
        let row = self.row.is_some();
        let record = gpui::canvas(move |bounds, _, _| {
            *visual.lock().expect("drag geometry lock") = Some(DragPose { bounds, lift, row, settled: false });
        }, |_, _, _, _| {}).absolute().inset_0();
        if let Some((width, indent)) = self.row {
            return div().relative().left(px(-3.0 * lift)).top(px(-lift))
                .w(px(width + 6.0 * lift)).h(px(28.0)).pl(px(indent)).pr(px(4.0))
                .flex().items_center().gap(px(6.0)).text_size(px(14.0)).text_color(theme.fg)
                .bg(theme.surface).rounded(px(4.0 + 5.0 * lift)).shadow(vec![drag_shadow(&theme, lift.min(1.0))])
                .child(icon(self.row_icon, 13.0, theme.muted))
                .child(div().min_w(px(0.0)).flex_1().truncate().child(self.item.title.clone())).child(record).into_any_element();
        }
        let (width, height) = self.size.unwrap_or((220.0, 34.0));
        // 排序时保持卡片形态和抓取点；不再缩成用于吸入文件夹的小纸片。
        if let Some(folder) = &self.folder {
            let pose = self.folder_fx.as_ref().map(|fx| {
                let mut fx = fx.borrow_mut();
                let pose = fx.pose(now, reduced_motion(cx));
                if fx.running() { window.request_animation_frame(); } pose
            }).unwrap_or_default();
            return div().relative().top(px(-lift)).w(px(width)).h(px(height + 3.0))
                .child(div().relative().top(px(3.0)).child(folder.render(width, height, pose, &theme))).child(record).into_any_element();
        }
        let reduced = reduced_motion(cx);
        let (elevation, hint) = self.document_motion.as_ref().map(|motion| {
            let mut motion = motion.borrow_mut();
            motion.advance(now, reduced);
            if motion.running() {
                window.request_animation_frame();
            }
            motion.values()
        }).unwrap_or_default();
        div().relative().top(px(-lift)).w(px(width)).h(px(height + 3.0))
            .children(self.document.as_ref().map(|cover| cover.render(width, height, elevation, hint, reduced, &theme)))
            .child(record).into_any_element()
    }
}
