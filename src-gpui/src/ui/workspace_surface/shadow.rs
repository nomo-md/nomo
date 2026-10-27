//! 完整轮廓的双层高斯投影。只压缩远离圆角的直线中段，再拉伸这些恒定截面。
//! 单套 2x 图像最多 444×248 像素（约 0.42 MiB，切片护边略增）；不随窗口面积增长。
//! 替换时主动移除 GPUI 图集中的旧图片，窗口关闭时其余资源随窗口和 SurfaceState 释放。
use super::{Geometry, Segment, TabRect, TAB_RAIL_HEIGHT, surface_contour, visible_tab};
use crate::theme::Theme;
use gpui::{Bounds, ContentMask, Corners, Hsla, RenderImage, Rgba, Window, point, px, size};
use image::{Frame, GrayImage, RgbaImage};
use std::{fmt::Write, sync::Arc};

const SCALE: f32 = 2.0;
const PAD: f32 = 15.0; // 旧版 soft sigma = 5，保留两侧各 3 sigma。
const SPAN: f32 = 64.0; // 每端 31.5 px，超过圆角 12 + 阴影半径 15。

#[derive(Default)]
struct Axis {
    source: f32,
    destination: f32,
    cuts: Vec<(f32, f32)>,
}

impl Axis {
    fn new() -> Self { Self { cuts: vec![(0.0, 0.0)], ..Self::default() } }

    fn append(&mut self, length: f32, compress: bool) {
        if compress && length >= SPAN {
            let end = (SPAN - 1.0) / 2.0;
            self.source += end;
            self.destination += end;
            self.cuts.push((self.source, self.destination));
            self.source += 1.0;
            self.destination += length - 2.0 * end;
            self.cuts.push((self.source, self.destination));
            self.source += end;
            self.destination += end;
        } else {
            self.source += length;
            self.destination += length;
        }
    }

    fn finish(mut self) -> Vec<Slice> {
        let padding = (self.source * SCALE).ceil() / SCALE - self.source;
        self.append(padding, false);
        self.cuts.push((self.source, self.destination));
        self.cuts.windows(2).filter_map(|pair| {
            let start = (pair[0].0 * SCALE).round() as u32;
            let end = (pair[1].0 * SCALE).round() as u32;
            (end > start).then_some(Slice {
                start, end,
                from: pair[0].1 + start as f32 / SCALE - pair[0].0,
                to: pair[1].1 + end as f32 / SCALE - pair[1].0,
            })
        }).collect()
    }
}

#[derive(Clone, Copy)]
struct Slice {
    start: u32,
    end: u32,
    from: f32,
    to: f32,
}

struct Layout {
    geometry: Geometry,
    x: Vec<Slice>,
    y: Vec<Slice>,
}

