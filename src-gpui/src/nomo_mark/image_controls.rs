//! 当前可见图片的宽度按钮，复用布局矩形与图片属性事务。
use super::*;
use crate::nomo_mark::image_properties::ImageWidth;
use crate::ui::{motion, popup};
use gpui::{Animation, AnimationExt, AnyElement, DispatchPhase, FontWeight, Hsla};
use std::time::Duration;

const WIDTHS: [(u8, &str); 3] = [(100, "100%"), (75, "75%"), (50, "50%")];

pub(super) struct ImageWidthHover {
    source: Range<usize>,
    image: Bounds<Pixels>,
    toolbar: Bounds<Pixels>,
    // 外层 None 表示当前图片属性无法安全编辑。
    selected: Option<Option<u8>>,
}

fn toolbar_bounds(image: Bounds<Pixels>, clip: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let width = px(120.0).min(clip.size.width);
    let height = px(28.0).min(clip.size.height);
    if width < px(100.0) || height < px(22.0) { return None; }
    let inset = px(5.0);
    let right = image.right() - if image.size.width >= width + inset * 2.0 { inset } else { px(0.0) };
    let top = image.top() + if image.size.height >= height + inset * 2.0 { inset } else { px(0.0) };
    let max_x = clip.left() + (clip.size.width - width).max(px(0.0));
    let max_y = clip.top() + (clip.size.height - height).max(px(0.0));
    Some(Bounds::new(
        point((right - width).clamp(clip.left(), max_x), top.clamp(clip.top(), max_y)),
        size(width, height),
    ))
}

impl NomoMark {
    fn can_resize_images(&self) -> bool {
        self.effective_mode() == ViewMode::Semantic
            && self.host.presentation == Presentation::Editable
            && !self.host.interaction.protected()
            && !self.auxiliary_input
            && self.object.is_none()
            && self.context_menu.is_none()
            && self.context_form.is_none()
            && self.code_language.is_none()
            && self.quick_insert.is_none()
            && !self.menu_open
            && !self.table_size_picker
            && !self.block_menu_open
    }

    fn image_width_target(
        &self,
        clip: Bounds<Pixels>,
        pointer: Point<Pixels>,
    ) -> Option<(Range<usize>, Bounds<Pixels>, Bounds<Pixels>)> {
        let origin = self.content_bounds()?.origin;
        let layout = self.layout.as_ref()?;
        // 窄图上的按钮可以伸到左侧，仍用本帧图片和裁切边界重新定位。
        let retained = self.image_width_hover.as_ref().filter(|hover| {
            hover.selected.is_some() && hover.toolbar.contains(&pointer)
        }).and_then(|hover| {
            layout.find_visible_image(origin, clip, &mut |source, image| {
                *source == hover.source && toolbar_bounds(image, clip).is_some_and(|bar| bar.contains(&pointer))
            })
        });
        let (source, image) = retained.or_else(|| {
            layout.find_visible_image(origin, clip, &mut |_, image| image.contains(&pointer))
        })?;
        let toolbar = toolbar_bounds(image, clip)?;
        Some((source, image, toolbar))
    }

    pub(super) fn image_width_controls(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.can_resize_images() || !self.layout_resources.has_images() {
            self.image_width_hover = None;
            return None;
        }
        let view_id = cx.entity_id();
        let owner = cx.entity().downgrade();
        let hover_owner = owner.clone();
        Some(gpui::canvas(
            move |_, window, cx| {
                owner.update(cx, |view, cx| {
                    let clip = view.visible_bounds?.intersect(&window.content_mask().bounds);
                    if clip.is_empty() {
                        view.image_width_hover = None;
                        return None;
                    }
                    let pointer = window.mouse_position();
                    let target = (clip.contains(&pointer)
                        && window.is_window_hovered()
                        && window.is_window_active()
                        && !window.modifiers().modified()
                        && !window.has_active_prompt()
                        && !cx.has_active_drag())
                        .then(|| view.image_width_target(clip, pointer)).flatten();
                    view.image_width_hover = target.map(|(source, image, toolbar)| {
                        let selected = view.image_width_hover.as_ref()
                            .filter(|old| old.source == source)
                            .map(|old| old.selected)
                            .unwrap_or_else(|| view.image_width_value(&source).map(|width| {
                                WIDTHS.iter().map(|(percent, _)| *percent).find(|percent| {
                                    matches!(width, Some(ImageWidth::Percent(value)) if value == f32::from(*percent))
                                })
                            }));
                        ImageWidthHover { source, image, toolbar, selected }
                    });
                    let hover = view.image_width_hover.as_ref();
                    let areas = hover.map(|hover| (hover.image, hover.selected.map(|_| hover.toolbar)));
                    let bar = hover.and_then(|hover| {
                        let selected = hover.selected?;
                        let bounds = hover.toolbar;
                        let mut bar = view.render_image_width_bar(hover.source.clone(), bounds, selected, cx);
                        let available = size(AvailableSpace::Definite(bounds.size.width), AvailableSpace::Definite(bounds.size.height));
                        bar.prepaint_as_root(bounds.origin, available, window, cx);
                        Some(bar)
                    });
                    Some((clip, areas, bar))
                }).ok().flatten()
            },
            move |_, frame, window, cx| {
                let Some((clip, areas, bar)) = frame else { return; };
                if let Some(mut bar) = bar { bar.paint(window, cx); }
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture { return; }
                    let position = event.position;
                    let over_editor = clip.contains(&position)
                        && window.is_window_hovered() && window.is_window_active()
                        && !event.modifiers.modified() && !window.has_active_prompt() && !cx.has_active_drag();
                    let changed = if let Some((image, toolbar)) = areas {
                        !over_editor || !(image.contains(&position) || toolbar.is_some_and(|bar| bar.contains(&position)))
                    } else {
                        over_editor && event.pressed_button.is_none()
                            && hover_owner.read_with(cx, |view, _| view.image_width_target(clip, position).is_some()).unwrap_or(false)
                    };
                    if changed { cx.notify(view_id); }
                });
            },
        ).absolute().inset_0())
    }

    fn render_image_width_bar(
        &self,
        source: Range<usize>,
        bounds: Bounds<Pixels>,
        selected: Option<u8>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text: Hsla = gpui::rgb(0xffffff).into();
        let identity = self.host.parsed.identity.clone();
        let mut bar = div().id(("image-width", source.start)).w(bounds.size.width).h(bounds.size.height)
            .flex().items_center().gap(px(2.0)).p(px(2.0))
            .rounded(px(8.0)).bg(gpui::rgba(0x161b24b8))
            .border_1().border_color(text.opacity(0.22))
            .text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(text);
        for (percent, label) in WIDTHS {
            let source = source.clone();
            let identity = identity.clone();
            bar = bar.child(div().id(("image-width-preset", usize::from(percent)))
                .h_full().flex_1().min_w_0().flex().items_center().justify_center()
                .rounded(px(5.0)).cursor_pointer().child(label)
                .when(selected == Some(percent), |button| button.bg(text.opacity(0.22)))
                .hover(|button| button.bg(text.opacity(0.28)))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    if view.host.parsed.identity == identity && view.can_resize_images() && selected != Some(percent) {
                        view.apply_image_width_preset(source.clone(), percent, cx);
                        window.focus(&view.focus, cx);
                    }
                })));
        }
        if popup::reduced_motion(cx) { return bar.into_any_element(); }
        bar.with_animation(("image-width-enter", source.start), Animation::new(Duration::from_millis(140)).with_easing(motion::ease),
            |bar, progress| bar.opacity(progress))
            .into_any_element()
    }
}
