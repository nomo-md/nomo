//! 文档卡片与排序浮层共用同一张封面，松手时不切换排版或重新生成预览。
use super::*;

const PRESS_DEPTH: f32 = 2.0;
const HINT_DURATION: Duration = Duration::from_millis(160);

pub(crate) struct DocumentMotion {
    elevation: motion::Spring<1>,
    hint: motion::ScalarMotion,
    last: Option<Instant>,
}

impl Default for DocumentMotion {
    fn default() -> Self {
        Self {
            elevation: motion::Spring::new([0.0], 800.0, 52.0),
            hint: motion::ScalarMotion::default(),
            last: None,
        }
    }
}

impl DocumentMotion {
    pub(super) fn retarget(&mut self, hovered: bool, pressed: bool, now: Instant, reduced: bool) {
        // 先走完旧目标到事件时刻的这段时间，再用当前速度转向新目标。
        self.advance(now, reduced);
        self.elevation.target = [match (hovered, pressed) {
            (true, false) => 1.0,
            (true, true) => (CARD_LIFT - PRESS_DEPTH) / CARD_LIFT,
            (false, _) => 0.0,
        }];
        if reduced {
            self.elevation.snap();
        }
        self.hint.set(hovered, reduced, HINT_DURATION);
    }

    pub(crate) fn advance(&mut self, now: Instant, reduced: bool) {
        let last = self.last.unwrap_or(now);
        self.last = Some(last.max(now));
        if reduced {
            self.elevation.snap();
        } else if self.elevation.running() {
            self.elevation.step(now.saturating_duration_since(last).as_secs_f32());
        }
        self.hint.retime(reduced, HINT_DURATION);
    }

    pub(crate) fn running(&self) -> bool {
        self.elevation.running() || self.hint.running()
    }

    pub(crate) fn values(&self) -> (f32, f32) {
        (self.elevation.p[0].clamp(0.0, 1.0), self.hint.value())
    }
}

#[derive(Clone)]
pub(crate) struct DocumentCover {
    pub title: SharedString,
    pub image: Option<Arc<RenderImage>>,
    pub error: Option<SharedString>,
    pub font_family: SharedString,
    pub open_label: SharedString,
    pub(super) thumbnail_opacity: f32,
}

impl DocumentCover {
    pub fn render(&self, width: f32, height: f32, elevation: f32, hint: f32, reduced: bool, theme: &Theme) -> gpui::AnyElement {
        let lift = if reduced { 0.0 } else { CARD_LIFT * elevation };
        let near_shadow = gpui::Hsla { a: (theme.workspace_shadow_near.a * (1.0 + 0.75 * elevation)).min(1.0), ..theme.workspace_shadow_near };
        let soft_shadow = gpui::Hsla { a: (theme.workspace_shadow_soft.a * (1.0 + 0.67 * elevation)).min(1.0), ..theme.workspace_shadow_soft };
        let inner_width = width - CARD_PADDING * 2.0 - 2.0;
        let inner_height = inner_width * thumbnail::HEIGHT as f32 / thumbnail::WIDTH as f32;
        let skeleton = || div().w(px(inner_width)).h(px(inner_height)).flex().flex_col().gap(px(10.0))
            .children([0.92, 1.0, 0.76, 0.88, 0.62].map(|fraction| div().w(px(inner_width * fraction)).h(px(5.0)).rounded(px(3.0))
                .bg(motion::color(theme.surface, theme.border, 0.45))));
        let image = if let Some(image) = &self.image {
            let image = gpui::img(image.clone()).w(px(inner_width)).h(px(inner_height)).rounded(px(3.0));
            if self.thumbnail_opacity < 1.0 {
                div().relative().size_full().child(skeleton().opacity(1.0 - self.thumbnail_opacity))
                    .child(image.absolute().top_0().left_0().opacity(self.thumbnail_opacity)).into_any_element()
            } else { image.into_any_element() }
        } else if let Some(error) = &self.error {
            div().w(px(inner_width)).h(px(inner_height)).flex().flex_col().items_center().justify_center().gap(px(8.0))
                .child(icon(IconName::FileText,24.0,theme.muted))
                .child(div().text_size(px(12.0)).text_color(theme.muted).text_center().line_clamp(3).child(error.clone())).into_any_element()
        } else {
            skeleton().into_any_element()
        };
        div().relative().w(px(width)).h(px(height + CARD_LIFT))
            .child(div().absolute().top(px(CARD_LIFT - lift)).left_0().w(px(width)).h(px(height))
                .rounded(px(14.0)).border_1().border_color(motion::color(theme.surface, theme.border, 0.3)).bg(theme.surface)
                .shadow(vec![
                    gpui::BoxShadow { inset: false, color: near_shadow, offset: gpui::point(px(0.0),px(1.0 + 2.0 * elevation)), blur_radius: px(2.0 + 5.0 * elevation), spread_radius: px(0.0) },
                    gpui::BoxShadow { inset: false, color: soft_shadow, offset: gpui::point(px(0.0),px(2.0 + 7.0 * elevation)), blur_radius: px(7.0 + 12.0 * elevation), spread_radius: px(0.0) },
                ])
                .p(px(CARD_PADDING)).flex().flex_col().gap(px(CARD_PREVIEW_GAP))
                .child(div().w_full().h(px(CARD_TITLE_HEIGHT)).flex_none().text_size(px(14.0)).line_height(px(20.0))
                    .font_family(self.font_family.clone()).font_weight(FontWeight::BOLD).text_color(theme.heading).line_clamp(2).child(self.title.clone()))
                .child(div().w(px(inner_width)).h(px(inner_height)).flex_none().overflow_hidden().child(image))
                .child(div().relative().top(px(if reduced { 0.0 } else { 3.0 * (1.0 - hint) })).opacity(hint)
                    .w_full().h(px(CARD_FOOTER_HEIGHT)).flex_none().flex().items_center().justify_between()
                    .text_size(px(11.0)).line_height(px(CARD_FOOTER_HEIGHT)).text_color(theme.muted)
                    .child(self.open_label.clone())
                    .child(icon(IconName::ArrowUp, 13.0, theme.muted)
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::FRAC_PI_4))))))
            .into_any_element()
    }
}
