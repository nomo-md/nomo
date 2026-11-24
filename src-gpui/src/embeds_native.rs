//! 编辑器、导出和 Quick Look 共用的无视图 SVG 渲染；不保存文档或位图。
use std::sync::{Arc, OnceLock};
#[path = "math_fallback.rs"]
mod math_fallback;
#[path = "math_compat.rs"]
mod math_compat;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MermaidPalette { pub mode: u8, pub colors: [u32; 9] }

fn math_font() -> Result<&'static latex_rust::MathFont, String> {
    static FONT: OnceLock<Result<latex_rust::MathFont, String>> = OnceLock::new();
    FONT.get_or_init(|| latex_rust::MathFont::stix_two_math().map_err(|error| format!("{error:?}")))
        .as_ref()
        .map_err(Clone::clone)
}

/// 公式转 SVG，并返回基线以下部分占总高度的比例。
pub(crate) fn math_svg(source: &str, display: bool, color: [u8; 3]) -> Result<(String, f32), String> {
    math_svg_with_backend(source, display, color).map(|(svg, depth, _)| (svg, depth))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MathBackend { Native, Compatibility }

fn compatibility_math_svg(source: &str, display: bool, color: [u8; 3], native_error: String) -> Result<(String, f32, MathBackend), String> {
    let rendered = std::panic::catch_unwind(|| math_compat::math_svg(source, display, color))
        .unwrap_or_else(|_| Err("兼容引擎内部错误".into()));
    rendered
        .map(|(svg, depth)| (svg, depth, MathBackend::Compatibility))
        .map_err(|compat| format!("{native_error}；兼容引擎：{compat}"))
}

fn compatibility_layout_error(error: &latex_rust::Error) -> bool {
    matches!(error, latex_rust::Error::Unsupported { what }
        if !what.contains("nests deeper than") && !what.contains("预算"))
}

fn math_svg_with_backend(source: &str, display: bool, color: [u8; 3]) -> Result<(String, f32, MathBackend), String> {
    if source.len() > 64 * 1024 { return Err("公式超出 64 KiB 渲染预算；原文已保留".into()); }
    let ast = match latex_rust::parse(source) {
        Ok(ast) => ast,
        Err(error) => {
            let message = format!("公式解析失败：{error}");
            // Keep genuine dangling escapes and unmatched fences as errors.
            // Never use a second parser to bypass the native nesting budget.
            if matches!(error, latex_rust::ParseError::TrailingBackslash | latex_rust::ParseError::UnmatchedDelimiter)
                || matches!(&error, latex_rust::ParseError::Malformed(why) if why.contains("nests deeper than")) {
                return Err(message);
            }
            return compatibility_math_svg(source, display, color, message);
        }
    };
    let font = math_font()?;
    let style = if display { latex_rust::MathStyle::Display } else { latex_rust::MathStyle::Text };
    let em = latex_rust::Dim::from_i64(12);
    let fallback = math_fallback::MathFallback::default();
    let tree = match latex_rust::layout_with_glyph_fallback(&ast, font, style, &em, &|ch, style| fallback.glyph(ch, style)) {
        Ok(tree) => tree,
        Err(error) if compatibility_layout_error(&error) => return compatibility_math_svg(source, display, color, describe_math_error(error)),
        Err(error) => return Err(describe_math_error(error)),
    };
    let as_f32 = |dim: &latex_rust::Dim| dim.to_svg_string().parse::<f32>().unwrap_or(0.0);
    let mut options = latex_rust::SvgOptions::new();
    options.font_size_pt = em;
    options.display = display;
    options.color = latex_rust::Color::rgb(color[0], color[1], color[2]);
    let image = match latex_rust::render_svg_with_metrics(&tree, font, &options) {
        Ok(image) => image,
        Err(error) if compatibility_layout_error(&error) => return compatibility_math_svg(source, display, color, describe_math_error(error)),
        Err(error) => return Err(describe_math_error(error)),
    };
    let height = as_f32(&image.height_pt);
    let baseline = as_f32(&image.baseline_pt);
    let ratio = if height > 0.0 { ((height - baseline) / height).clamp(0.0, 1.0) } else { 0.0 };
    Ok((image.svg, ratio, MathBackend::Native))
}

fn describe_math_error(error: latex_rust::Error) -> String {
    let text = format!("{error:?}");
    if text.contains("MissingGlyph") {
        format!("公式缺少可用字形（已尝试系统字体）：{text}")
    } else {
        format!("公式渲染失败：{text}")
    }
}

#[cfg(test)]
mod math_tests {
    use super::*;

    fn assert_visible(source: &str, display: bool) -> MathBackend {
        let (svg, depth, actual) = math_svg_with_backend(source, display, [32, 48, 64])
            .unwrap_or_else(|error| panic!("{source:?}: {error}"));
        assert!(depth.is_finite() && (0.0..=1.0).contains(&depth));
        assert!(!svg.contains("<text"), "unresolved glyphs: {source}");
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
        let size = tree.size().to_int_size();
        let mut pixels = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
        resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixels.as_mut());
        assert!(pixels.pixels().iter().any(|pixel| pixel.alpha() != 0), "{source}");
        actual
    }

    fn assert_renders(source: &str, display: bool, backend: MathBackend) {
        assert_eq!(assert_visible(source, display), backend, "{source}");
    }

    #[test]
    fn primary_math_preserves_existing_grammar_and_geometry() {
        for formula in [
            r"\frac{1}{2}+\sqrt{x}",
            r"\left\{x\middle|\frac{a}{b}>0\right\}",
            r"a \pmod{30}", r"a \bmod n", r"\underline{\qquad\qquad}",
            r"\begin{gathered}x+y=1\\x-y=0\end{gathered}",
            r"\begin{bmatrix}1&2\\3&4\end{bmatrix}",
        ] {
            assert_renders(formula, true, MathBackend::Native);
        }
    }

    #[test]
    fn compatibility_renderer_accepts_command_families_and_environments() {
        // These are different grammar families, not substitutes for one input.
        for formula in [
            r"\argmax_{x\in A} f(x)",
            r"\text{\AA, \'{a}}",
            r"\begin{alignedat}{2}x&+y&=1\\x&-y&=0\end{alignedat}",
            r"\begin{matrix}a&b\\\hline c&d\end{matrix}",
            r"\mathchoice{D}{T}{S}{SS}",
            r"a\mod{n}", r"a\pod{n}", r"\bm{AB}",
            r"\def\sq#1{#1^2}\sq{x}+\sq{y}",
        ] {
            assert_renders(formula, true, MathBackend::Compatibility);
            assert_renders(formula, false, MathBackend::Compatibility);
        }
    }

    #[test]
    fn explicit_whitespace_has_one_semantics_in_math_and_text() {
        let expected = latex_rust::parse(r"x\ y").unwrap();
        let trailing = latex_rust::parse(r"x\ ").unwrap();
        let text = latex_rust::parse(r"\text{a\ b}").unwrap();
        for space in [" ", "\t", "\n", "\r\n", "\r", "\n  ", "\u{b}", "\u{c}"] {
            assert_eq!(latex_rust::parse(&format!("x\\{space}y")).unwrap(), expected);
            assert_eq!(latex_rust::parse(&format!("x\\{space}")).unwrap(), trailing);
            assert_eq!(latex_rust::parse(&format!("\\text{{a\\{space}b}}")).unwrap(), text);
            assert_renders(&format!("x\\{space}"), false, MathBackend::Native);
        }
        assert!(latex_rust::parse(r"\hat{}").is_err());
    }

    #[test]
    fn common_math_grammar_families_render_in_both_styles() {
        let mut examples = Vec::new();
        for name in ["frac", "dfrac", "tfrac", "binom", "dbinom", "tbinom"] {
            examples.push(format!(r"\{name}{{a^2+b}}{{c+d}}"));
        }
        for name in ["mathbf", "mathbb", "mathcal", "mathfrak", "mathsf", "mathtt", "mathit", "boldsymbol", "bm"] {
            examples.push(format!(r"\{name}{{AB}}+x"));
        }
        for name in ["hat", "widehat", "tilde", "widetilde", "overline", "underline", "overrightarrow", "vec", "cancel", "bcancel", "xcancel", "boxed"] {
            examples.push(format!(r"\{name}{{x+y}}"));
        }
        for name in ["sin", "cos", "ln", "log", "sinh", "lim", "argmax", "argmin", "max", "min", "det", "gcd"] {
            examples.push(format!(r"\{name}_{{i\in A}} f(i)"));
        }
        for name in ["thinspace", "medspace", "thickspace", "negthinspace", "enspace", "quad", "qquad"] {
            examples.push(format!(r"a\{name} b"));
        }
        for (open, close) in [("(", ")"), ("[", "]"), (r"\{", r"\}"), ("|", "|"), (r"\langle", r"\rangle"), (r"\lfloor", r"\rfloor"), (r"\lceil", r"\rceil"), (r"\lVert", r"\rVert")] {
            examples.push(format!(r"\left{open} \frac{{a}}{{b}} \middle| x>0 \right{close}"));
        }
        for formula in [
            r"\sqrt[3]{x+\frac{1}{2}}", r"\sum_{k=1}^n k^2", r"\int_0^1 x\,dx", r"\iint_D f(x,y)\,dx\,dy",
            r"\overset{!}{=}", r"\underset{n\to\infty}{\lim}", r"\underbrace{x+x}_{n}\overbrace{y+y}^{m}",
            r"a\pmod n", r"a\mod{n}", r"a\pod{n}", r"a\bmod n", r"\operatorname{rank}(A)",
            r"\xrightarrow{n\to\infty}", r"a\coloneqq b", r"\mathclap{x}+y", r"x+\hphantom{00}+y",
            r"\text{one }\textbf{two}\text{ \AA}", r"\def\sq#1{#1^2}\sq{x}+\sq{y}",
        ] { examples.push(formula.to_owned()); }
        for formula in &examples {
            assert_visible(formula, true);
            assert_visible(formula, false);
        }
        for name in ["matrix", "pmatrix", "bmatrix", "Bmatrix", "vmatrix", "Vmatrix", "smallmatrix"] {
            assert_visible(&format!(r"\begin{{{name}}}1&2\\3&4\end{{{name}}}"), true);
        }
        for formula in [r"\begin{cases}x&x>0\\-x&x<0\end{cases}", r"\begin{aligned}a&=b\\c&=d\end{aligned}",
            r"\begin{alignedat}{2}10&x+&3&y=2\\3&x+&13&y=4\end{alignedat}",
            r"\begin{gathered}x=1\\y=2\end{gathered}", r"\begin{array}{c|c}1&2\\\hline 3&4\end{array}"] {
            assert_visible(formula, true);
        }
    }

    #[test]
    fn malformed_or_over_budget_formulas_still_fail() {
        for formula in [r"\nomoUnknownCommand{x}", r"\left(x", "x\\", r"\frac{1}", r"\begin{matrix}x\end{cases}", r"\def\loopcmd{\loopcmd}\loopcmd"] {
            assert!(math_svg(formula, false, [0; 3]).is_err(), "{formula}");
        }
        let deep = "{".repeat(40) + "x" + &"}".repeat(40);
        assert!(math_svg(&deep, false, [0; 3]).is_err());
        let oversized = String::from(r"\argmax ") + &"x".repeat(16 * 1024);
        assert!(math_svg(&oversized, false, [0; 3]).unwrap_err().contains("预算"));
        assert!(math_svg(&"x".repeat(64 * 1024 + 1), true, [0; 3]).unwrap_err().contains("预算"));
    }
}

