//! 按触发元素的真实边界布置弹层；只在弹层挂载期间测量和播放入场动效。
use gpui::{
    Animation, AnimationExt, AnyElement, AvailableSpace, Bounds, Div, ElementId, IntoElement,
    Length, Pixels, Point, Size, Stateful, StatefulInteractiveElement, Styled, canvas, point, px, size,
};
use std::{cell::Cell, rc::Rc, time::Duration};

#[derive(Clone)]
pub(crate) enum PopupAnchor {
    /// 弹层作为触发元素的绝对定位子元素，共用本帧布局边界。
    Parent,
    Bounds(Bounds<Pixels>),
    /// 工具栏等跨视图弹层读取本帧边界，缺少测量时使用打开位置。
    Tracked(Rc<Cell<Option<Bounds<Pixels>>>>, Point<Pixels>),
}

impl PopupAnchor {
    pub(crate) fn contains_trigger(&self, position: Point<Pixels>) -> bool {
        matches!(self, Self::Tracked(bounds, _) if bounds.get().is_some_and(|bounds| bounds.contains(&position)))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PopupPlacement { BelowLeft, BelowRight, AboveLeft, AboveRight, Right }

const GAP: f32 = 6.0;
const MARGIN: f32 = 8.0;

/// 内容必须有稳定 ID；高度受可用空间约束，长菜单通过自身滚动访问。
pub(crate) fn popup(
    id: impl Into<ElementId>,
    anchor: PopupAnchor,
    placement: PopupPlacement,
    panel: Stateful<Div>,
    reduced: bool,
) -> impl IntoElement {
    popup_panel(id, anchor, placement, panel, reduced, true, false)
}

/// 原版右下角控件：优先向上、10px 间隔，140ms 淡入并从下方移入。
pub(crate) fn status_popup(
    id: impl Into<ElementId>, anchor: PopupAnchor, panel: Stateful<Div>, reduced: bool,
) -> impl IntoElement {
    popup_panel(id, anchor, PopupPlacement::AboveRight, panel, reduced, true, true)
}

/// 列表自行收缩和滚动，保持快捷插入的标题与操作提示固定。
pub(crate) fn popup_with_list(
    id: impl Into<ElementId>, anchor: PopupAnchor, placement: PopupPlacement,
    panel: Stateful<Div>, reduced: bool,
) -> impl IntoElement {
    popup_panel(id, anchor, placement, panel, reduced, false, false)
}

fn popup_panel(
    id: impl Into<ElementId>, anchor: PopupAnchor, placement: PopupPlacement,
    mut panel: Stateful<Div>, reduced: bool, scroll_panel: bool, status: bool,
) -> impl IntoElement {
    let id = id.into();
    canvas(
        move |bounds, window, cx| {
            let anchor = match anchor {
                PopupAnchor::Parent => bounds,
                PopupAnchor::Bounds(bounds) => bounds,
                PopupAnchor::Tracked(bounds, fallback) => bounds.get()
                    .unwrap_or_else(|| Bounds::new(fallback, size(px(0.0), px(0.0)))),
            };
            let viewport = window.viewport_size();
            let margin = px(MARGIN) + window.client_inset().unwrap_or(px(0.0));
            let limits = Bounds::new(point(margin, margin), size(
                (viewport.width - margin * 2.0).max(px(0.0)),
                (viewport.height - margin * 2.0).max(px(0.0)),
            ));
            let gap = px(if status { 10.0 } else { GAP });
            let below = (limits.bottom() - anchor.bottom() - gap).max(px(0.0));
            let above = (anchor.top() - gap - limits.top()).max(px(0.0));
            let max_height = match placement {
                PopupPlacement::Right => limits.size.height,
                _ => below.max(above),
            };
            let max_width = match placement {
                PopupPlacement::Right => (limits.right() - anchor.right() - px(2.0))
                    .max(anchor.left() - limits.left() - px(2.0)).max(px(0.0)),
                _ => limits.size.width,
            };
            // 保留调用方已有的高度上限，例如隐藏标签列表的 320px。
            let cap = |length: Option<Length>, available: Pixels| match length {
                Some(Length::Definite(length)) => length.to_pixels(available.into(), window.rem_size()).min(available),
                _ => available,
            };
            let max_width = cap(panel.style().max_size.width, max_width);
            let max_height = cap(panel.style().max_size.height, max_height);
            let panel = panel.max_w(max_width).max_h(max_height).overflow_x_hidden();
            let panel = if scroll_panel { panel.overflow_y_scroll() } else { panel.overflow_y_hidden() };
            let duration = Duration::from_millis(if status { 140 } else if reduced { 90 } else { 160 });
            let mut panel = if status && reduced { panel.into_any_element() } else { panel.with_animation(id, Animation::new(duration)
                .with_easing(|progress| 1.0 - (1.0 - progress).powi(3)),
                move |panel, progress| {
                    let panel = if reduced { panel } else { panel.relative().top(px((if status { 4.0 } else { -3.0 }) * (1.0 - progress))) };
                    panel.opacity(if reduced { 0.78 + 0.22 * progress } else { progress })
                })
                .into_any_element() };
            let measured = panel.layout_as_root(size(AvailableSpace::MaxContent, AvailableSpace::MaxContent), window, cx);
            let origin = popup_origin(anchor, measured, limits, placement, below, above, gap);
            panel.prepaint_at(origin, window, cx);
            panel
        },
        |_, mut panel: AnyElement, window, cx| panel.paint(window, cx),
    ).absolute().top_0().left_0().size_full()
}

fn popup_origin(
    anchor: Bounds<Pixels>,
    measured: Size<Pixels>,
    limits: Bounds<Pixels>,
    placement: PopupPlacement,
    below: Pixels,
    above: Pixels,
    gap: Pixels,
) -> Point<Pixels> {
    let (x, y) = match placement {
        PopupPlacement::BelowLeft | PopupPlacement::BelowRight | PopupPlacement::AboveLeft | PopupPlacement::AboveRight => {
            let x = if matches!(placement, PopupPlacement::BelowRight | PopupPlacement::AboveRight) {
                anchor.right() - measured.width
            } else { anchor.left() };
            // 不能直接向窗口内平移，否则弹层会盖回触发按钮。
            let go_above = if matches!(placement, PopupPlacement::AboveLeft | PopupPlacement::AboveRight) {
                measured.height <= above || above >= below
            } else { measured.height > below && above > below };
            let y = if go_above { anchor.top() - gap - measured.height }
                else { anchor.bottom() + gap };
            (x, y)
        }
        PopupPlacement::Right => {
            let right = anchor.right() + px(2.0);
            let left = anchor.left() - px(2.0) - measured.width;
            let x = if right + measured.width <= limits.right() { right } else { left };
            (x, anchor.top() - px(4.0))
        }
    };
    point(
        x.clamp(limits.left(), (limits.right() - measured.width).max(limits.left())).round(),
        y.clamp(limits.top(), (limits.bottom() - measured.height).max(limits.top())).round(),
    )
}

pub(crate) fn reduced_motion(cx: &gpui::App) -> bool {
    cx.try_global::<super::motion::MotionPreference>().is_some_and(|preference| preference.reduced)
}
