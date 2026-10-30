//! LaTeX-Rust: a pure Rust LaTeX **math** renderer.
//!
//! Parse a math string to a typed AST, lay it out with a TeX-faithful box
//! model whose dimensions are exact rational [`Dim`] values (no hardware `f32` /
//! `f64` in layout arithmetic), and render to SVG, PNG (`features = ["png"]`),
//! or egui shapes (`features = ["egui"]`).
//!
//! Crate layout: [`parser`],
//! [`mod@layout`], [`font`], [`render`] (`svg` / `png` / `egui`).
//!
//! Unsupported constructs return [`Error`] — never a fake render.
//!
//! # Examples
//!
//! ```
//! use latex_rust::{parse, layout, tokenize, lookup, MathFont, MathStyle, Dim, MathBox};
//!
//! let ast = parse(r"\frac{1}{2}").expect("parse");
//! assert_eq!(ast.gold(), r#"(frac (atom Ord "1") (atom Ord "2"))"#);
//!
//! let tokens = tokenize(r"\frac{1}{2}").expect("tokens");
//! assert!(!tokens.is_empty());
//! assert_eq!(lookup(r"\alpha").unwrap().glyph, "α");
//!
//! let font = MathFont::stix_two_math().expect("STIX Two Math");
//! assert_eq!(font.units_per_em(), 1000);
//!
//! let boxed = layout(&ast, &font, MathStyle::Text).expect("layout");
//! assert!(!boxed.width.is_zero());
//!
//! let svg = latex_rust::latex_to_svg(r"x", &font, &latex_rust::SvgOptions::new()).expect("svg");
//! assert!(svg.contains("<path"));
//!
//! let packed = MathBox::hpack(vec![
//!     MathBox::rule(Dim::one(), Dim::zero(), Dim::zero()),
//!     MathBox::rule(Dim::ratio(1, 2), Dim::zero(), Dim::zero()),
//! ]);
//! assert_eq!(packed.width, Dim::ratio(3, 2));
//! ```

#![deny(missing_docs)]
#![deny(clippy::float_arithmetic)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod atoms;
mod color;
mod dim;
mod error;
mod hash;
mod style_map;
mod symbols;

/// OpenType MATH metrics and the embedded STIX Two Math face.
pub mod font;
/// AST → TeX-faithful [`MathBox`](layout::MathBox).
pub mod layout;
/// LaTeX math → [`MathNode`](parser::MathNode) AST.
pub mod parser;
/// Box model → SVG, PNG, or egui primitives.
pub mod render;

pub use atoms::symbol_atom_kind;
pub use color::{named_color, parse_color_spec, Color, ColorTable};
pub use dim::{Dim, DIM_PREC};
pub use error::{Error, FontError, ParseError};
pub use font::{
    GlyphMetrics, MathFont, STIX_TWO_MATH_NAME, STIX_TWO_MATH_OTF, STIX_TWO_MATH_SHA256,
};
pub use layout::{
    layout, layout_with_em_size_pt, layout_with_glyph_fallback, layout_with_max_depth, layout_with_numbering,
    layout_with_numbering_and_em_size_pt, BoxContent, MathBox, MathParams, MathStyle, NumberFormat,
    NumberStyle, NumberingConfig, NumberingState, FallbackGlyph, GlyphFallback,
};
pub use parser::{
    format_tokens, parse, parse_with_colors, parse_with_options, preprocess, tokenize, AccentKind,
    AtomKind, ColSpec, DelimSize, Delimiter, EnvRow, EqNumber, FracStyle, IntegralKind, MathNode,
    MatrixStyle, ParseOptions, PhantomKind, SpaceKind, TextItem, TextRun, TextStyle, Token,
    DEFAULT_MAX_NESTING_DEPTH,
};
#[cfg(feature = "egui")]
pub use render::egui::{latex_to_shapes, paint_egui, shapes};
pub use render::egui::{render_egui, EguiOptions};
pub use render::png::{latex_to_png, render_png, PngBackground, PngOptions};
pub use render::svg::{latex_to_svg, render_svg, SvgOptions};
pub use style_map::styled_char;
pub use symbols::{category_count, glyph_char, lookup, symbols, SymbolEntry, SymbolKind};