/// Mermaid 浅色 / 深色配置。浅色沿用 Mermaid 默认主题（与 Web 版一致），深色使用 Mermaid 的 dark 主题；
/// 背景透明，由编辑器底色衬托。
fn mermaid_engine(dark: bool, palette: Option<MermaidPalette>, font_family: &str) -> merman::Engine {
    let mode = palette.map_or(if dark { 1 } else { 0 }, |palette| palette.mode);
    let family = format!("{}, {}, sans-serif", serde_json::to_string(font_family).unwrap(), serde_json::to_string(MERMAID_FONT_FAMILY).unwrap());
    let mut value = serde_json::json!({
        "theme": match mode { 1 => "dark", 2 => "base", _ => "default" },
        "darkMode": dark,
        "fontFamily": family,
        "themeVariables": { "fontFamily": family },
    });
    if let Some(palette) = palette.filter(|palette| palette.mode == 2) {
        let colors = palette.colors.map(|color| format!("#{color:06x}"));
        let [document, surface, fg, border, muted, rail, chrome, accent, heading] = &colors;
        value["themeVariables"] = serde_json::json!({
            "fontFamily": family,
            "background": document, "primaryColor": surface, "primaryTextColor": fg, "primaryBorderColor": border,
            "lineColor": muted, "secondaryColor": rail, "tertiaryColor": chrome, "mainBkg": surface,
            "nodeBorder": accent, "clusterBkg": rail, "clusterBorder": border,
            "edgeLabelBackground": document, "textColor": fg, "titleColor": heading,
        });
    }
    merman::Engine::new().with_site_config(merman::MermaidConfig::from_value(value))
}

