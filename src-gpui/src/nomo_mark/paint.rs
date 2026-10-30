//! 与窗口无关的显示列表绘制，资源由宿主已有的共享 Store 提供。
use crate::embeds::EmbedStore;
use crate::images::ImageStore;
use crate::layout::paint_box;
use crate::markdown::render::{BlockLayout, DisplayItem, Paint};
use crate::theme::Theme;
use crate::ui::components::IconName;
use gpui::{
    App, Bounds, Corners, Hsla, Pixels, Point, TransformationMatrix, Window, point, px, size,
};
use std::ops::Range;

/// 指针命中与正文共用同一帧几何；只注册可见区域，不触发鼠标移动时的重排。
pub(super) fn prepaint_cursors(
    layout: &BlockLayout, origin: Point<Pixels>, clip: Bounds<Pixels>, window: &mut Window,
    cursors: &mut Vec<(gpui::Hitbox, gpui::CursorStyle)>,
) {
    use gpui::CursorStyle::{Arrow, IBeam, PointingHand};
    for item in &layout.items {
        let (x, y, w, h, cursor) = match item {
            DisplayItem::Text { x, y, layout, map } if !map.is_empty() =>
                (*x, *y, layout.max_line_width().max(4.0), layout.height(), IBeam),
            DisplayItem::TableCell { x, y, w, h, .. } => (*x, *y, *w, *h, IBeam),
            DisplayItem::Checkbox { x, y, size, .. } | DisplayItem::Disclosure { x, y, size, .. }
                | DisplayItem::CopyButton { x, y, size, .. } => (*x, *y, *size, *size, PointingHand),
            DisplayItem::TocLink { x, y, w, h, .. } => (*x, *y, *w, *h, PointingHand),
            DisplayItem::BlockControls { x, y, w, h, kind, .. } =>
                (*x, *y, *w, *h, if matches!(*kind, "rule" | "diagram-toolbar") { Arrow } else { PointingHand }),
            DisplayItem::CodeBody { x, y, w, h, body, .. } => {
                let (width, height) = body.viewport(*w, *h);
                (*x + body.gutter, *y, width, height, IBeam)
            }
            DisplayItem::Horizontal { x, y, w, h, offset, layout, .. } => {
                let viewport = Bounds::new(origin + point(px(*x), px(*y)), size(px(*w), px((*h - 8.0).max(0.0)))).intersect(&clip);
                if viewport.size.width > px(0.0) && viewport.size.height > px(0.0) {
                    prepaint_cursors(layout, origin + point(px(*x - offset.get()), px(*y)), viewport, window, cursors);
                }
                continue;
            }
            _ => continue,
        };
        let bounds = Bounds::new(origin + point(px(x), px(y)), size(px(w), px(h))).intersect(&clip);
        if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) {
            cursors.push((window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal), cursor));
        }
    }
}

pub fn paint_block(
    layout: &BlockLayout,
    origin: Point<Pixels>,
    clip: Range<f32>,
    selection: Option<Range<usize>>,
    cell_selections: &[Range<usize>],
    theme: &Theme,
    store: &ImageStore,
    window: &mut Window,
    cx: &App,
) {
    // 背景必须先于选区；表格和代码块的实色底不能覆盖选择反馈。
    paint_layer(layout, origin, clip.clone(), true, theme, store, window, cx);
    if let Some(selection) = selection {
        for (x, y, w, h) in layout.source_rects_clipped(selection, clip.clone()) {
            paint_box(Bounds::new(origin + point(px(x), px(y)), size(px(w), px(h))), theme.selection, 0.0, None, window);
        }
    }
    for (x, y, w, h) in layout.selected_cell_rects(cell_selections) {
        if y + h < clip.start || y > clip.end { continue; }
        paint_box(Bounds::new(origin + point(px(x), px(y)), size(px(w), px(h))), theme.selection, 0.0, None, window);
    }
    paint_layer(layout, origin, clip, false, theme, store, window, cx);
}

