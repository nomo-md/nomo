//! Render backends: SVG, PNG (`tiny-skia`, feature `png`), and egui
//! (`features = ["egui"]`).
//!
//! Pipeline: [`crate::layout::MathBox`] → this module. SVG is the v1 output.
//! PNG and egui are optional. Without their features those entry points return
//! [`crate::Error::Unsupported`].

pub mod egui;
pub mod png;
pub mod svg;

#[cfg(feature = "egui")]
pub use egui::{latex_to_shapes, paint_egui, shapes};
pub use egui::{render_egui, EguiOptions};
pub use png::{latex_to_png, render_png, PngBackground, PngOptions};
pub use svg::{latex_to_svg, render_svg, SvgOptions};
