//! 灰白档案套：完整背板、低平口前套与同向错层纸页；仅前页读取真实缩略图。
//! 复用最初参考 Quiet.md 的弹簧积分；MIT 声明见 assets/licenses/LICENSE-Rendering。
use crate::{theme::Theme, ui::motion::{self, Spring}};
use gpui::{div, px, point, size, prelude::*, AnyElement, Bounds, BoxShadow, ContentMask, Corners, Hsla, PathBuilder, Pixels, Point, RenderImage, SharedString};
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub(crate) struct FolderCover {
    pub title: SharedString,
    pub summary: SharedString,
    pub document_title: SharedString,
    pub image: Option<Arc<RenderImage>>,
    pub error: Option<SharedString>,
    pub papers: usize,
    pub font_family: SharedString,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FolderPose { pub open: f32, pub papers: [f32; 3], pub front: f32, pub dip: f32 }

pub(crate) struct FolderFx {
    open: Spring<1>,
    papers: [Spring<1>; 3],
    front: Spring<1>,
    dip: Spring<1>,
    last: Option<Instant>,
}

impl Default for FolderFx {
    fn default() -> Self {
        Self { open: Spring::new([0.0], 320.0, 31.0),
            papers: [Spring::new([0.0], 340.0, 27.0), Spring::new([0.0], 260.0, 24.0), Spring::new([0.0], 210.0, 23.0)],
            front: Spring::new([0.0], 360.0, 30.0), dip: Spring::new([0.0], 480.0, 24.0), last: None }
    }
}

impl FolderFx {
    pub fn hover(&mut self, hovered: bool) {
        self.open.target = [if hovered { 1.0 } else { 0.0 }];
        for paper in &mut self.papers { paper.target = self.open.target; }
        self.front.target = self.open.target;
    }
    pub fn press(&mut self) { self.dip.v[0] += 14.0; }
    pub fn running(&self) -> bool { self.open.running() || self.front.running() || self.dip.running() || self.papers.iter().any(Spring::running) }
    pub fn pose(&mut self, now: Instant, reduced: bool) -> FolderPose {
        let dt = self.last.replace(now).map_or(1.0 / 120.0, |last| now.saturating_duration_since(last).as_secs_f32());
        if reduced {
            self.hover(false); self.open.snap(); self.front.snap(); self.dip.snap();
            for paper in &mut self.papers { paper.snap(); }
        } else {
            self.open.step(dt); self.front.step(dt); self.dip.step(dt);
            for paper in &mut self.papers { paper.step(dt); }
        }
        FolderPose { open: self.open.p[0], papers: self.papers.map(|paper| paper.p[0]), front: self.front.p[0], dip: self.dip.p[0] }
    }
}

impl FolderCover {
    pub fn render(&self, width: f32, height: f32, pose: FolderPose, theme: &Theme) -> AnyElement {
        let (w, h) = (width, height);
        // 背板以 240×268 为基准，前套静止时占下方 80。
        let (sx, sy) = (w / 240.0, h / 268.0);
        let lift = (-3.0 * pose.open + 4.0 * pose.dip) * sy;
        let front_top = (268.0 - 80.0 + 3.0 * pose.front + 2.0 * pose.dip) * sy;
        let front_corner = 6.0 * sx.min(sy);
        let front_side_top = front_top + front_corner;
        let dark = theme.dark;
        let neutral = |light, dark_color| -> Hsla { gpui::rgb(if dark { dark_color } else { light }).into() };
        let face = theme.surface;
        let back = neutral(0xf5f6f8, 0x25292e);
        let front = neutral(0xfafbfc, 0x2b2f35);
        let edge = neutral(0xcbd0d5, 0x4d5157);
        let shell_edge = neutral(0xe2e6eb, 0x40464d);
        let ink = neutral(0xd9dde2, 0x4b5057);
        let heading = neutral(0x72777e, 0xadb2ba);
        let title_ink = neutral(0x2a2d31, 0xe6e8ec);
        // 沿用文档卡片的阴影令牌，静止时贴近背景，悬停时随背板抬升展开。
        let elevation = (pose.open - 0.7 * pose.dip).clamp(0.0, 1.0);
        let near_shadow = Hsla {
            a: (theme.workspace_shadow_near.a * (0.8 + 1.55 * elevation)).min(1.0),
            ..theme.workspace_shadow_near
        };
        let soft_shadow = Hsla {
            a: (theme.workspace_shadow_soft.a * (0.7 + 1.7 * elevation)).min(1.0),
            ..theme.workspace_shadow_soft
        };
        let paper_shadow = BoxShadow { inset: false, color: Hsla { a: if dark { 0.22 } else { 0.10 }, ..gpui::black() },
            offset: point(px(0.0), px(2.0 * sy)), blur_radius: px(6.0), spread_radius: px(0.0) };
        let papers: Vec<_> = (0..self.papers.min(3)).map(|i| {
            // 展开后仍保留层距：前页上移时不会追上并盖住后两页的顶边。
            let (x, top, width, height, angle, slide, rise, turn) = [
                (16.0, 48.0, 208.0, 200.0, 0.0, 0.0, 18.0, 0.0),
                (21.0, 32.0, 200.0, 216.0, 1.1, 6.0, 18.0, 1.7),
                (27.0, 16.0, 194.0, 232.0, 2.2, 10.0, 12.0, 2.0),
            ][i];
            let p = pose.papers[i];
            Paper { x: (x + slide * p) * sx, y: (top - rise * p) * sy + lift,
                width: width * sx, height: height * sy, angle: (angle + turn * p).to_radians() }
        }).collect();
        let first = papers.first().copied();
        // 文档名只作为稳定种子；重绘不会随机改变背页的排版。
        let seed = self.title.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x100000001b3));
        let back_canvas = gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let o = bounds.origin;
            let p = |x, y| o + point(px(x), px(y + lift));
            let r = 15.0 * sx.min(sy);
            let shell = Paper { x: 0.0, y: lift, width: w, height: h, angle: 0.0 };
            window.paint_drop_shadows(Bounds::new(p(0.0, 0.0), size(px(w), px(h))), Corners::all(px(r)),
                &[BoxShadow { inset: false, color: near_shadow,
                    offset: point(px(0.0), px(1.0 + 2.0 * elevation)), blur_radius: px(2.0 + 5.0 * elevation), spread_radius: px(0.0) },
                  BoxShadow { inset: false, color: soft_shadow,
                    offset: point(px(0.0), px(2.0 + 9.0 * elevation)), blur_radius: px(6.0 + 17.0 * elevation), spread_radius: px(0.0) }]);
            if let Ok(path) = shell.outline(o, r, PathBuilder::fill()) { window.paint_path(path, back); }
            if let Ok(path) = shell.outline(o, r, PathBuilder::stroke(px(0.8))) {
                // 前套覆盖的区域也要裁掉背板描边，否则轮廓外侧会留下一道细线。
                let bounds = Bounds::from_corners(p(-1.0, -1.0), p(w + 1.0, front_side_top));
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    window.paint_path(path, shell_edge);
                });
            }
            for (i, paper) in papers.iter().enumerate().rev() {
                window.paint_drop_shadows(Bounds::new(o + point(px(paper.x), px(paper.y)), size(px(paper.width), px(paper.height))), Corners::all(px(10.0)), &[paper_shadow.clone()]);
                let sheet = motion::color(face, back, [0.0, 0.12, 0.28][i]);
                if let Ok(path) = paper.outline(o, 10.0, PathBuilder::fill()) { window.paint_path(path, sheet); }
                if let Ok(path) = paper.outline(o, 10.0, PathBuilder::stroke(px(1.0))) { window.paint_path(path, edge); }
                if i == 0 { continue; }
                let mut rng = seed.wrapping_add(i as u64 + 1);
                let mut random = || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; (rng % 1000) as f32 / 1000.0 };
                let mut y = 16.0;
                while y < paper.height - 14.0 {
                    let title = y == 16.0;
                    let fraction = if title { 0.42 + 0.30 * random() } else if random() < 0.2 { 0.30 + 0.30 * random() } else { 0.72 + 0.28 * random() };
                    let mut path = PathBuilder::stroke(px(if title { 4.0 } else { 3.0 }));
                    path.move_to(paper.point(o, 12.0, y));
                    path.line_to(paper.point(o, 12.0 + (paper.width - 24.0) * fraction, y));
                    if let Ok(path) = path.build() { window.paint_path(path, if title { heading } else { ink }); }
                    y += if title { 14.0 } else { 8.5 };
                }
            }
        }).absolute().inset_0();

        let content = first.map(|page| {
            let content_width = page.width - 24.0;
            let body = if let Some(image) = &self.image {
                gpui::img(image.clone()).w(px(content_width)).h(px(content_width * super::thumbnail::HEIGHT as f32 / super::thumbnail::WIDTH as f32)).into_any_element()
            } else if let Some(error) = &self.error {
                div().w(px(content_width)).text_size(px(11.0)).text_color(theme.muted).line_clamp(3).child(error.clone()).into_any_element()
            } else {
                div().w(px(content_width)).flex().flex_col().gap(px(8.5))
                    .children([0.95, 0.84, 0.91, 0.55, 0.94, 0.80].map(|fraction| div().w(px(content_width * fraction)).h(px(3.0)).rounded_full().bg(ink))).into_any_element()
            };
            div().absolute().left(px(page.x)).top(px(page.y)).w(px(page.width)).h(px(page.height)).rounded(px(10.0)).overflow_hidden()
                .p(px(12.0)).flex().flex_col().gap(px(10.0)).font_family(self.font_family.clone())
                .child(div().w_full().max_h(px(36.0)).flex_none().text_size(px(13.0)).line_height(px(18.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD).text_color(theme.heading).line_clamp(2).child(self.document_title.clone()))
                .child(div().flex_none().child(body))
        });
        let front_canvas = gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let r = 15.0 * sx.min(sy);
            let p = |x, y| bounds.origin + point(px(x), px(y + lift));
            let mut fill = PathBuilder::fill();
            fill.move_to(p(0.0, front_side_top));
            fill.curve_to(p(front_corner, front_top), p(0.0, front_top));
            fill.line_to(p(w - front_corner, front_top));
            fill.curve_to(p(w, front_side_top), p(w, front_top));
            fill.line_to(p(w, h - r));
            fill.curve_to(p(w - r, h), p(w, h));
            fill.line_to(p(r, h));
            fill.curve_to(p(0.0, h - r), p(0.0, h));
            fill.close();
            if let Ok(path) = fill.build() { window.paint_path(path, front); }
        }).absolute().inset_0();
        div().relative().w(px(w)).h(px(h)).child(back_canvas).children(content).child(front_canvas)
            .child(div().absolute().left(px(21.0 * sx)).right(px(19.0 * sx)).bottom(px(20.0 * sy - lift)).flex().flex_col().gap(px(4.0))
                .child(div().text_size(px(14.0)).line_height(px(20.0)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(title_ink).truncate().child(self.title.clone()))
                .child(div().text_size(px(11.0)).line_height(px(16.0)).text_color(heading).child(self.summary.clone())))
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
struct Paper { x: f32, y: f32, width: f32, height: f32, angle: f32 }
impl Paper {
    fn point(&self, origin: Point<Pixels>, x: f32, y: f32) -> Point<Pixels> {
        let (x, y) = (x - self.width * 0.5, y - self.height * 0.5);
        let (sin, cos) = self.angle.sin_cos();
        origin + point(px(self.x + self.width * 0.5 + x * cos - y * sin), px(self.y + self.height * 0.5 + x * sin + y * cos))
    }
    fn outline(&self, origin: Point<Pixels>, radius: f32, mut path: PathBuilder) -> gpui::Result<gpui::Path<Pixels>> {
        let p = |x, y| self.point(origin, x, y);
        let (w, h, r) = (self.width, self.height, radius);
        path.move_to(p(r, 0.0)); path.line_to(p(w - r, 0.0)); path.curve_to(p(w, r), p(w, 0.0));
        path.line_to(p(w, h - r)); path.curve_to(p(w - r, h), p(w, h));
        path.line_to(p(r, h)); path.curve_to(p(0.0, h - r), p(0.0, h));
        path.line_to(p(0.0, r)); path.curve_to(p(r, 0.0), p(0.0, 0.0)); path.close(); path.build()
    }
}
