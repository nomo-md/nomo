//! 工作区卡片与活动标签共用的轮廓；这里只保存少量几何和当前路径。
use crate::theme::Theme;
use gpui::{
    Bounds, BoxShadow, ContentMask, Corners, EntityId, IntoElement, Path, PathBuilder, Pixels, ScrollHandle,
    Styled, Window, canvas, point, px, size,
};
use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};

pub const TAB_RAIL_HEIGHT: f32 = 30.0;
const TAB_HEIGHT: f32 = 30.0;
const OUTER_RADIUS: f32 = 12.0;
const RETURN_RADIUS: f32 = 8.0;
const SWITCH_DURATION: Duration = Duration::from_millis(180);

#[derive(Clone, Copy, Debug, PartialEq)]
struct TabRect {
    left: f32,
    width: f32,
}

impl TabRect {
    fn interpolate(self, target: Self, progress: f32) -> Self {
        Self {
            left: self.left + (target.left - self.left) * progress,
            width: self.width + (target.width - self.width) * progress,
        }
    }
}

struct TabMotion {
    from: TabRect,
    to: TabRect,
    started: Instant,
}

impl TabMotion {
    fn position(&self, now: Instant) -> (TabRect, bool) {
        let progress = (now.duration_since(self.started).as_secs_f32() / SWITCH_DURATION.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        (self.from.interpolate(self.to, eased), progress < 1.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    bounds: Bounds<Pixels>,
    tab: Option<TabRect>,
}

struct Paths {
    geometry: Geometry,
    fill: Path<Pixels>,
    outline: Path<Pixels>,
    corner_cover: Option<Path<Pixels>>,
}

#[derive(Default)]
struct SurfaceState {
    reduced_motion: bool,
    active: Option<EntityId>,
    tab_count: usize,
    scroll_x: f32,
    viewport_width: Option<f32>,
    target: Option<TabRect>,
    motion: Option<TabMotion>,
    visual: Option<TabRect>,
    // 最多保留当前几何的三条短路径；窗口关闭时随 Workspace 释放。
    paths: Option<Paths>,
}

impl SurfaceState {
    fn clear(&mut self) {
        self.active = None;
        self.target = None;
        self.motion = None;
        self.visual = None;
        self.viewport_width = None;
    }

    fn observe(&mut self, id: EntityId, index: usize, count: usize, bounds: Bounds<Pixels>, scroll: &ScrollHandle, window: &mut Window) {
        let viewport = scroll.bounds();
        let viewport_width = f32::from(viewport.size.width);
        let resized = self.viewport_width.is_some_and(|previous| (previous - viewport_width).abs() >= 0.5);
        self.viewport_width = Some(viewport_width);
        if resized && (bounds.left() < viewport.left() || bounds.right() > viewport.right()) {
            // 窗口或侧栏改变可用宽度时，保持当前标签可见；手动滚动不强制回弹。
            scroll.scroll_to_item(index);
            window.request_animation_frame();
        }
        let left = f32::from(bounds.left()).max(f32::from(viewport.left()));
        let right = f32::from(bounds.right()).min(f32::from(viewport.right()));
        let target = (right > left).then_some(TabRect { left, width: right - left });
        let scroll_x = f32::from(scroll.offset().x);
        let now = Instant::now();
        let current = self.motion.as_ref().map(|motion| motion.position(now).0).or(self.target);

        if self.active != Some(id) {
            self.motion = match (current, target) {
                (Some(from), Some(to)) if !self.reduced_motion && self.active.is_some()
                    && self.tab_count == count && (self.scroll_x - scroll_x).abs() < 0.5 && from != to => {
                    Some(TabMotion { from, to, started: now })
                }
                _ => None,
            };
        } else if self.target != target || (self.scroll_x - scroll_x).abs() >= 0.5 {
            // 尺寸与可见范围变化直接定位，不把窗口调整或滚动误认为切换动画。
            self.motion = None;
        }
        self.active = Some(id);
        self.tab_count = count;
        self.scroll_x = scroll_x;
        self.target = target;
    }

    fn update_visual(&mut self, now: Instant) -> bool {
        if let Some(motion) = &self.motion {
            let (position, running) = motion.position(now);
            self.visual = Some(position);
            if !running { self.motion = None; }
            running
        } else {
            self.visual = self.target;
            false
        }
    }

    fn paths(&mut self, geometry: Geometry) -> Option<&Paths> {
        if self.paths.as_ref().is_none_or(|paths| paths.geometry != geometry) {
            self.paths = build_paths(geometry);
        }
        self.paths.as_ref()
    }
}

/// 背景与描边分层，避免编辑区内容遮住边线；不持有文档或视图回调。
#[derive(Clone, Default)]
pub struct WorkspaceSurface(Rc<RefCell<SurfaceState>>);

impl WorkspaceSurface {
    pub fn set_reduced_motion(&self, reduced: bool) {
        let mut state = self.0.borrow_mut();
        state.reduced_motion = reduced;
        if reduced { state.motion = None; state.visual = state.target; }
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn tab_marker(&self, id: EntityId, index: usize, count: usize, scroll: ScrollHandle) -> impl IntoElement {
        let state = self.0.clone();
        canvas(
            move |bounds, window, _| state.borrow_mut().observe(id, index, count, bounds, &scroll, window),
            |_, _, _, _| {},
        ).absolute().top_0().left_0().size_full()
    }

    pub fn background(&self, theme: Theme, animate_ui: bool) -> impl IntoElement {
        let state = self.0.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if f32::from(bounds.size.width) <= 0.0 || f32::from(bounds.size.height) <= TAB_RAIL_HEIGHT {
                    if animate_ui { window.request_animation_frame(); }
                    return;
                }
                let mut state = state.borrow_mut();
                let running = state.update_visual(Instant::now());
                let geometry = Geometry { bounds, tab: state.visual };
                paint_shadows(geometry, &theme, window);
                if let Some(paths) = state.paths(geometry) {
                    // 面板正文已有自己的背景；只在标签及圆角连接区域绘制轮廓填充。
                    let fill_region = Bounds::from_corners(bounds.origin, point(
                        bounds.right(), (bounds.top() + px(TAB_RAIL_HEIGHT + OUTER_RADIUS)).min(bounds.bottom()),
                    ));
                    window.with_content_mask(Some(ContentMask { bounds: fill_region }), |window| {
                        window.paint_path(paths.fill.clone(), theme.surface);
                    });
                }
                if running || animate_ui { window.request_animation_frame(); }
            },
        ).absolute().top_0().left_0().size_full()
    }

    pub fn outline(&self, theme: Theme) -> impl IntoElement {
        let state = self.0.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if f32::from(bounds.size.width) <= 0.0 || f32::from(bounds.size.height) <= TAB_RAIL_HEIGHT { return; }
                let mut state = state.borrow_mut();
                let geometry = Geometry { bounds, tab: state.visual };
                if let Some(paths) = state.paths(geometry) {
                    // GPUI 的 overflow mask 是矩形；覆盖圆角外侧，避免无工具栏时正文露出直角。
                    if let Some(corner) = &paths.corner_cover {
                        window.paint_path(corner.clone(), theme.rail_bg);
                    }
                    window.paint_path(paths.outline.clone(), theme.workspace_edge);
                }
            },
        ).absolute().top_0().left_0().size_full()
    }
}

