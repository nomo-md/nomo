//! 编辑器、导出和 Quick Look 共用的无视图 SVG 渲染；不保存文档或位图。
use std::sync::{Arc, OnceLock};
#[path = "math_fallback.rs"]
mod math_fallback;

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
    if source.len() > 64 * 1024 { return Err("公式超出 64 KiB 渲染预算；原文已保留".into()); }
    let font = math_font()?;
    let ast = latex_rust::parse(source.trim()).map_err(|error| format!("公式解析失败：{error}"))?;
    let style = if display { latex_rust::MathStyle::Display } else { latex_rust::MathStyle::Text };
    let em = latex_rust::Dim::from_i64(12);
    let fallback = math_fallback::MathFallback::default();
    let tree = latex_rust::layout_with_glyph_fallback(&ast, font, style, &em, &|ch, style| fallback.glyph(ch, style)).map_err(describe_math_error)?;
    let as_f32 = |dim: &latex_rust::Dim| dim.to_svg_string().parse::<f32>().unwrap_or(0.0);
    let (height, depth) = (as_f32(&tree.height), as_f32(&tree.depth));
    let ratio = if height + depth > 0.0 { depth / (height + depth) } else { 0.0 };
    let mut options = latex_rust::SvgOptions::new();
    options.font_size_pt = em;
    options.display = display;
    options.color = latex_rust::Color::rgb(color[0], color[1], color[2]);
    let svg = latex_rust::render_svg(&tree, font, &options).map_err(describe_math_error)?;
    Ok((svg, ratio))
}

fn describe_math_error(error: latex_rust::Error) -> String {
    let text = format!("{error:?}");
    if text.contains("MissingGlyph") {
        format!("公式缺少可用字形（已尝试系统字体）：{text}")
    } else {
        format!("公式渲染失败：{text}")
    }
}

/// Mermaid 浅色 / 深色配置。浅色沿用 Mermaid 默认主题（与 Web 版一致），深色使用 Mermaid 的 dark 主题；
/// 背景透明，由编辑器底色衬托。
fn mermaid_engine(dark: bool, palette: Option<MermaidPalette>) -> merman::Engine {
    let mode = palette.map_or(if dark { 1 } else { 0 }, |palette| palette.mode);
    let mut value = serde_json::json!({
        "theme": match mode { 1 => "dark", 2 => "base", _ => "default" },
        "darkMode": dark,
        "fontFamily": MERMAID_FONT_FAMILY,
    });
    if let Some(palette) = palette.filter(|palette| palette.mode == 2) {
        let colors = palette.colors.map(|color| format!("#{color:06x}"));
        let [document, surface, fg, border, muted, rail, chrome, accent, heading] = &colors;
        value["themeVariables"] = serde_json::json!({
            "background": document, "primaryColor": surface, "primaryTextColor": fg, "primaryBorderColor": border,
            "lineColor": muted, "secondaryColor": rail, "tertiaryColor": chrome, "mainBkg": surface,
            "nodeBorder": accent, "clusterBkg": rail, "clusterBorder": border,
            "edgeLabelBackground": document, "textColor": fg, "titleColor": heading,
        });
    }
    merman::Engine::new().with_site_config(merman::MermaidConfig::from_value(value))
}

/// Mermaid 文字使用的字体：冬青黑体（含 W3/W6 字重，23 MB，位于常规系统字体目录）。
/// 苹方 PingFang.ttc 有 78 MB，超出 merman 默认的 64 MB 常驻上限，且常驻代价过高。
#[cfg(target_os = "macos")]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Hiragino Sans GB";
#[cfg(target_os = "windows")]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Microsoft YaHei";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MERMAID_FONT_FAMILY: &str = "Noto Sans CJK SC";

fn mermaid_font_file() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    { "/System/Library/Fonts/Hiragino Sans GB.ttc".into() }
    #[cfg(target_os = "windows")]
    { std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:/Windows".into())).join("Fonts/msyh.ttc") }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into() }
}

/// Mermaid 文本测量用的真实字体。默认的确定性测量与实际字形宽度不一致，类图等会截断文字；
/// 改用真实字体后布局与绘制使用同一份度量。首次渲染图表时加载并常驻；字体缺失时返回 None，
/// 退回确定性测量。
fn mermaid_fonts() -> Option<merman::svg::export::NativeFontContext> {
    static CONTEXT: OnceLock<Option<merman::svg::export::NativeFontContext>> = OnceLock::new();
    CONTEXT
        .get_or_init(|| {
            let result = merman::svg::export::NativeFontOptions::default().with_font_file(mermaid_font_file()).load(&merman::OperationControl::new());
            if let Err(error) = &result {
                eprintln!("Mermaid 字体加载失败，改用估算测量：{error}");
            }
            result.ok()
        })
        .clone()
}

#[cfg(test)]
pub(crate) fn mermaid_svg(source: &str, dark: bool) -> Result<String, String> {
    mermaid_svg_controlled(source, dark, merman::OperationControl::new())
}

/// 页首预览给图表单独设置时间预算；超时使用占位，不拖住后续文件。
pub(crate) fn thumbnail_mermaid(source: &str, dark: bool) -> Result<String, String> {
    mermaid_svg_controlled(source, dark, merman::OperationControl::new().with_deadline(std::time::Duration::from_millis(150)))
}

/// 导出复用原生图表管线，单图限时且不进入编辑区位图缓存。
pub(crate) fn export_mermaid(source: &str, dark: bool, palette: MermaidPalette) -> Result<String, String> {
    if source.len() > 64 * 1024 { return Err("图表超过 64 KiB 导出预算".into()); }
    mermaid_svg_with_palette(source, dark, Some(palette), merman::OperationControl::new().with_deadline(std::time::Duration::from_secs(3)))
}

fn mermaid_svg_controlled(source: &str, dark: bool, control: merman::OperationControl) -> Result<String, String> {
    mermaid_svg_with_palette(source, dark, None, control)
}

pub(crate) fn mermaid_svg_with_palette(source: &str, dark: bool, palette: Option<MermaidPalette>, control: merman::OperationControl) -> Result<String, String> {
    use merman::svg::SvgPipeline;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let request = merman::RenderRequest::svg(
        source,
        control,
        merman::SvgRequest {
            environment: match mermaid_fonts() {
                Some(fonts) => merman::SvgEnvironment::deterministic().with_native_font_context(fonts).map_err(|error| format!("字体加载失败：{error}"))?,
                None => merman::SvgEnvironment::deterministic(),
            },
            options: merman::svg::SvgRenderOptions { diagram_id: Some(format!("nomo-mermaid-{id}")), ..Default::default() },
            // resvg 不支持 `<foreignObject>` 中的 HTML 标签，改用转换为纯 SVG 文本的管线。
            pipeline: Some(SvgPipeline::resvg_safe()),
            ..Default::default()
        },
    );
    match merman::Renderer::new().with_engine(mermaid_engine(dark, palette)).render(request) {
        Ok(merman::RenderOutput::Svg(Some(svg))) => Ok(svg.svg().to_string()),
        Ok(_) => Err("未识别的 Mermaid 图表类型".into()),
        Err(error) => Err(format!("Mermaid 渲染失败：{error}")),
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
