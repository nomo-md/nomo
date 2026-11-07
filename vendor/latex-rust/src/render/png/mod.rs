//! PNG raster backend. Milestone 9; optional feature `png` pulls in `tiny-skia`.
//!
//! Without the feature every entry point is [`Error::Unsupported`].
//! Color channels use [`Color::to_rgba8`](crate::Color::to_rgba8).
//!
//! `tiny-skia` picks a SIMD path at runtime. No user configuration and no extra
//! compile flags are required; output is correct on every supported architecture:
//!
//! - On x86_64 with AVX2 — tiny-skia runs at peak performance automatically
//! - On Apple Silicon — tiny-skia uses the ARM NEON path, also fast

#[cfg(feature = "png")]
mod raster;

use crate::color::Color;
use crate::dim::Dim;
use crate::error::Error;
use crate::font::MathFont;
use crate::layout::MathBox;
use crate::parser::parse;

/// Canvas behind the expression.
///
/// # Examples
///
/// ```
/// use latex_rust::{Color, PngBackground};
///
/// assert_eq!(PngBackground::Transparent, PngBackground::Transparent);
/// let _ = PngBackground::White;
/// let _ = PngBackground::Color(Color::rgb(255, 255, 255));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngBackground {
    /// Fully transparent (default).
    Transparent,
    /// Opaque white.
    White,
    /// Opaque custom fill.
    Color(Color),
}

/// Options for [`render_png`].
///
/// Default: 12 pt, 144 dpi, black fill, transparent background, text style.
///
/// # Examples
///
/// ```
/// use latex_rust::{Dim, PngOptions};
///
/// let mut opt = PngOptions::new();
/// opt.dpi = Dim::from_i64(144);
/// assert!(!opt.display);
/// ```
#[derive(Clone, Debug)]
pub struct PngOptions {
    /// Em size in points (same meaning as [`crate::SvgOptions::font_size_pt`]).
    pub font_size_pt: Dim,
    /// Raster resolution in dots per inch. Must be in `(0, 2400]`.
    pub dpi: Dim,
    /// Default glyph fill.
    pub color: Color,
    /// Canvas behind the glyphs.
    pub background: PngBackground,
    /// When using [`latex_to_png`], pick display vs text style.
    pub display: bool,
}

impl Default for PngOptions {
    fn default() -> Self {
        Self {
            font_size_pt: Dim::from_i64(12),
            dpi: Dim::from_i64(144),
            color: Color::rgb(0, 0, 0),
            background: PngBackground::Transparent,
            display: false,
        }
    }
}

impl PngOptions {
    /// 12 pt, 144 dpi, black fill, transparent background, text style.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse, lay out, and rasterize `latex` to PNG bytes.
///
/// Without `features = ["png"]` this is [`Error::Unsupported`]. DPI must be in
/// `(0, 2400]`.
///
/// # Arguments
///
/// * `latex` — math source (see [`crate::parse()`]).
/// * `font` — face used for layout and outlines.
/// * `options` — em size, DPI, fill, background, and display vs text.
///
/// # Returns
///
/// PNG file bytes (`0x89 PNG` header when the feature is on).
///
/// # Errors
///
/// * [`Error::Unsupported`] — `png` feature off, or unsupported construct.
/// * [`Error::InvalidOption`] — DPI outside `(0, 2400]`.
/// * [`Error::Parse`] / [`Error::Font`] — parse or missing glyph.
///
/// # Examples
///
/// ```
/// use latex_rust::{latex_to_png, MathFont, PngOptions};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let r = latex_to_png(r"x", &font, &PngOptions::new());
/// #[cfg(not(feature = "png"))]
/// assert!(r.is_err());
/// #[cfg(feature = "png")]
/// assert!(r.unwrap().starts_with(b"\x89PNG"));
/// ```
pub fn latex_to_png(latex: &str, font: &MathFont, options: &PngOptions) -> Result<Vec<u8>, Error> {
    let ast = parse(latex)?;
    #[cfg(not(feature = "png"))]
    {
        let _ = (font, options, ast);
        Err(Error::Unsupported {
            what: "png renderer".into(),
        })
    }
    #[cfg(feature = "png")]
    {
        use crate::layout::{layout_with_em_size_pt, MathStyle};
        let style = if options.display {
            MathStyle::Display
        } else {
            MathStyle::Text
        };
        let tree = layout_with_em_size_pt(&ast, font, style, &options.font_size_pt)?;
        render_png(&tree, font, options)
    }
}

/// Rasterize a laid-out [`MathBox`] to PNG bytes.
///
/// # Arguments
///
/// * `tree` — box model from [`crate::layout()`].
/// * `font` — face that supplied the glyph ids on `tree`.
/// * `options` — em size, DPI, fill, and background (`display` is ignored).
///
/// # Returns
///
/// PNG file bytes when `features = ["png"]` is enabled.
///
/// # Errors
///
/// Same as [`latex_to_png`], except parse errors cannot occur.
///
/// # Examples
///
/// ```
/// use latex_rust::{render_png, MathBox, MathFont, PngOptions};
///
/// let font = MathFont::stix_two_math().unwrap();
/// let r = render_png(&MathBox::empty(), &font, &PngOptions::new());
/// #[cfg(not(feature = "png"))]
/// assert!(r.is_err());
/// #[cfg(feature = "png")]
/// assert!(r.is_ok());
/// ```
pub fn render_png(tree: &MathBox, font: &MathFont, options: &PngOptions) -> Result<Vec<u8>, Error> {
    #[cfg(not(feature = "png"))]
    {
        let _ = (tree, font, options);
        Err(Error::Unsupported {
            what: "png renderer".into(),
        })
    }
    #[cfg(feature = "png")]
    {
        raster::render(tree, font, options)
    }
}

#[cfg(all(test, not(feature = "png")))]
mod tests {
    use super::*;
    use crate::font::MathFont;
    use crate::layout::MathBox;

    #[test]
    fn png_is_unsupported() {
        let font = MathFont::stix_two_math().expect("STIX");
        let err = render_png(&MathBox::empty(), &font, &PngOptions::new()).expect_err("png");
        assert!(err.to_string().contains("png"), "{err}");
        let err = latex_to_png("x", &font, &PngOptions::new()).expect_err("png latex");
        assert!(err.to_string().contains("png"), "{err}");
    }
}
