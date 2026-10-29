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
pub fn paint_block(
    layout: &BlockLayout,
    origin: Point<Pixels>,
    clip: Range<f32>,
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
            Paint::CodeBorder => theme.code_border,
            Paint::QuoteBorder => theme.quote_border,
            Paint::CalloutBg(role) => role.resolve(theme).opacity(if theme.dark { 0.14 } else { 0.07 }),
            Paint::CalloutBorder(role) => role.resolve(theme).opacity(if theme.dark { 0.32 } else { 0.24 }),
            Paint::TableBorder => theme.table_border,
            Paint::TableHeader => theme.table_header_bg,
            Paint::Surface => theme.surface,
        }
    };
    for item in &layout.items {
        match item {
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
                    paint_block(
                        layout,
                        origin + point(px(x - offset.get()), px(*y)),
                        (clip.start - y)..(clip.end - y).min(*h - 8.0),
                        theme,
                        store,
                        window,
                        cx,
                    )
                });
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
                        crate::layout::InlineAsset::Embed(key) => embeds.image(*key),
                        crate::layout::InlineAsset::Image(path) => store.image(path),
                    };
                    if let Some(image) = image {
                        let bounds =
                            Bounds::new(origin + point(px(x + ox), px(y + oy)), size(px(w), px(h)));
                        let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
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
                        let _ = window.paint_image(bounds, bounds, Corners::all(px(6.0)), image, 0, false);
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
