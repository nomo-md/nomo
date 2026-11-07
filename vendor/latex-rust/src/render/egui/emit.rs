//! `MathBox` → `egui::Shape`. Feature `egui` only.

use std::collections::HashMap;

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, TextureId, Vec2};
use epaint::{CornerRadius, RectShape, StrokeKind, Vertex};

use super::tessellate::{tessellate, GlyphTris};
use super::EguiOptions;
use crate::color::Color;
use crate::dim::Dim;
use crate::error::Error;
use crate::font::MathFont;
use crate::layout::{BoxContent, MathBox};

pub(super) fn shapes(
    tree: &MathBox,
    font: &MathFont,
    options: &EguiOptions,
    origin: Pos2,
    pixels_per_point: f32,
) -> Result<(Vec<Shape>, Rect), Error> {
    let ppp = Dim::from_ieee32_bits(pixels_per_point.to_bits());
    if ppp.is_nan() || ppp.is_zero() || is_neg(&ppp) {
        return Err(Error::InvalidOption {
            what: "pixels_per_point must be positive".into(),
        });
    }
    if options.font_size_pt.is_nan()
        || is_neg(&options.font_size_pt)
        || options.font_size_pt.is_zero()
    {
        return Err(Error::InvalidOption {
            what: "font_size_pt must be positive".into(),
        });
    }
    let em_px = &options.font_size_pt * &ppp;
    let fu_px = &em_px / &Dim::from_i64(i64::from(font.units_per_em()));
    let ox = Dim::from_ieee32_bits(origin.x.to_bits());
    let oy = Dim::from_ieee32_bits(origin.y.to_bits());
    let w = &tree.width * &em_px;
    let h = (&tree.height + &tree.depth) * &em_px;
    let rect = Rect::from_min_size(origin, Vec2::new(px(&w), px(&h)));
    let baseline = &oy + &(&tree.height * &em_px);
    let mut cache: HashMap<u16, GlyphTris> = HashMap::new();
    let mut out = Vec::new();
    emit(
        tree,
        font,
        &em_px,
        &fu_px,
        &ox,
        &baseline,
        options.color,
        &mut cache,
        &mut out,
    )?;
    Ok((out, rect))
}

fn is_neg(d: &Dim) -> bool {
    matches!(d.cmp(&Dim::zero()), Some(core::cmp::Ordering::Less))
}

fn px(d: &Dim) -> f32 {
    f32::from_bits(d.to_ieee32_bits())
}

fn color32(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

struct EguiTask<'a> {
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
    cache: &mut HashMap<u16, GlyphTris>,
    out: &mut Vec<Shape>,
) -> Result<(), Error> {
    let mut stack = vec![EguiTask {
        bx,
        origin_x: origin_x.clone(),
        parent_baseline: parent_baseline.clone(),
        fill,
    }];
    while let Some(EguiTask {
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
            out,
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
    cache: &mut HashMap<u16, GlyphTris>,
    out: &mut Vec<Shape>,
    stack: &mut Vec<EguiTask<'a>>,
) -> Result<(), Error> {
    let baseline = parent_baseline - &(bx.shift.clone() * em_px);
    match &bx.content {
        BoxContent::Empty | BoxContent::Kern(_) => Ok(()),
        BoxContent::Glyph {
            glyph_id, scale, outline, ..
        } => {
            if outline.is_some() { return Err(Error::Unsupported { what: "host glyph outlines require the SVG backend".into() }); }
            let tris = glyph_tris(font, *glyph_id, cache)?;
            let k = fu_px * scale;
            out.push(mesh_shape(tris, origin_x, &baseline, &k, fill));
            Ok(())
        }
        BoxContent::Rule => {
            push_fill_rect(bx, origin_x, &baseline, em_px, fill, out);
            Ok(())
        }
        BoxContent::HList(kids) => {
            let mut x = origin_x.clone();
            let mut placed = Vec::with_capacity(kids.len());
            for k in kids {
                placed.push((k, x.clone()));
                x = &x + &(k.width.clone() * em_px);
            }
            for (k, x) in placed.into_iter().rev() {
                stack.push(EguiTask {
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
                stack.push(EguiTask {
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
                stack.push(EguiTask {
                    bx: k,
                    origin_x: origin_x.clone(),
                    parent_baseline: baseline.clone(),
                    fill,
                });
            }
            Ok(())
        }
        BoxContent::Color(c, inner) => {
            stack.push(EguiTask {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill: *c,
            });
            Ok(())
        }
        BoxContent::BackColor(c, inner) => {
            push_fill_rect(bx, origin_x, &baseline, em_px, *c, out);
            stack.push(EguiTask {
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
            let stroke = Stroke::new(px(&(thickness.clone() * em_px)), color32(fill));
            out.push(Shape::line_segment(
                [Pos2::new(px(&x1p), px(&y1p)), Pos2::new(px(&x2p), px(&y2p))],
                stroke,
            ));
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
                let r = Rect::from_min_size(Pos2::new(px(&x), px(&y)), Vec2::new(px(&rw), px(&rh)));
                let border = stroke.unwrap_or(fill);
                // The rect is already inset by half the stroke, which is where a
                // centered stroke sat before `StrokeKind` existed.
                out.push(Shape::Rect(RectShape::stroke(
                    r,
                    CornerRadius::ZERO,
                    Stroke::new(px(&t), color32(border)),
                    StrokeKind::Middle,
                )));
            }
            stack.push(EguiTask {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill,
            });
            Ok(())
        }
    }
}

fn push_fill_rect(
    bx: &MathBox,
    origin_x: &Dim,
    baseline: &Dim,
    em_px: &Dim,
    fill: Color,
    out: &mut Vec<Shape>,
) {
    let w = &bx.width * em_px;
    let h_ink = (&bx.height + &bx.depth) * em_px;
    if w.is_zero() || h_ink.is_zero() {
        return;
    }
    let y = baseline - &(bx.height.clone() * em_px);
    let r = Rect::from_min_size(
        Pos2::new(px(origin_x), px(&y)),
        Vec2::new(px(&w), px(&h_ink)),
    );
    out.push(Shape::rect_filled(r, CornerRadius::ZERO, color32(fill)));
}

fn glyph_tris(
    font: &MathFont,
    glyph_id: u16,
    cache: &mut HashMap<u16, GlyphTris>,
) -> Result<GlyphTris, Error> {
    if let Some(t) = cache.get(&glyph_id) {
        return Ok(t.clone());
    }
    let t = tessellate(font, glyph_id)?;
    cache.insert(glyph_id, t.clone());
    Ok(t)
}

fn mesh_shape(tris: GlyphTris, origin_x: &Dim, baseline: &Dim, fu_px: &Dim, fill: Color) -> Shape {
    let col = color32(fill);
    let mut mesh = Mesh {
        indices: tris.indices,
        vertices: Vec::with_capacity(tris.vertices.len()),
        texture_id: TextureId::default(),
    };
    for (fx, fy) in tris.vertices {
        let x = origin_x + &(&fx * fu_px);
        let y = baseline - &(&fy * fu_px);
        mesh.vertices.push(Vertex {
            pos: Pos2::new(px(&x), px(&y)),
            uv: Pos2::ZERO,
            color: col,
        });
    }
    Shape::mesh(mesh)
}