impl Layout {
    fn new(geometry: Geometry) -> Self {
        let mut x = Axis::new();
        x.append(PAD, false);
        let width = f32::from(geometry.bounds.size.width);
        let tab = visible_tab(geometry).map(|tab| {
            let left = tab.left - f32::from(geometry.bounds.left());
            x.append(left, true);
            let tab_left = x.source - PAD;
            x.append(tab.width, true);
            let tab_width = x.source - PAD - tab_left;
            x.append((width - left - tab.width).max(0.0), true);
            TabRect { left: tab_left, width: tab_width }
        });
        if tab.is_none() { x.append(width, true); }
        let compact_width = x.source - PAD;
        x.append(PAD, false);
        let mut y = Axis::new();
        y.append(PAD + TAB_RAIL_HEIGHT, false);
        y.append(f32::from(geometry.bounds.size.height) - TAB_RAIL_HEIGHT, true);
        let compact_height = y.source - PAD;
        y.append(PAD, false);
        Self {
            geometry: Geometry { bounds: Bounds::new(point(px(0.0), px(0.0)), size(px(compact_width), px(compact_height))), tab },
            x: x.finish(), y: y.finish(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Key {
    geometry: Geometry,
    near: Hsla,
    soft: Hsla,
}

struct Tile {
    x: usize,
    y: usize,
    source: (u32, u32, u32, u32),
    image: Arc<RenderImage>,
}

#[derive(Default)]
pub(super) struct ShadowCache {
    key: Option<Key>,
    tiles: Vec<Tile>,
}

impl ShadowCache {
    pub(super) fn paint(&mut self, geometry: Geometry, theme: &Theme, window: &mut Window) {
        let layout = Layout::new(geometry);
        let key = Key { geometry: layout.geometry, near: theme.workspace_shadow_near, soft: theme.workspace_shadow_soft };
        if self.key != Some(key) {
            for tile in self.tiles.drain(..) {
                if let Err(error) = window.drop_image(tile.image) { eprintln!("工作区阴影释放失败: {error}"); }
            }
            // 失败也记住当前键，避免每帧重复分配和刷日志；尺寸或主题变化后再尝试。
            self.key = Some(key);
            match rasterize(key) {
                Ok(raster) => self.tiles = tiles(&raster, &layout),
                Err(error) => eprintln!("工作区阴影绘制失败: {error}"),
            }
        }
        let origin = geometry.bounds.origin - point(px(PAD), px(PAD));
        let scale = window.scale_factor();
        let snap = |value: gpui::Pixels| px((f32::from(value) * scale).round() / scale);
        for tile in &self.tiles {
            let x = layout.x[tile.x];
            let y = layout.y[tile.y];
            let clip = Bounds::from_corners(
                point(snap(origin.x + px(x.from)), snap(origin.y + px(y.from))),
                point(snap(origin.x + px(x.to)), snap(origin.y + px(y.to))),
            );
            // 裁剪后不可见的切片不提交；圆角补绘通常只需要一个切片。
            if clip.intersect(&window.content_mask().bounds).is_empty() { continue; }
            let sx = f32::from(clip.size.width) / (x.end - x.start) as f32;
            let sy = f32::from(clip.size.height) / (y.end - y.start) as f32;
            let (left, top, width, height) = tile.source;
            let bounds = Bounds::new(
                clip.origin - point(px((x.start - left) as f32 * sx), px((y.start - top) as f32 * sy)),
                size(px(width as f32 * sx), px(height as f32 * sy)),
            );
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                if let Err(error) = window.paint_image(bounds, bounds, Corners::default(), tile.image.clone(), 0, false) {
                    eprintln!("工作区阴影上传失败: {error}");
                }
            });
        }
    }
}

fn tiles(raster: &RgbaImage, layout: &Layout) -> Vec<Tile> {
    let mut tiles = Vec::new();
    for (y, row) in layout.y.iter().enumerate() {
        for (x, column) in layout.x.iter().enumerate() {
            // 一像素护边供双线性采样读取相邻颜色，避免切片边界出现亮缝。
            let left = column.start.saturating_sub(1);
            let top = row.start.saturating_sub(1);
            let width = (column.end + 1).min(raster.width()) - left;
            let height = (row.end + 1).min(raster.height()) - top;
            let mut buffer = image::imageops::crop_imm(raster, left, top, width, height).to_image();
            for pixel in buffer.pixels_mut() { pixel.0.swap(0, 2); } // 预乘 RGBA → GPUI BGRA。
            tiles.push(Tile { x, y, source: (left, top, width, height), image: Arc::new(RenderImage::new(vec![Frame::new(buffer)])) });
        }
    }
    tiles
}

fn rasterize(key: Key) -> Result<RgbaImage, String> {
    let width = f32::from(key.geometry.bounds.size.width) + PAD * 2.0;
    let height = f32::from(key.geometry.bounds.size.height) + PAD * 2.0;
    let path = svg_path(key.geometry);
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="-{PAD} -{PAD} {width} {height}"><path d="{path}" fill="white"/></svg>"#);
    let silhouette = render_svg(&svg, width, height)?;
    let mask = GrayImage::from_raw(silhouette.width(), silhouette.height(), silhouette.pixels().map(|pixel| pixel.0[3]).collect()).ok_or("阴影蒙版无效")?;
    // 两层都从完整轮廓的 Alpha 生成。近阴影采用高斯卷积，宽阴影采用线性成本的
    // 高斯近似，避免 SVG 滤镜对四个颜色通道反复模糊、分配和合成阻塞标签动画。
    let near = image::imageops::blur(&mask, 0.8 * SCALE);
    let soft = image::imageops::fast_blur(&mask, 5.0 * SCALE);
    let near_color: Rgba = key.near.into();
    let soft_color: Rgba = key.soft.into();
    Ok(RgbaImage::from_fn(mask.width(), mask.height(), |x, y| {
        let near_alpha = f32::from(near.get_pixel(x, y).0[0]) / 255.0 * near_color.a;
        let soft_alpha = f32::from(soft.get_pixel(x, y).0[0]) / 255.0 * soft_color.a * (1.0 - near_alpha);
        let channel = |near: f32, soft: f32| ((near * near_alpha + soft * soft_alpha) * 255.0).round() as u8;
        image::Rgba([
            channel(near_color.r, soft_color.r), channel(near_color.g, soft_color.g),
            channel(near_color.b, soft_color.b), ((near_alpha + soft_alpha) * 255.0).round() as u8,
        ])
    }))
}

fn svg_path(geometry: Geometry) -> String {
    let mut path = String::new();
    for segment in surface_contour(geometry) {
        let _ = match segment {
            Segment::Move(x, y) => write!(path, "M{x} {y} "),
            Segment::Line(x, y) => write!(path, "L{x} {y} "),
            Segment::Arc(radius, sweep, x, y) => write!(path, "A{radius} {radius} 0 0 {} {x} {y} ", u8::from(sweep)),
            Segment::Close => write!(path, "Z"),
        };
    }
    path
}

fn render_svg(svg: &str, width: f32, height: f32) -> Result<RgbaImage, String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|error| error.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new((width * SCALE).ceil() as u32, (height * SCALE).ceil() as u32).ok_or("阴影位图尺寸无效")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pixmap.as_mut());
    RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take()).ok_or_else(|| "阴影位图数据无效".into())
}

