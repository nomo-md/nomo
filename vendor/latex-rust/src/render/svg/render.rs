//! `MathBox` tree → self-contained SVG 1.1.

use std::collections::HashMap;

use super::outline;
use crate::color::Color;
use crate::dim::Dim;
use crate::error::{Error, FontError};
use crate::font::MathFont;
use crate::layout::{layout_with_em_size_pt, BoxContent, MathBox, MathStyle};
use crate::parser::parse;

/// Options for [`render_svg`].
///
/// # Examples
///
/// ```
/// use latex_rust::{Color, Dim, SvgOptions};
///
/// let mut opt = SvgOptions::new();
/// opt.font_size_pt = Dim::from_i64(12);
/// opt.color = Color::rgb(0, 0, 0);
/// opt.display = false;
/// ```
#[derive(Clone, Debug)]
pub struct SvgOptions {
    /// Em size in points (SVG `width`/`height` use `pt`).
    pub font_size_pt: Dim,
    /// Default glyph fill.
    pub color: Color,
    /// When using [`latex_to_svg`], pick display vs text style.
    pub display: bool,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self {
            font_size_pt: Dim::from_i64(12),
            color: Color::rgb(0, 0, 0),
            display: false,
        }
    }
}

impl SvgOptions {
    /// 12 pt, black fill, text style.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse, lay out, and render `latex` to a self-contained SVG document.
///
/// Glyphs are `<path>` elements from STIX Two Math outlines. Rules are `<rect>`.
///
/// # Arguments
///
/// * `latex` — math source (see [`crate::parse()`]).
/// * `font` — face used for layout and outlines.
/// * `options` — em size, default fill, and display vs text style.
///
/// # Returns
///
/// An SVG 1.1 document string.
///
/// # Errors
///
/// * [`Error::Parse`] — tokenizer or parser rejected `latex`.
/// * [`Error::Font`] — a required glyph is missing.
/// * [`Error::Unsupported`] — construct this renderer will not invent.
///
/// # Examples
///
/// ```
/// use latex_rust::{latex_to_svg, MathFont, SvgOptions};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let svg = latex_to_svg(r"\frac{1}{2}", &font, &SvgOptions::new()).unwrap();
/// assert!(svg.contains("<svg"));
/// ```
pub fn latex_to_svg(latex: &str, font: &MathFont, options: &SvgOptions) -> Result<String, Error> {
    let ast = parse(latex)?;
    let style = if options.display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let tree = layout_with_em_size_pt(&ast, font, style, &options.font_size_pt)?;
    render_svg(&tree, font, options)
}

/// Render a laid-out [`MathBox`] to a self-contained SVG document.
///
/// # Arguments
///
/// * `tree` — box model from [`crate::layout()`].
/// * `font` — face that supplied the glyph ids on `tree`.
/// * `options` — em size and default fill (`display` is ignored here).
///
/// # Returns
///
/// An SVG 1.1 document string.
///
/// # Errors
///
/// * [`Error::Font`] — outline missing for a glyph id on `tree`.
/// * [`Error::Unsupported`] — a box the SVG backend cannot emit.
///
/// # Examples
///
/// ```
/// use latex_rust::{layout, parse, render_svg, MathFont, MathStyle, SvgOptions};
///
/// let ast = parse(r"x").unwrap();
/// let font = MathFont::stix_two_math().unwrap();
/// let tree = layout(&ast, &font, MathStyle::Text).unwrap();
/// let svg = render_svg(&tree, &font, &SvgOptions::new()).unwrap();
/// assert!(svg.contains("<path"));
/// ```
pub fn render_svg(tree: &MathBox, font: &MathFont, options: &SvgOptions) -> Result<String, Error> {
    Ok(render_svg_with_metrics(tree, font, options)?.svg)
}

/// Complete physical image metrics in points. Baseline is measured down from
/// the top of the image, independently of the SVG coordinate origin.
#[derive(Clone, Debug)]
pub struct SvgImage {
    /// Self-contained SVG artwork.
    pub svg: String,
    /// Physical viewport width in points.
    pub width_pt: Dim,
    /// Physical viewport height in points.
    pub height_pt: Dim,
    /// Distance from viewport top to mathematical baseline in points.
    pub baseline_pt: Dim,
}

/// Render with explicit baseline and an ink-containing viewport.
pub fn render_svg_with_metrics(
    tree: &MathBox,
    font: &MathFont,
    options: &SvgOptions,
) -> Result<SvgImage, Error> {
    let em_pt = options.font_size_pt.clone();
    let fu_pt = &em_pt / &Dim::from_i64(i64::from(font.units_per_em()));
    let svg_w = &tree.width * &em_pt;
    let svg_h = (&tree.height + &tree.depth) * &em_pt;
    let baseline = &tree.height * &em_pt;
    let mut cache: HashMap<u16, String> = HashMap::new();
    let mut body = String::new();
    let mut bounds = [Dim::zero(), Dim::zero(), svg_w.clone(), svg_h.clone()];
    emit(
        tree,
        font,
        &em_pt,
        &fu_pt,
        &Dim::zero(),
        &baseline,
        options.color,
        &mut cache,
        &mut body,
        &mut bounds,
    )?;
    let width = &bounds[2] - &bounds[0];
    let height = &bounds[3] - &bounds[1];
    let mut out = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}pt" height="{}pt" viewBox="{} {} {} {}">"#,
        width.to_svg_string(), height.to_svg_string(),
        bounds[0].to_svg_string(), bounds[1].to_svg_string(),
        width.to_svg_string(), height.to_svg_string()
    ));
    out.push('\n');
    out.push_str(r#"<g fill=""#);
    out.push_str(&options.color.css_hex());
    out.push_str("\">\n");
    out.push_str(&body);
    out.push_str("</g>\n</svg>\n");
    Ok(SvgImage {
        svg: out,
        width_pt: width,
        height_pt: height,
        baseline_pt: &baseline - &bounds[1],
    })
}

