//! Self-contained SVG renderer. Glyphs are `<path>` elements from STIX Two Math.

pub(crate) mod outline;
mod render;

pub use render::{latex_to_svg, render_svg, SvgOptions};
