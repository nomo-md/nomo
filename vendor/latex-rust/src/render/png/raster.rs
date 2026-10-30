//! `MathBox` → PNG via `tiny-skia`. Feature `png` only.
//!
//! Pixel coordinates are [`Dim`](crate::Dim) until the tiny-skia call, then
//! IEEE-754 bits via [`Dim::to_ieee32_bits`]. No layout `f32`/`f64` arithmetic.
//!
//! SIMD is automatic: AVX2 on x86_64, NEON on Apple Silicon. No compile flags.

use std::collections::HashMap;

use skrifa::outline::OutlinePen;
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::{PngBackground, PngOptions};
use crate::color::Color;
use crate::dim::Dim;
use crate::error::{Error, FontError};
use crate::font::MathFont;
use crate::layout::{BoxContent, MathBox};

pub(super) fn render(
    tree: &MathBox,
    font: &MathFont,
    options: &PngOptions,
) -> Result<Vec<u8>, Error> {
    validate_options(options)?;
    let em_px = pixels_per_em(&options.font_size_pt, &options.dpi);
    let fu_px = &em_px / &Dim::from_i64(i64::from(font.units_per_em()));
    let w_dim = &tree.width * &em_px;
    let h_dim = (&tree.height + &tree.depth) * &em_px;
    let w = w_dim.ceil_to_u32()?.max(1);
    let h = h_dim.ceil_to_u32()?.max(1);
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| Error::InvalidOption {
        what: format!("png pixmap {w}x{h}"),
    })?;
    fill_background(&mut pixmap, options.background);
    let baseline = &tree.height * &em_px;
    let mut cache: HashMap<u16, Path> = HashMap::new();
    emit(
        tree,
        font,
        &em_px,
        &fu_px,
        &Dim::zero(),
        &baseline,
        options.color,
        &mut cache,
        &mut pixmap,
    )?;
    pixmap.encode_png().map_err(|_| Error::Unsupported {
        what: "png encode".into(),
    })
}

fn validate_options(options: &PngOptions) -> Result<(), Error> {
    if options.dpi.is_nan() || options.font_size_pt.is_nan() {
        return Err(Error::InvalidOption {
            what: "png dpi or font size is NaN".into(),
        });
    }
    if options.dpi.is_zero()
        || matches!(
            options.dpi.cmp(&Dim::zero()),
            Some(core::cmp::Ordering::Less) | Some(core::cmp::Ordering::Equal)
        )
    {
        return Err(Error::InvalidOption {
            what: "png dpi must be positive".into(),
        });
    }
    if matches!(
        options.dpi.cmp(&Dim::from_i64(2400)),
        Some(core::cmp::Ordering::Greater)
    ) {
        return Err(Error::InvalidOption {
            what: "png dpi exceeds 2400".into(),
        });
    }
    Ok(())
}

fn pixels_per_em(font_size_pt: &Dim, dpi: &Dim) -> Dim {
    font_size_pt * dpi / Dim::from_i64(72)
}

fn fill_background(pixmap: &mut Pixmap, bg: PngBackground) {
    match bg {
        PngBackground::Transparent => {}
        PngBackground::White => pixmap.fill(tiny_skia::Color::WHITE),
        PngBackground::Color(c) => pixmap.fill(skia_color(c, 255)),
    }
}

fn skia_color(c: Color, a: u8) -> tiny_skia::Color {
    let [r, g, b, _] = c.to_rgba8();
    tiny_skia::Color::from_rgba8(r, g, b, a)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(skia_color(c, 255));
    p.anti_alias = true;
    p
}

fn px(d: &Dim) -> f32 {
    f32::from_bits(d.to_ieee32_bits())
}

struct PngTask<'a> {
    bx: &'a MathBox,
    origin_x: Dim,
    parent_baseline: Dim,
    fill: Color,
}

