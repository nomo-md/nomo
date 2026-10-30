//! egui primitive backend. Milestone 10; optional feature `egui`.
//!
//! Without the feature every entry point is [`Error::Unsupported`].
//! With `features = ["egui"]` a [`MathBox`] becomes `egui::Shape`
//! meshes and rects — no SVG intermediate.

#[cfg(feature = "egui")]
mod emit;
#[cfg(feature = "egui")]
mod tessellate;

use crate::color::Color;
use crate::dim::Dim;
use crate::error::Error;
use crate::font::MathFont;
use crate::layout::MathBox;

/// Options for [`shapes`] / [`paint_egui`].
///
/// Default: 14 pt, black fill, text style.
///
/// # Examples
///
/// ```
/// use latex_rust::{Color, Dim, EguiOptions};
///
/// let mut opt = EguiOptions::new();
/// opt.font_size_pt = Dim::from_i64(14);
/// opt.color = Color::rgb(0, 0, 0);
/// opt.display = false;
/// ```
#[derive(Clone, Debug)]
pub struct EguiOptions {
    /// Em size in points (same meaning as [`crate::SvgOptions::font_size_pt`]).
    pub font_size_pt: Dim,
    /// Default glyph fill.
    pub color: Color,
    /// When using [`latex_to_shapes`], pick display vs text style.
    pub display: bool,
}

impl Default for EguiOptions {
    fn default() -> Self {
        Self {
            font_size_pt: Dim::from_i64(14),
            color: Color::rgb(0, 0, 0),
            display: false,
        }
    }
}

impl EguiOptions {
    /// 14 pt, black fill, text style.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Probe the egui backend: tessellate `tree` or return [`Error::Unsupported`].
///
/// Without `features = ["egui"]` this is [`Error::Unsupported`]. With the
/// feature, use [`shapes`] to keep the emitted primitives.
///
/// # Arguments
///
/// * `tree` — box model from [`crate::layout()`].
/// * `font` — face that supplied the glyph ids on `tree`.
///
/// # Returns
///
/// `Ok(())` when the feature is on and tessellation succeeds.
///
/// # Errors
///
/// * [`Error::Unsupported`] — `egui` feature off, or a box that cannot tessellate.
/// * [`Error::Font`] — missing glyph outline.
/// * [`Error::InvalidOption`] — non-positive font size (feature on).
///
/// # Examples
///
/// ```
/// use latex_rust::{render_egui, MathBox, MathFont};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let r = render_egui(&MathBox::empty(), &font);
/// #[cfg(not(feature = "egui"))]
/// assert!(r.is_err());
/// #[cfg(feature = "egui")]
/// assert!(r.is_ok());
/// ```
pub fn render_egui(tree: &MathBox, font: &MathFont) -> Result<(), Error> {
    #[cfg(not(feature = "egui"))]
    {
        let _ = (tree, font);
        Err(Error::Unsupported {
            what: "egui renderer".into(),
        })
    }
    #[cfg(feature = "egui")]
    {
        let _ = shapes(tree, font, &EguiOptions::new(), egui::Pos2::ZERO, 1.0)?;
        Ok(())
    }
}

/// `MathBox` → egui shapes and the layout bounding rect.
///
/// `pixels_per_point` is egui's device pixel ratio. Zero or negative is
/// [`Error::InvalidOption`]. Glyph tessellation is cached process-wide so a
/// later render of the same glyphs is a cache hit.
///
/// # Arguments
///
/// * `tree` — box model from [`crate::layout()`].
/// * `font` — face that supplied the glyph ids on `tree`.
/// * `options` — em size and default fill.
/// * `origin` — top-left of the layout rect, in egui points.
/// * `pixels_per_point` — device pixel ratio (must be positive).
///
/// # Returns
///
/// Shape list and the bounding `Rect` of the expression.
///
/// # Errors
///
/// * [`Error::InvalidOption`] — non-positive `pixels_per_point` or font size.
/// * [`Error::Font`] — missing glyph outline.
/// * [`Error::Unsupported`] — tessellation produced no triangles.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "egui")]
/// # {
/// use latex_rust::{layout, parse, shapes, EguiOptions, MathFont, MathStyle};
///
/// let ast = parse(r"x").unwrap();
/// let font = MathFont::stix_two_math().unwrap();
/// let tree = layout(&ast, &font, MathStyle::Text).unwrap();
/// let (shapes, rect) = shapes(&tree, &font, &EguiOptions::new(), egui::Pos2::ZERO, 1.0).unwrap();
/// assert!(!shapes.is_empty());
/// assert!(rect.width() > 0.0);
/// # }
/// ```
#[cfg(feature = "egui")]
pub fn shapes(
    tree: &MathBox,
    font: &MathFont,
    options: &EguiOptions,
    origin: egui::Pos2,
    pixels_per_point: f32,
) -> Result<(Vec<egui::Shape>, egui::Rect), Error> {
    emit::shapes(tree, font, options, origin, pixels_per_point)
}

/// Parse, lay out, and emit egui shapes.
///
/// # Arguments
///
/// * `latex` — math source (see [`crate::parse()`]).
/// * `font` — face used for layout and outlines.
/// * `options` — em size, fill, and display vs text style.
/// * `origin` — top-left of the layout rect, in egui points.
/// * `pixels_per_point` — device pixel ratio (must be positive).
///
/// # Returns
///
/// Shape list and the bounding `Rect` of the expression.
///
/// # Errors
///
/// Same as [`crate::parse()`] plus [`shapes`].
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "egui")]
/// # {
/// use latex_rust::{latex_to_shapes, EguiOptions, MathFont};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let (shapes, _) = latex_to_shapes(r"x", &font, &EguiOptions::new(), egui::Pos2::ZERO, 1.0).unwrap();
/// assert!(!shapes.is_empty());
/// # }
/// ```
#[cfg(feature = "egui")]
pub fn latex_to_shapes(
    latex: &str,
    font: &MathFont,
    options: &EguiOptions,
    origin: egui::Pos2,
    pixels_per_point: f32,
) -> Result<(Vec<egui::Shape>, egui::Rect), Error> {
    use crate::layout::{layout_with_em_size_pt, MathStyle};
    use crate::parser::parse;
    let ast = parse(latex)?;
    let style = if options.display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let tree = layout_with_em_size_pt(&ast, font, style, &options.font_size_pt)?;
    shapes(&tree, font, options, origin, pixels_per_point)
}