/// 正文字体没有所需字形时的 CJK 回退；独立 Quick Look 入口也沿用此默认字体。
#[cfg(target_os = "macos")]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Hiragino Sans GB";
#[cfg(target_os = "windows")]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Microsoft YaHei";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Noto Sans CJK SC";

/// 与正文选中的字体共用系统字体目录；测量与 SVG 使用相同的主字体和 CJK 回退。
/// 只缓存最近一种字体组合，切换时释放旧组合；在途渲染只持有自己的 Arc，结束即释放。
/// 字体文件最多 8 个／128 MiB（覆盖 macOS 的 PingFang 字体集），测量缓存沿用 4 MiB 上限。
pub(crate) fn mermaid_fonts(font_family: &str) -> Option<merman::svg::export::NativeFontContext> {
    use resvg::usvg::fontdb::{Family, Query, Source, Style, Weight};
    type CachedFonts = Option<(String, Option<merman::svg::export::NativeFontContext>)>;
    static CONTEXT: std::sync::Mutex<CachedFonts> = std::sync::Mutex::new(None);
    let mut cached = CONTEXT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, fonts)) = cached.as_ref().filter(|(family, _)| family == font_family) { return fonts.clone(); }
    *cached = None;
    let database = font_database();
    let mut paths = Vec::new();
    for family in [font_family, MERMAID_FONT_FAMILY] {
        for (weight, style) in [(Weight::NORMAL, Style::Normal), (Weight::BOLD, Style::Normal),
            (Weight::NORMAL, Style::Italic), (Weight::BOLD, Style::Italic)] {
            let Some(id) = database.query(&Query { families: &[Family::Name(family)], weight, style, ..Default::default() }) else { continue; };
            let path = match database.face_source(id) {
                Some((Source::File(path) | Source::SharedFile(path, _), _)) => path,
                _ => continue,
            };
            if !paths.contains(&path) { paths.push(path); }
        }
    }
    let mut options = merman::svg::export::NativeFontOptions::default();
    options.limits.max_files = 8;
    options.limits.max_bytes = 128 * 1024 * 1024;
    for path in paths { options = options.with_font_file(path); }
    let result = options.load(&merman::OperationControl::new());
    if let Err(error) = &result { eprintln!("Mermaid 字体加载失败，改用估算测量：{error}"); }
    let fonts = result.ok();
    *cached = Some((font_family.to_string(), fonts.clone()));
    fonts
}