fn paint_shadows(geometry: Geometry, theme: &Theme, window: &mut Window) {
    let bounds = geometry.bounds;
    let body_top = bounds.top() + px(TAB_RAIL_HEIGHT);
    let panel = Bounds::from_corners(point(bounds.left(), body_top), bounds.bottom_right());
    let shadows = [
        BoxShadow { color: theme.workspace_shadow_near, offset: point(px(0.0), px(0.0)), blur_radius: px(1.0), spread_radius: px(0.0) },
        BoxShadow { color: theme.workspace_shadow_soft, offset: point(px(0.0), px(0.0)), blur_radius: px(9.0), spread_radius: px(0.0) },
    ];
    let max_left_radius = visible_tab(geometry)
        .map_or(f32::from(panel.size.width), |tab| tab.left - f32::from(panel.left()));
    let radius = px(OUTER_RADIUS.min(max_left_radius).min(f32::from(panel.size.height)).max(0.0));
    let margin = px(27.0);
    let top_strip = Bounds::from_corners(
        point(panel.left() - margin, panel.top() - margin),
        point(panel.right() + margin, (panel.top() + radius).min(panel.bottom())),
    );
    let left_strip = Bounds::from_corners(
        point(panel.left() - margin, (panel.top() + radius).min(panel.bottom())),
        point(panel.left() + radius, panel.bottom()),
    );
    let panel_corners = Corners { top_left: radius, ..Corners::default() };
    for strip in [top_strip, left_strip] {
        window.with_content_mask(Some(ContentMask { bounds: strip }), |window| {
            window.paint_shadows(panel, panel_corners.clone(), &shadows);
        });
    }
    if let Some(tab) = visible_tab(geometry) {
        let tab_bounds = Bounds::new(point(px(tab.left), body_top - px(TAB_HEIGHT)), size(px(tab.width), px(TAB_HEIGHT)));
        let radius = px(OUTER_RADIUS.min(tab.width / 2.0).min(TAB_HEIGHT / 2.0));
        let strip = Bounds::from_corners(
            point(tab_bounds.left() - margin, tab_bounds.top() - margin),
            point(tab_bounds.right() + margin, body_top),
        );
        window.with_content_mask(Some(ContentMask { bounds: strip }), |window| {
            window.paint_shadows(tab_bounds, Corners { top_left: radius, top_right: radius, ..Corners::default() }, &shadows);
        });
    }
}