enum SvgTask<'a> {
    Draw {
        bx: &'a MathBox,
        origin_x: Dim,
        parent_baseline: Dim,
        fill: Color,
    },
    CloseGroup,
}

#[allow(clippy::too_many_arguments)]
fn emit(
    bx: &MathBox,
    font: &MathFont,
    em_pt: &Dim,
    fu_pt: &Dim,
    origin_x: &Dim,
    parent_baseline: &Dim,
    fill: Color,
    cache: &mut HashMap<u16, String>,
    out: &mut String,
    bounds: &mut [Dim; 4],
) -> Result<(), Error> {
    let mut stack = vec![SvgTask::Draw {
        bx,
        origin_x: origin_x.clone(),
        parent_baseline: parent_baseline.clone(),
        fill,
    }];
    while let Some(task) = stack.pop() {
        match task {
            SvgTask::CloseGroup => out.push_str("</g>\n"),
            SvgTask::Draw {
                bx,
                origin_x,
                parent_baseline,
                fill,
            } => {
                let baseline = &parent_baseline - &(&bx.shift * em_pt);
                let rect = match &bx.content {
                    BoxContent::Glyph {
                        glyph_id, scale, outline: None, ..
                    } => font.outline_bounds(*glyph_id).map(|b| {
                        let k = fu_pt * scale;
                        [
                            &origin_x + &(&b[0] * &k),
                            &baseline - &(&b[3] * &k),
                            &origin_x + &(&b[2] * &k),
                            &baseline - &(&b[1] * &k),
                        ]
                    }),
                    // Host outlines are not glyph 0 in the math face. Their
                    // measured vertical metrics and advance are already in the box.
                    BoxContent::Glyph { outline: Some(_), .. } => Some([
                        origin_x.clone(),
                        &baseline - &(&bx.height * em_pt),
                        &origin_x + &(&bx.width * em_pt),
                        &baseline + &(&bx.depth * em_pt),
                    ]),
                    BoxContent::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        thickness,
                    } => {
                        let half = (thickness * em_pt) / Dim::from_i64(2);
                        Some([
                            &origin_x + &(x1.min(x2) * em_pt) - &half,
                            &baseline - &(y1.max(y2) * em_pt) - &half,
                            &origin_x + &(x1.max(x2) * em_pt) + &half,
                            &baseline - &(y1.min(y2) * em_pt) + &half,
                        ])
                    }
                    BoxContent::Rule | BoxContent::Frame { .. } | BoxContent::BackColor(..) => {
                        Some([
                            origin_x.clone(),
                            &baseline - &(&bx.height * em_pt),
                            &origin_x + &(&bx.width * em_pt),
                            &baseline + &(&bx.depth * em_pt),
                        ])
                    }
                    _ => None,
                };
                if let Some(rect) = rect {
                    bounds[0] = bounds[0].min(&rect[0]);
                    bounds[1] = bounds[1].min(&rect[1]);
                    bounds[2] = bounds[2].max(&rect[2]);
                    bounds[3] = bounds[3].max(&rect[3]);
                }
                emit_one(
                    bx,
                    font,
                    em_pt,
                    fu_pt,
                    &origin_x,
                    &parent_baseline,
                    fill,
                    cache,
                    out,
                    &mut stack,
                )?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_one<'a>(
    bx: &'a MathBox,
    font: &MathFont,
    em_pt: &Dim,
    fu_pt: &Dim,
    origin_x: &Dim,
    parent_baseline: &Dim,
    fill: Color,
    cache: &mut HashMap<u16, String>,
    out: &mut String,
    stack: &mut Vec<SvgTask<'a>>,
) -> Result<(), Error> {
    let baseline = parent_baseline - &(bx.shift.clone() * em_pt);
    match &bx.content {
        BoxContent::Empty | BoxContent::Kern(_) => Ok(()),
        BoxContent::Glyph {
            ch,
            glyph_id,
            scale,
            outline,
        } => {
            let d = if let Some(outline) = outline {
                outline.to_string()
            } else if let Some(s) = cache.get(glyph_id) {
                s.clone()
            } else {
                let s = outline::glyph_path_d(font, *glyph_id)?;
                if s.is_empty() {
                    return Err(FontError::MissingGlyph { ch: *ch }.into());
                }
                cache.insert(*glyph_id, s.clone());
                s
            };
            let k = fu_pt * scale;
            let sx = k.to_svg_string();
            let nsx = (-k).to_svg_string();
            out.push_str(&format!(
                r#"<path d="{d}" transform="translate({} {}) scale({sx} {nsx})"/>"#,
                origin_x.to_svg_string(),
                baseline.to_svg_string(),
            ));
            out.push('\n');
            Ok(())
        }
        BoxContent::Rule => {
            let w = &bx.width * em_pt;
            let h_ink = (&bx.height + &bx.depth) * em_pt;
            if w.is_zero() || h_ink.is_zero() {
                return Ok(());
            }
            let y = &baseline - &(bx.height.clone() * em_pt);
            out.push_str(&format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}"/>"#,
                origin_x.to_svg_string(),
                y.to_svg_string(),
                w.to_svg_string(),
                h_ink.to_svg_string()
            ));
            out.push('\n');
            Ok(())
        }
        BoxContent::HList(kids) => {
            let mut x = origin_x.clone();
            let mut placed = Vec::with_capacity(kids.len());
            for k in kids {
                placed.push((k, x.clone()));
                x = &x + &(k.width.clone() * em_pt);
            }
            for (k, x) in placed.into_iter().rev() {
                stack.push(SvgTask::Draw {
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
            let mut y_below = kids[0].depth.clone() * em_pt;
            for k in kids.iter().skip(1) {
                let child_base = &baseline + &y_below + &(k.height.clone() * em_pt);
                placed.push((k, child_base));
                y_below = &y_below + &((&k.height + &k.depth) * em_pt);
            }
            for (k, child_base) in placed.into_iter().rev() {
                stack.push(SvgTask::Draw {
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
                stack.push(SvgTask::Draw {
                    bx: k,
                    origin_x: origin_x.clone(),
                    parent_baseline: baseline.clone(),
                    fill,
                });
            }
            Ok(())
        }
        BoxContent::Color(c, inner) => {
            out.push_str(&format!(
                r#"<g fill="{hex}" stroke="{hex}">"#,
                hex = c.css_hex()
            ));
            out.push('\n');
            stack.push(SvgTask::CloseGroup);
            stack.push(SvgTask::Draw {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill: *c,
            });
            Ok(())
        }
        BoxContent::BackColor(c, inner) => {
            let w = &bx.width * em_pt;
            let h_ink = (&bx.height + &bx.depth) * em_pt;
            let y = &baseline - &(bx.height.clone() * em_pt);
            out.push_str(&format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                origin_x.to_svg_string(),
                y.to_svg_string(),
                w.to_svg_string(),
                h_ink.to_svg_string(),
                c.css_hex()
            ));
            out.push('\n');
            stack.push(SvgTask::Draw {
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
            let x1p = origin_x + &(x1.clone() * em_pt);
            let y1p = &baseline - &(y1.clone() * em_pt);
            let x2p = origin_x + &(x2.clone() * em_pt);
            let y2p = &baseline - &(y2.clone() * em_pt);
            let sw = thickness.clone() * em_pt;
            out.push_str(&format!(
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}"/>"#,
                x1p.to_svg_string(),
                y1p.to_svg_string(),
                x2p.to_svg_string(),
                y2p.to_svg_string(),
                fill.css_hex(),
                sw.to_svg_string(),
            ));
            out.push('\n');
            Ok(())
        }
        BoxContent::Frame {
            thickness,
            stroke,
            inner,
        } => {
            let w = &bx.width * em_pt;
            let h_ink = (&bx.height + &bx.depth) * em_pt;
            let t = thickness.clone() * em_pt;
            let half = &t / &Dim::from_i64(2);
            let x = origin_x + &half;
            let y = &(&baseline - &(bx.height.clone() * em_pt)) + &half;
            let rw = (&w - &t).max(&Dim::zero());
            let rh = (&h_ink - &t).max(&Dim::zero());
            let stroke_css = stroke.unwrap_or(fill).css_hex();
            if !rw.is_zero() && !rh.is_zero() {
                out.push_str(&format!(
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="{}"/>"#,
                    x.to_svg_string(),
                    y.to_svg_string(),
                    rw.to_svg_string(),
                    rh.to_svg_string(),
                    stroke_css,
                    t.to_svg_string(),
                ));
                out.push('\n');
            }
            stack.push(SvgTask::Draw {
                bx: inner,
                origin_x: origin_x.clone(),
                parent_baseline: baseline,
                fill,
            });
            Ok(())
        }
    }
}