#[allow(clippy::too_many_arguments)]
fn emit(
    bx: &MathBox,
    font: &MathFont,
    em_px: &Dim,
    fu_px: &Dim,
    origin_x: &Dim,
    parent_baseline: &Dim,
    fill: Color,
    cache: &mut HashMap<u16, Path>,
    pixmap: &mut Pixmap,
) -> Result<(), Error> {
    let mut stack = vec![PngTask {
        bx,
        origin_x: origin_x.clone(),
        parent_baseline: parent_baseline.clone(),
        fill,
    }];
    while let Some(PngTask {
        bx,
        origin_x,
        parent_baseline,
        fill,
    }) = stack.pop()
    {
        emit_one(
            bx,
            font,
            em_px,
            fu_px,
            &origin_x,
            &parent_baseline,
            fill,
            cache,
            pixmap,
            &mut stack,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_one<'a>(
    bx: &'a MathBox,
    font: &MathFont,
    em_px: &Dim,
    fu_px: &Dim,
    origin_x: &Dim,
    parent_baseline: &Dim,
    fill: Color,
    cache: &mut HashMap<u16, Path>,
    pixmap: &mut Pixmap,
    stack: &mut Vec<PngTask<'a>>,
) -> Result<(), Error> {
    let baseline = parent_baseline - &(bx.shift.clone() * em_px);
    match &bx.content {
        BoxContent::Empty | BoxContent::Kern(_) => Ok(()),
        BoxContent::Glyph {
            glyph_id, scale, outline, ..
        } => {
            if outline.is_some() { return Err(Error::Unsupported { what: "host glyph outlines require the SVG backend".into() }); }
            let path = glyph_path(font, *glyph_id, cache)?;
            let k = fu_px * scale;
            let sx = px(&k);
            let sy = px(&-k);
            let tx = px(origin_x);
            let ty = px(&baseline);
            let t = Transform::from_row(sx, 0.0, 0.0, sy, tx, ty);
            pixmap.fill_path(&path, &paint(fill), FillRule::Winding, t, None);
            Ok(())
        }
        BoxContent::Rule => fill_box_rect(bx, origin_x, &baseline, em_px, fill, pixmap),
        BoxContent::HList(kids) => {
            let mut x = origin_x.clone();
            let mut placed = Vec::with_capacity(kids.len());
            for k in kids {
                placed.push((k, x.clone()));
                x = &x + &(k.width.clone() * em_px);
            }
            for (k, x) in placed.into_iter().rev() {
                stack.push(PngTask {
                    bx: k,
                    origin_x: x,
                    parent_baseline: baseline.clone(),
                    fill,
                });
            }
            Ok(())
        }
        BoxContent::VList(kids) => {
            if kids.is_empty() {
                return Ok(());
            }
            let mut placed = Vec::with_capacity(kids.len());
            placed.push((&kids[0], baseline.clone()));
            let mut y_below = kids[0].depth.clone() * em_px;
            for k in kids.iter().skip(1) {
                let child_base = &baseline + &y_below + &(k.height.clone() * em_px);
                placed.push((k, child_base));
                y_below = &y_below + &((&k.height + &k.depth) * em_px);
            }
            for (k, child_base) in placed.into_iter().rev() {
                stack.push(PngTask {
                    bx: k,
                    origin_x: origin_x.clone(),
                    parent_baseline: child_base,
                    fill,
                });
            }
            Ok(())
        }
        BoxContent::Overlap(kids) => {
            for k in kids.iter().rev() {
                stack.push(PngTask {
                    bx: k,
                    origin_x: origin_x.clone(),
                    parent_baseline: baseline.clone(),
                    fill,
                });
            }
            Ok(())
        }
        BoxContent::Color(c, inner) => {
            stack.push(PngTask {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill: *c,
            });
            Ok(())
        }
        BoxContent::BackColor(c, inner) => {
            fill_box_rect(bx, origin_x, &baseline, em_px, *c, pixmap)?;
            stack.push(PngTask {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill,
            });
            Ok(())
        }
        BoxContent::Line {
            x1,
            y1,
            x2,
            y2,
            thickness,
        } => {
            let x1p = origin_x + &(x1.clone() * em_px);
            let y1p = &baseline - &(y1.clone() * em_px);
            let x2p = origin_x + &(x2.clone() * em_px);
            let y2p = &baseline - &(y2.clone() * em_px);
            let mut pb = PathBuilder::new();
            pb.move_to(px(&x1p), px(&y1p));
            pb.line_to(px(&x2p), px(&y2p));
            let Some(path) = pb.finish() else {
                return Ok(());
            };
            let stroke = Stroke {
                width: px(&(thickness.clone() * em_px)),
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint(fill), &stroke, Transform::identity(), None);
            Ok(())
        }
        BoxContent::Frame {
            thickness,
            stroke,
            inner,
        } => {
            let w = &bx.width * em_px;
            let h_ink = (&bx.height + &bx.depth) * em_px;
            let t = thickness.clone() * em_px;
            let half = &t / &Dim::from_i64(2);
            let x = origin_x + &half;
            let y = &(&baseline - &(bx.height.clone() * em_px)) + &half;
            let rw = (&w - &t).max(&Dim::zero());
            let rh = (&h_ink - &t).max(&Dim::zero());
            if !rw.is_zero() && !rh.is_zero() {
                if let Some(rect) = Rect::from_xywh(px(&x), px(&y), px(&rw), px(&rh)) {
                    let st = Stroke {
                        width: px(&t),
                        ..Stroke::default()
                    };
                    let border = stroke.unwrap_or(fill);
                    pixmap.stroke_path(
                        &PathBuilder::from_rect(rect),
                        &paint(border),
                        &st,
                        Transform::identity(),
                        None,
                    );
                }
            }
            stack.push(PngTask {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill,
            });
            Ok(())
        }
    }
}

fn fill_box_rect(
    bx: &MathBox,
    origin_x: &Dim,
    baseline: &Dim,
    em_px: &Dim,
    fill: Color,
    pixmap: &mut Pixmap,
) -> Result<(), Error> {
    let w = &bx.width * em_px;
    let h_ink = (&bx.height + &bx.depth) * em_px;
    if w.is_zero() || h_ink.is_zero() {
        return Ok(());
    }
    let y = baseline - &(bx.height.clone() * em_px);
    if let Some(rect) = Rect::from_xywh(px(origin_x), px(&y), px(&w), px(&h_ink)) {
        pixmap.fill_rect(rect, &paint(fill), Transform::identity(), None);
    }
    Ok(())
}

fn glyph_path(
    font: &MathFont,
    glyph_id: u16,
    cache: &mut HashMap<u16, Path>,
) -> Result<Path, Error> {
    if let Some(p) = cache.get(&glyph_id) {
        return Ok(p.clone());
    }
    let path = outline_path(font, glyph_id)?;
    cache.insert(glyph_id, path.clone());
    Ok(path)
}

fn outline_path(font: &MathFont, glyph_id: u16) -> Result<Path, Error> {
    let mut b = DimOutline {
        pb: PathBuilder::new(),
    };
    if !font.draw_glyph(glyph_id, &mut b) {
        return Err(FontError::MissingGlyph { ch: '\u{FFFD}' }.into());
    }
    b.pb.finish()
        .ok_or_else(|| FontError::MissingGlyph { ch: '\u{FFFD}' }.into())
}

/// TrueType `f32` points become [`Dim`] immediately, then IEEE bits for tiny-skia.
struct DimOutline {
    pb: PathBuilder,
}

impl DimOutline {
    fn pt(x: f32) -> f32 {
        f32::from_bits(Dim::from_ieee32_bits(x.to_bits()).to_ieee32_bits())
    }
}

impl OutlinePen for DimOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(Self::pt(x), Self::pt(y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(Self::pt(x), Self::pt(y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb
            .quad_to(Self::pt(x1), Self::pt(y1), Self::pt(x), Self::pt(y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            Self::pt(x1),
            Self::pt(y1),
            Self::pt(x2),
            Self::pt(y2),
            Self::pt(x),
            Self::pt(y),
        );
    }

    fn close(&mut self) {
        self.pb.close();
    }
}