fn visible_tab(geometry: Geometry) -> Option<TabRect> {
    let tab = geometry.tab?;
    let left = tab.left.max(f32::from(geometry.bounds.left()));
    let right = (tab.left + tab.width).min(f32::from(geometry.bounds.right()));
    (right > left).then_some(TabRect { left, width: right - left })
}

fn surface_path(geometry: Geometry, mut path: PathBuilder) -> gpui::Result<Path<Pixels>> {
    let bounds = geometry.bounds;
    let left = f32::from(bounds.left());
    let right = f32::from(bounds.right());
    let bottom = f32::from(bounds.bottom());
    let top = f32::from(bounds.top()) + TAB_RAIL_HEIGHT;
    let tab = visible_tab(geometry);
    let body_radius = OUTER_RADIUS.min(bottom - top).min(tab.map_or(right - left, |tab| tab.left - left)).max(0.0);
    let pt = |x, y| point(px(x), px(y));
    let arc = |path: &mut PathBuilder, radius: f32, sweep: bool, x: f32, y: f32| {
        if radius > 0.0 { path.arc_to(pt(radius, radius), px(0.0), false, sweep, pt(x, y)); }
        else { path.line_to(pt(x, y)); }
    };
    path.move_to(pt(left, bottom));
    path.line_to(pt(left, top + body_radius));
    arc(&mut path, body_radius, true, left + body_radius, top);
    if let Some(tab) = tab {
        let tab_right = tab.left + tab.width;
        let return_left = RETURN_RADIUS.min(tab.left - left - body_radius).min(TAB_HEIGHT / 2.0).max(0.0);
        let return_right = RETURN_RADIUS.min(right - tab_right).min(TAB_HEIGHT / 2.0).max(0.0);
        let radius = OUTER_RADIUS.min(TAB_HEIGHT / 2.0).min(tab.width / 2.0);
        path.line_to(pt(tab.left - return_left, top));
        arc(&mut path, return_left, false, tab.left, top - return_left);
        path.line_to(pt(tab.left, top - TAB_HEIGHT + radius));
        arc(&mut path, radius, true, tab.left + radius, top - TAB_HEIGHT);
        path.line_to(pt(tab_right - radius, top - TAB_HEIGHT));
        arc(&mut path, radius, true, tab_right, top - TAB_HEIGHT + radius);
        path.line_to(pt(tab_right, top - return_right));
        arc(&mut path, return_right, false, tab_right + return_right, top);
    }
    path.line_to(pt(right, top));
    path.line_to(pt(right, bottom));
    path.line_to(pt(left, bottom));
    path.close();
    path.build()
}

fn build_paths(geometry: Geometry) -> Option<Paths> {
    let build = || -> gpui::Result<Paths> {
        let fill = surface_path(geometry, PathBuilder::fill())?;
        let outline = surface_path(geometry, PathBuilder::stroke(px(1.0)))?;
        let bounds = geometry.bounds;
        let left = f32::from(bounds.left());
        let top = f32::from(bounds.top()) + TAB_RAIL_HEIGHT;
        let tab = visible_tab(geometry);
        let radius = OUTER_RADIUS.min(f32::from(bounds.bottom()) - top)
            .min(tab.map_or(f32::from(bounds.size.width), |tab| tab.left - left)).max(0.0);
        let corner_cover = if radius > 0.0 {
            let mut corner = PathBuilder::fill();
            corner.move_to(point(px(left), px(top)));
            corner.line_to(point(px(left + radius), px(top)));
            corner.arc_to(point(px(radius), px(radius)), px(0.0), false, false, point(px(left), px(top + radius)));
            corner.close();
            Some(corner.build()?)
        } else { None };
        Ok(Paths { geometry, fill, outline, corner_cover })
    };
    match build() {
        Ok(paths) => Some(paths),
        Err(error) => { eprintln!("工作区边框绘制失败: {error}"); None }
    }
}