/// Paint shapes through an egui [`egui::Painter`].
///
/// Uses `painter.ctx().pixels_per_point()` as the device pixel ratio.
///
/// # Arguments
///
/// * `tree` — box model from [`crate::layout()`].
/// * `font` — face that supplied the glyph ids on `tree`.
/// * `options` — em size and default fill.
/// * `painter` — destination painter.
/// * `origin` — top-left of the layout rect, in egui points.
///
/// # Returns
///
/// The bounding `Rect` of the painted expression.
///
/// # Errors
///
/// Same as [`shapes`].
///
/// # Examples
///
/// This entry point needs a live `egui::Painter` from an egui app. See [`shapes`]
/// for a harness-free equivalent.
#[cfg(feature = "egui")]
pub fn paint_egui(
    tree: &MathBox,
    font: &MathFont,
    options: &EguiOptions,
    painter: &egui::Painter,
    origin: egui::Pos2,
) -> Result<egui::Rect, Error> {
    let ppp = painter.ctx().pixels_per_point();
    let (shapes, rect) = shapes(tree, font, options, origin, ppp)?;
    painter.extend(shapes);
    Ok(rect)
}

#[cfg(all(test, not(feature = "egui")))]
mod tests {
    use super::*;
    use crate::font::MathFont;
    use crate::layout::MathBox;

    #[test]
    fn egui_is_unsupported() {
        let font = MathFont::stix_two_math().expect("STIX");
        let err = render_egui(&MathBox::empty(), &font).expect_err("egui");
        assert!(err.to_string().contains("egui"), "{err}");
    }
}