#[cfg(test)]
fn rasterize_reference(key: Key) -> Result<RgbaImage, String> {
    let width = f32::from(key.geometry.bounds.size.width) + PAD * 2.0;
    let height = f32::from(key.geometry.bounds.size.height) + PAD * 2.0;
    let path = svg_path(key.geometry);
    let color = |color: Hsla| {
        let color: Rgba = color.into();
        format!("flood-color=\"rgb({},{},{})\" flood-opacity=\"{}\"", color.r * 255.0, color.g * 255.0, color.b * 255.0, color.a)
    };
    // 独立参照：旧 WebView 的完整 SVG 滤镜，不压缩、不使用蒙版快速模糊。
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="-{PAD} -{PAD} {width} {height}">
      <defs><filter id="shadow" filterUnits="userSpaceOnUse" x="-{PAD}" y="-{PAD}" width="{width}" height="{height}" color-interpolation-filters="sRGB">
        <feGaussianBlur in="SourceAlpha" stdDeviation="0.8" result="near-alpha"/>
        <feFlood {} result="near-color"/><feComposite in="near-color" in2="near-alpha" operator="in" result="near"/>
        <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="soft-alpha"/>
        <feFlood {} result="soft-color"/><feComposite in="soft-color" in2="soft-alpha" operator="in" result="soft"/>
        <feMerge><feMergeNode in="soft"/><feMergeNode in="near"/></feMerge>
      </filter></defs><path d="{path}" fill="white" filter="url(#shadow)"/></svg>"#, color(key.near), color(key.soft));
    render_svg(&svg, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(width: f32, height: f32, tab: Option<(f32, f32)>) -> Geometry {
        Geometry {
            bounds: Bounds::new(point(px(0.0), px(0.0)), size(px(width), px(height))),
            tab: tab.map(|(left, width)| TabRect { left, width }),
        }
    }

    fn key(geometry: Geometry, theme: &Theme) -> Key {
        Key { geometry, near: theme.workspace_shadow_near, soft: theme.workspace_shadow_soft }
    }

    fn expanded(layout: &Layout, raster: &RgbaImage) -> RgbaImage {
        let width = (layout.x.last().unwrap().to * SCALE).round() as u32;
        let height = (layout.y.last().unwrap().to * SCALE).round() as u32;
        let mut output = RgbaImage::new(width, height);
        for row in &layout.y {
            for column in &layout.x {
                let tile = image::imageops::crop_imm(raster, column.start, row.start, column.end - column.start, row.end - row.start).to_image();
                let left = (column.from * SCALE).round() as u32;
                let top = (row.from * SCALE).round() as u32;
                let width = (column.to * SCALE).round() as u32 - left;
                let height = (row.to * SCALE).round() as u32 - top;
                let tile = image::imageops::resize(&tile, width, height, image::imageops::FilterType::Triangle);
                image::imageops::replace(&mut output, &tile, left.into(), top.into());
            }
        }
        output
    }

    #[test]
    fn compressed_shadow_matches_full_contour_gaussian() {
        // 首标签、中间标签、左右滚动裁切、空工作区、极窄工作区。
        for (index, geometry) in [
            geometry(640.0, 300.0, Some((18.0, 180.0))),
            geometry(640.0, 300.0, Some((270.0, 180.0))),
            geometry(640.0, 300.0, Some((-60.0, 180.0))),
            geometry(640.0, 300.0, Some((580.0, 180.0))),
            geometry(640.0, 300.0, None),
            geometry(19.0, 40.0, Some((0.0, 19.0))),
        ].into_iter().enumerate() {
            for theme in [Theme::light(), Theme::dark()] {
                let layout = Layout::new(geometry);
                let compact = rasterize(key(layout.geometry, &theme)).unwrap();
                let actual = expanded(&layout, &compact);
                let expected = rasterize_reference(key(geometry, &theme)).unwrap();
                assert_eq!(actual.dimensions(), expected.dimensions());
                let max_error = actual.as_raw().iter().zip(expected.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                // SVG 滤镜与快速高斯近似的量化差异：浅色不超过 1/255，深色不超过 3/255。
                assert!(max_error <= if theme.dark { 3 } else { 1 }, "case {index}, dark={}: max channel error {max_error}", theme.dark);
                // 整条左边的阴影必须存在且平稳；不能再在圆角下方被截掉。
                if index < 5 {
                    let alpha = actual.get_pixel(((PAD - 2.0) * SCALE) as u32, (180.0 * SCALE) as u32).0[3];
                    assert!(alpha > 0, "missing left shadow");
                }
            }
        }
    }

    #[test]
    fn shadow_cache_is_bounded_independent_of_window_size() {
        let geometry = geometry(16_384.0, 8_192.0, Some((8_000.0, 180.0)));
        let layout = Layout::new(geometry);
        let raster = rasterize(key(layout.geometry, &Theme::light())).unwrap();
        assert!(raster.width() <= 444 && raster.height() <= 248);
        let tiles = tiles(&raster, &layout);
        let bytes: usize = tiles.iter().map(|tile| tile.image.as_bytes(0).unwrap().len()).sum();
        assert!(bytes <= 512 * 1024, "shadow retained {bytes} bytes");
        assert!(tiles.len() <= 21);
    }

    #[test]
    fn identical_cache_keys_have_identical_slice_boundaries() {
        // 长度刚好达到压缩阈值时也必须使用相同切片，否则缓存命中后索引会错位。
        let a = Layout::new(geometry(192.0, 94.0, Some((64.0, 64.0))));
        let b = Layout::new(geometry(900.0, 600.0, Some((300.0, 180.0))));
        assert!(a.geometry == b.geometry);
        let sources = |slices: &[Slice]| slices.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>();
        assert_eq!(sources(&a.x), sources(&b.x));
        assert_eq!(sources(&a.y), sources(&b.y));
    }
}