#[cfg(test)]
pub(crate) fn mermaid_svg(source: &str, dark: bool) -> Result<String, String> {
    mermaid_svg_controlled(source, dark, merman::OperationControl::new())
}

/// 页首预览给图表单独设置时间预算；超时使用占位，不拖住后续文件。
pub(crate) fn thumbnail_mermaid(source: &str, dark: bool, font_family: &str) -> Result<String, String> {
    let _ = mermaid_fonts(font_family);
    mermaid_svg_with_palette(source, dark, None, font_family, merman::OperationControl::new().with_deadline(std::time::Duration::from_millis(150)))
}

/// 导出复用原生图表管线，单图限时且不进入编辑区位图缓存。
pub(crate) fn export_mermaid(source: &str, dark: bool, palette: MermaidPalette, font_family: &str) -> Result<String, String> {
    if source.len() > 64 * 1024 { return Err("图表超过 64 KiB 导出预算".into()); }
    let _ = mermaid_fonts(font_family);
    mermaid_svg_with_palette(source, dark, Some(palette), font_family, merman::OperationControl::new().with_deadline(std::time::Duration::from_secs(3)))
}

#[cfg(test)]
fn mermaid_svg_controlled(source: &str, dark: bool, control: merman::OperationControl) -> Result<String, String> {
    mermaid_svg_with_palette(source, dark, None, MERMAID_FONT_FAMILY, control)
}

