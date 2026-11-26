//! Pure-Rust KaTeX-compatible fallback for formulas outside the primary grammar.
//! Both engines use the same source and SVG pipeline; no document rewriting.

use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_svg::{SvgColorSyntax, SvgOptions, render_to_svg_with_color_syntax};
use ratex_types::{color::Color, math_style::MathStyle};

const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_DISPLAY_ITEMS: usize = 4096;
const MAX_SVG_BYTES: usize = 8 * 1024 * 1024;
const MAX_VIEWPORT_EM: f64 = 4096.0;

pub(super) fn math_svg(source: &str, display: bool, color: [u8; 3]) -> Result<(String, f32), String> {
    if source.len() > MAX_INPUT_BYTES { return Err("兼容公式超过 16 KiB 预算".into()); }
    // The parser bounds macro expansion and nesting; the layout validator
    // bounds recursive output. Limit our own display list and SVG as well.
    let ast = ratex_parser::parse(source).map_err(|error| error.to_string())?;
    let style = if display { MathStyle::Display } else { MathStyle::Text };
    let color = Color::rgb(color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0);
    let tree = layout(&ast, &LayoutOptions::default().with_style(style).with_color(color));
    validate_layout(&tree).map_err(|error| error.to_string())?;
    let list = to_display_list(&tree);
    let height = list.height + list.depth;
    if list.items.len() > MAX_DISPLAY_ITEMS { return Err("兼容公式超过排版预算".into()); }
    if list.items.is_empty() || !list.width.is_finite() || !height.is_finite() || list.width <= 0.0 || height <= 0.0 {
        return Err("兼容公式没有可绘制的内容".into());
    }
    if list.width.max(height) > MAX_VIEWPORT_EM { return Err("兼容公式尺寸超过预算".into()); }
    // Match the primary engine's 12pt = 16px em. No padding so the layout
    // baseline maps directly to the native inline object baseline.
    let options = SvgOptions {
        font_size: 16.0, padding: 0.0, stroke_width: 1.0,
        embed_glyphs: true, font_dir: String::new(),
    };
    let svg = render_to_svg_with_color_syntax(&list, &options, SvgColorSyntax::Rgb);
    if svg.len() > MAX_SVG_BYTES { return Err("兼容公式 SVG 超过预算".into()); }
    // Unresolved glyphs must not silently disappear when resvg has no KaTeX font.
    if svg.contains("<text") { return Err("兼容公式缺少可用字形轮廓".into()); }
    Ok((svg, (list.depth / height).clamp(0.0, 1.0) as f32))
}