fn paint_layer(
    layout: &BlockLayout,
    origin: Point<Pixels>,
    clip: Range<f32>,
    background: bool,
    theme: &Theme,
    store: &ImageStore,
    window: &mut Window,
    cx: &App,
) {
    let embeds = cx.global::<EmbedStore>();
    let resolve = |paint: Paint| -> Hsla {
        match paint {
            Paint::Border => theme.border,
            Paint::CodeBg => theme.code_bg,
            Paint::TocCard => theme.surface,
            Paint::CodeBorder => theme.code_border,
            Paint::QuoteBorder => theme.quote_border,
            Paint::TableBorder => theme.table_border,
            Paint::TableHeader => theme.table_header_bg,
            Paint::Surface => theme.surface,
            Paint::Role(role) => role.resolve(theme),
            Paint::CalloutBorder(role) => match role {
                crate::layout::ColorRole::Note => theme.callout_note,
                crate::layout::ColorRole::Tip => theme.callout_tip,
                crate::layout::ColorRole::Important => theme.callout_important,
                crate::layout::ColorRole::Warning => theme.callout_warning,
                crate::layout::ColorRole::Caution => theme.callout_caution,
                _ => role.resolve(theme),
            },
        }
    };
    for item in &layout.items {
        if !matches!(item, DisplayItem::Horizontal { .. } | DisplayItem::CodeBody { .. })
            && matches!(item, DisplayItem::Rect { .. }) != background { continue; }
        match item {
            DisplayItem::CalloutHeader { x, y, size: side, kind, role, .. } => {
                let path = match kind {
                    pulldown_cmark::BlockQuoteKind::Note => IconName::Info.path(),
                    pulldown_cmark::BlockQuoteKind::Tip => "icons/callout-tip.svg",
                    pulldown_cmark::BlockQuoteKind::Important => "icons/callout-important.svg",
                    pulldown_cmark::BlockQuoteKind::Warning => IconName::AlertTriangle.path(),
                    pulldown_cmark::BlockQuoteKind::Caution => "icons/callout-caution.svg",
                };
                let _ = window.paint_svg(Bounds::new(origin + point(px(*x), px(*y)), size(px(*side), px(*side))),
                    path.into(), None, TransformationMatrix::unit(), role.resolve(theme), cx);
            }
            DisplayItem::CodeHeader { .. } | DisplayItem::CodeLanguage { .. } | DisplayItem::TableCell { .. } | DisplayItem::BlockControls { .. } | DisplayItem::TocLink { .. } => {}
            DisplayItem::CodeBody { x, y, w, h, body, .. } => {
                if y+h < clip.start || *y > clip.end { continue; }
                let (width, height) = body.viewport(*w, *h);
                let (dx, dy) = body.offset.get();
                let local_clip = (clip.start-y+dy).max(dy)..(clip.end-y+dy).min(dy+height);
                let left = origin + point(px(*x), px(*y));
                if body.gutter > 0.0 {
                    window.with_content_mask(Some(gpui::ContentMask { bounds: Bounds::new(left, size(px(body.gutter), px(height))) }), |window| {
                        paint_layer(&body.numbers, left-point(px(0.0), px(dy)), local_clip.clone(), background, theme, store, window, cx);
                    });
                    if background {
                        let border = body.scrollbar/13.0;
                        paint_box(Bounds::new(left+point(px(body.gutter-border), px(0.0)), size(px(border), px(*h))), theme.code_border, 0.0, None, window);
                    }
                }
                let content = left + point(px(body.gutter), px(0.0));
                window.with_content_mask(Some(gpui::ContentMask { bounds: Bounds::new(content, size(px(width), px(height))) }), |window| {
                    paint_layer(&body.code, content-point(px(dx), px(dy)), local_clip, background, theme, store, window, cx);
                });
                if background {
                    let inset = body.scrollbar*0.25;
                    if body.content_width > width {
                        let ratio = width/body.content_width;
                        paint_box(Bounds::new(content+point(px(dx*ratio+inset), px(height+inset)), size(px((width*ratio-2.0*inset).max(2.0)), px(body.scrollbar-2.0*inset))), theme.scrollbar, inset, None, window);
                    }
                    if body.code.height > height {
                        let ratio = height/body.code.height;
                        paint_box(Bounds::new(content+point(px(width+inset), px(dy*ratio+inset)), size(px(body.scrollbar-2.0*inset), px((height*ratio-2.0*inset).max(2.0)))), theme.scrollbar, inset, None, window);
                    }
                }
            }
            DisplayItem::Horizontal {
                x,
                y,
                w,
                h,
                content_width,
                offset,
                layout,
                ..
            } => {
                if y + h < clip.start || *y > clip.end {
                    continue;
                }
                let viewport =
                    Bounds::new(origin + point(px(*x), px(*y)), size(px(*w), px(*h - 8.0)));
                window.with_content_mask(Some(gpui::ContentMask { bounds: viewport }), |window| {
                    paint_layer(
                        layout,
                        origin + point(px(x - offset.get()), px(*y)),
                        (clip.start - y)..(clip.end - y).min(*h - 8.0),
                        background,
                        theme,
                        store,
                        window,
                        cx,
                    )
                });
                if !background { continue; }
                let bar = Bounds::new(
                    origin + point(px(*x), px(y + h - 6.0)),
                    size(px(*w), px(5.0)),
                );
                paint_box(bar, theme.code_bg, 2.0, None, window);
                let ratio = w / content_width;
                paint_box(
                    Bounds::new(
                        bar.origin + point(px(offset.get() * ratio), px(0.0)),
                        size(px(w * ratio), px(5.0)),
                    ),
                    theme.border,
                    2.0,
                    None,
                    window,
                );
            }
            DisplayItem::Disclosure {
                x,
                y,
                size: side,
                open,
                ..
            } => {
                let bounds =
                    Bounds::new(origin + point(px(*x), px(*y)), size(px(*side), px(*side)));
                let icon = if *open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                };
                let _ = window.paint_svg(
                    bounds,
                    icon.path().into(),
                    None,
                    TransformationMatrix::unit(),
                    theme.muted,
                    cx,
                );
            }
            DisplayItem::Rect {
                x,
                y,
                w,
                h,
                paint,
                radius,
                border,
            } => {
                if y + h < clip.start || *y > clip.end {
                    continue;
                }
                let bounds = Bounds::new(origin + point(px(*x), px(*y)), size(px(*w), px(*h)));
                if *paint == Paint::TocCard {
                    window.paint_drop_shadows(bounds, Corners::all(px(*radius)), &[gpui::BoxShadow {
                        inset: false, color: theme.workspace_shadow_near, offset: point(px(0.0), px(2.0)),
                        blur_radius: px(8.0), spread_radius: px(0.0),
                    }]);
                }
                paint_box(
                    bounds,
                    resolve(*paint),
                    *radius,
                    border.map(|border| (resolve(border), 1.0)),
                    window,
                );
            }
            DisplayItem::Text { x, y, layout, .. } => {
                if y + layout.height() < clip.start || *y > clip.end {
                    continue;
                }
                layout.paint(
                    origin + point(px(*x), px(*y)),
                    (clip.start - y)..(clip.end - y),
                    theme,
                    window,
                );
                // 行内公式位图按基线贴到占位位置。
                for (key, ox, oy, w, h) in layout.object_rects_clipped((clip.start-y)..(clip.end-y)) {
                    let image = match key {
                        crate::layout::InlineAsset::MathInput => {
                            let bounds = Bounds::new(origin + point(px(x + ox), px(y + oy)), size(px(w), px(h)));
                            paint_box(bounds, theme.surface, 4.0, Some((theme.accent, 1.0)), window);
                            None
                        }
                        crate::layout::InlineAsset::Embed(key) => embeds.image(*key),
                        crate::layout::InlineAsset::Image(path) => store.image(path),
                        crate::layout::InlineAsset::Comment { .. } => {
                            let bounds = Bounds::new(origin + point(px(x + ox), px(y + oy)), size(px(w), px(h)));
                            crate::nomo_mark::comments::paint(key, bounds, theme, window);
                            None
                        }
                    };
                    if let Some(image) = image {
                        let bounds =
                            Bounds::new(origin + point(px(x + ox), px(y + oy)), size(px(w), px(h)));
                        let mask = window.content_mask().bounds;
                        if bounds.right() <= mask.left() || bounds.left() >= mask.right() { continue; }
                        let frame = if let crate::layout::InlineAsset::Image(path) = key { store.frame(path, window, cx) } else { 0 };
                        let _ = window.paint_image(bounds, bounds, Corners::default(), image, frame, false);
                    }
                }
            }
            DisplayItem::Embed {
                x, y, w, h, key, ..
            } => {
                if y + h < clip.start || *y > clip.end {
                    continue;
                }
                let bounds = Bounds::new(origin + point(px(*x), px(*y)), size(px(*w), px(*h)));
                match embeds.image(*key) {
                    Some(image) => {
                        let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
                    }
                    None => paint_box(bounds, theme.code_bg, 6.0, None, window),
                }
            }
            DisplayItem::Image {
                x, y, w, h, path, ..
            } => {
                if y + h < clip.start || *y > clip.end {
                    continue;
                }
                let bounds = Bounds::new(origin + point(px(*x), px(*y)), size(px(*w), px(*h)));
                match store.image(path) {
                    Some(image) => {
                        let mask = window.content_mask().bounds;
                        if bounds.right() <= mask.left() || bounds.left() >= mask.right() { continue; }
                        let frame = store.frame(path, window, cx);
                        let _ = window.paint_image(bounds, bounds, Corners::all(px(6.0)), image, frame, false);
                    }
                    None => paint_box(bounds, theme.code_bg, 6.0, None, window),
                }
            }
            DisplayItem::CopyButton {
                x, y, size: side, ..
            } => {
                if y + side < clip.start || *y > clip.end {
                    continue;
                }
                let inset = side * 0.2;
                let bounds = Bounds::new(
                    origin + point(px(x + inset), px(y + inset)),
                    size(px(side - inset * 2.0), px(side - inset * 2.0)),
                );
                let _ = window.paint_svg(
                    bounds,
                    IconName::Copy.path().into(),
                    None,
                    TransformationMatrix::unit(),
                    theme.muted,
                    cx,
                );
            }
            DisplayItem::Checkbox {
                x,
                y,
                size: side,
                checked,
                ..
            } => {
                let bounds =
                    Bounds::new(origin + point(px(*x), px(*y)), size(px(*side), px(*side)));
                if *checked {
                    // 勾号由块布局中紧随其后的文字项绘制。
                    paint_box(bounds, theme.accent, 4.0, None, window);
                } else {
                    paint_box(bounds, theme.surface, 4.0, Some((theme.muted, 1.4)), window);
                }
            }
        }
    }
}