pub(crate) fn mermaid_svg_with_palette(source: &str, dark: bool, palette: Option<MermaidPalette>, font_family: &str, control: merman::OperationControl) -> Result<String, String> {
    use merman::svg::SvgPipeline;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let request = merman::RenderRequest::svg(
        source,
        control,
        merman::SvgRequest {
            environment: match mermaid_fonts(font_family) {
                Some(fonts) => merman::SvgEnvironment::deterministic().with_native_font_context(fonts).map_err(|error| format!("字体加载失败：{error}"))?,
                None => merman::SvgEnvironment::deterministic(),
            },
            options: merman::svg::SvgRenderOptions { diagram_id: Some(format!("nomo-mermaid-{id}")), ..Default::default() },
            // resvg 不支持 `<foreignObject>` 中的 HTML 标签，改用转换为纯 SVG 文本的管线。
            pipeline: Some(SvgPipeline::resvg_safe()),
            ..Default::default()
        },
    );
    match merman::Renderer::new().with_engine(mermaid_engine(dark, palette, font_family)).render(request) {
        Ok(merman::RenderOutput::Svg(Some(svg))) => Ok(svg.svg().to_string()),
        Ok(_) => Err("未识别的 Mermaid 图表类型".into()),
        Err(error) => Err(format!("Mermaid 渲染失败：{error}")),
    }
}

/// resvg 的默认缺字回退按系统枚举顺序选字形，可能把粗体中文退回常规字重。
/// 图表优先沿用测量时的 CJK 回退及主字体字重，避免 Segoe UI 等西文字体造成测绘差异。
pub(crate) fn mermaid_font_resolver() -> resvg::usvg::FontResolver<'static> {
    use resvg::usvg::fontdb::{Family, Query};
    let fallback = resvg::usvg::FontResolver::default_fallback_selector();
    resvg::usvg::FontResolver {
        select_fallback: Box::new(move |ch, excluded, database| {
            if let Some(base) = excluded.first().and_then(|id| database.face(*id)) {
                let query = Query { families: &[Family::Name(MERMAID_FONT_FAMILY)], weight: base.weight,
                    style: base.style, stretch: base.stretch };
                if let Some(id) = database.query(&query).filter(|id| !excluded.contains(id)) {
                    let supports = database.with_face_data(id, |bytes, index|
                        ttf_parser::Face::parse(bytes, index).ok().is_some_and(|face| face.glyph_index(ch).is_some()));
                    if supports == Some(true) { return Some(id); }
                }
            }
            fallback(ch, excluded, database)
        }),
        ..Default::default()
    }
}

/// SVG 文本渲染所需字体库：系统字体（含冬青黑体），无衬线回退设为冬青黑体，与测量一致。
pub(crate) fn font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static DATABASE: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            database.load_system_fonts();
            database.set_sans_serif_family(MERMAID_FONT_FAMILY);
            Arc::new(database)
        })
        .clone()
}
