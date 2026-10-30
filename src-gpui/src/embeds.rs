//! 公式与 Mermaid 图表的后台渲染和有界缓存（GPUI 全局状态）。
//!
//! 渲染链路全部为 Rust：公式由 latex-rust 按 TeX 规则排版成 SVG（字形已转为路径），
//! Mermaid 由 merman 解析、布局并输出 resvg 兼容的 SVG；两者再用 resvg 按 2 倍分辨率栅格化，
//! 交给 GPUI 作为位图绘制。
//!
//! 生命周期：布局阶段查询状态，缺失的条目登记后由 `request` 统一在后台线程渲染，完成后递增
//! `generation` 并刷新窗口；含公式或图表的块把 generation 纳入布局缓存键，因而在结果就绪后重排。
//! 位图缓存按估算字节做 LRU 淘汰；失败结果也会缓存，避免同一错误源反复渲染。

use gpui::{App, Global, RenderImage};
use image::{Frame, RgbaImage};
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

#[path = "math_fallback.rs"]
mod math_fallback;

/// 位图缓存上限（BGRA 字节）。800 个块级公式在 2 倍分辨率下约占 60–90 MiB，留出余量。
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
/// 单张位图的边长上限（设备像素），防止超大图表占用过多内存；超出时降低栅格倍率。
const MAX_RASTER_SIDE: f32 = 4096.0;
/// 栅格倍率：按 2 倍（视网膜屏）渲染，逻辑尺寸不变。
const RASTER_SCALE: f32 = 2.0;
/// latex-rust 以 12pt 作为 em 渲染，对应 16 个逻辑像素。
const BASE_EM_PX: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EmbedKind {
    /// `display` 为真时使用显示样式（块级公式），否则为行内样式。
    Math { display: bool },
    Mermaid,
    /// 只有少量颜色值参与键，继续使用同一份有界位图缓存。
    MermaidThemed(MermaidPalette),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MermaidPalette { pub mode: u8, pub colors: [u32; 9] }

impl EmbedKind {
    pub fn with_palette(self, palette: MermaidPalette) -> Self {
        if matches!(self, Self::Mermaid) { Self::MermaidThemed(palette) } else { self }
    }
}

/// 一次渲染请求。`em_px` 为正文字号（逻辑像素），`color` 为公式前景色，`dark` 决定图表配色。
#[derive(Clone, Debug)]
pub struct EmbedSpec {
    pub kind: EmbedKind,
    pub source: String,
    pub em_px: f32,
    pub color: [u8; 3],
    pub dark: bool,
}

impl EmbedSpec {
    pub fn key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.kind.hash(&mut hasher);
        // 围栏解析和局部草稿保留的首尾换行不同，但它们使用同一份渲染结果。
        self.source.trim().hash(&mut hasher);
        self.em_px.to_bits().hash(&mut hasher);
        self.color.hash(&mut hasher);
        self.dark.hash(&mut hasher);
        hasher.finish()
    }
}

/// 渲染结果的逻辑尺寸（像素）；`depth` 为基线以下的部分，用于行内公式对齐基线。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmbedInfo {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

#[derive(Clone, Debug)]
pub enum EmbedState {
    Pending,
    Ready(EmbedInfo),
    Failed(Arc<str>),
}

enum Entry {
    Loading,
    Ready { info: EmbedInfo, image: Arc<RenderImage>, bytes: usize, used: Cell<u64> },
    Failed(Arc<str>),
}

#[derive(Default)]
pub struct EmbedStore {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: Cell<u64>,
    pub generation: u64,
}

impl Global for EmbedStore {}

impl EmbedStore {
    /// 查询状态；返回 None 表示尚未请求，调用方应登记到 `request`。
    pub fn state(&self, key: u64) -> Option<EmbedState> {
        Some(match self.entries.get(&key)? {
            Entry::Loading => EmbedState::Pending,
            Entry::Ready { info, .. } => EmbedState::Ready(*info),
            Entry::Failed(message) => EmbedState::Failed(message.clone()),
        })
    }

    pub fn image(&self, key: u64) -> Option<Arc<RenderImage>> {
        self.clock.set(self.clock.get() + 1);
        match self.entries.get(&key) {
            Some(Entry::Ready { image, used, .. }) => {
                used.set(self.clock.get());
                Some(image.clone())
            }
            _ => None,
        }
    }

    /// CPU 缓存和窗口 atlas 必须一起淘汰，避免公式／图表纹理持续累积。
    fn insert(&mut self, key: u64, result: Result<(EmbedInfo, RenderImage, usize), String>) -> Vec<Arc<RenderImage>> {
        let mut retired = Vec::new();
        if let Some(Entry::Ready { image, bytes, .. }) = self.entries.remove(&key) {
            self.bytes -= bytes;
            retired.push(image);
        }
        self.clock.set(self.clock.get() + 1);
        match result {
            Ok((info, image, bytes)) => {
                self.bytes += bytes;
                self.entries.insert(key, Entry::Ready { info, image: Arc::new(image), bytes, used: Cell::new(self.clock.get()) });
                while self.bytes > MAX_CACHE_BYTES {
                    let oldest = self
                        .entries
                        .iter()
                        .filter_map(|(key, entry)| match entry {
                            Entry::Ready { used, .. } => Some((*key, used.get())),
                            _ => None,
                        })
                        .min_by_key(|(_, used)| *used);
                    let Some((oldest, _)) = oldest else { break };
                    // 淘汰后删除条目本身：下次可见时重新渲染，而不是停留在“渲染中”。
                    if let Some(Entry::Ready { image, bytes, .. }) = self.entries.remove(&oldest) {
                        self.bytes -= bytes;
                        retired.push(image);
                    }
                }
            }
            Err(message) => {
                self.entries.insert(key, Entry::Failed(message.into()));
            }
        }
        self.generation += 1;
        retired
    }
}

/// 在后台渲染一批请求；已缓存或正在渲染的条目会被跳过。
pub fn request(specs: Vec<EmbedSpec>, cx: &mut App) {
    for spec in specs {
        let key = spec.key();
        let store = cx.global_mut::<EmbedStore>();
        if store.entries.contains_key(&key) {
            continue;
        }
        store.entries.insert(key, Entry::Loading);
        let task = cx.background_executor().spawn(async move { render(&spec) });
        cx.spawn(async move |cx| {
            let result = task.await;
            cx.update(|cx| {
                let retired = cx.global_mut::<EmbedStore>().insert(key, result);
                for image in retired { cx.drop_image(image, None); }
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

fn render(spec: &EmbedSpec) -> Result<(EmbedInfo, RenderImage, usize), String> {
    let scale_to_em = spec.em_px / BASE_EM_PX;
    let source = spec.source.trim();
    match spec.kind {
        EmbedKind::Math { display } => {
            let (svg, depth_ratio) = math_svg(source, display, spec.color)?;
            let (image, width, height, bytes) = rasterize(&svg, scale_to_em, false)?;
            Ok((EmbedInfo { width, height, depth: height * depth_ratio }, image, bytes))
        }
        EmbedKind::Mermaid | EmbedKind::MermaidThemed(_) => {
            let palette = if let EmbedKind::MermaidThemed(palette) = spec.kind { Some(palette) } else { None };
            let svg = mermaid_svg_with_palette(source, spec.dark, palette, merman::OperationControl::new())?;
            let (image, width, height, bytes) = rasterize(&svg, scale_to_em, true)?;
            Ok((EmbedInfo { width, height, depth: 0.0 }, image, bytes))
        }
    }
}

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
const MERMAID_FONT_FAMILY: &str = "Hiragino Sans GB";
#[cfg(target_os = "windows")]
const MERMAID_FONT_FAMILY: &str = "Microsoft YaHei";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MERMAID_FONT_FAMILY: &str = "Noto Sans CJK SC";

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
fn mermaid_svg(source: &str, dark: bool) -> Result<String, String> {
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

fn mermaid_svg_with_palette(source: &str, dark: bool, palette: Option<MermaidPalette>, control: merman::OperationControl) -> Result<String, String> {
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

/// 栅格化 SVG：`scale` 为逻辑缩放（相对 SVG 自身尺寸），实际按 2 倍分辨率出图并转为 BGRA。
fn rasterize(svg: &str, scale: f32, mermaid_text: bool) -> Result<(RenderImage, f32, f32, usize), String> {
    let mut options = resvg::usvg::Options { fontdb: font_database(), ..Default::default() };
    if mermaid_text {
        options.font_family = MERMAID_FONT_FAMILY.into();
    }
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|error| format!("SVG 解析失败：{error}"))?;
    let size = tree.size();
    let (width, height) = (size.width() * scale, size.height() * scale);
    if width < 1.0 || height < 1.0 {
        return Err("渲染结果为空".into());
    }
    let mut raster = RASTER_SCALE;
    let longest = width.max(height) * raster;
    if longest > MAX_RASTER_SIDE {
        raster *= MAX_RASTER_SIDE / longest;
    }
    let (pixel_width, pixel_height) = ((width * raster).ceil() as u32, (height * raster).ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_width, pixel_height).ok_or("位图尺寸无效")?;
    let factor = scale * raster;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(factor, factor), &mut pixmap.as_mut());
    // tiny-skia 输出预乘 RGBA；GPUI 期望 BGRA（与其 img 元素的转换一致）。
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bytes = data.len();
    let buffer = RgbaImage::from_raw(pixel_width, pixel_height, data).ok_or("位图数据长度不匹配")?;
    Ok((RenderImage::new(vec![Frame::new(buffer)]), width, height, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_eviction_returns_embed_texture_and_preserves_budget() {
        let mut store = EmbedStore::default();
        let decoded = || Ok((EmbedInfo { width: 1.0, height: 1.0, depth: 0.0 },
            RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), MAX_CACHE_BYTES / 2));
        assert!(store.insert(1, decoded()).is_empty());
        assert!(store.insert(2, decoded()).is_empty());
        let oldest = store.image(1).unwrap();
        store.image(2);
        let retired = store.insert(3, decoded());
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &oldest));
        assert!(store.state(1).is_none());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
        assert_eq!(store.insert(2, Err("failed".into())).len(), 1);
        assert_eq!(store.bytes, MAX_CACHE_BYTES / 2);
        assert!(store.insert(2, decoded()).is_empty());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
    }

    #[test]
    fn renders_display_math_with_depth() {
        let spec = EmbedSpec { kind: EmbedKind::Math { display: true }, source: r"\frac{a}{b} + \sum_{i=1}^n x_i".into(), em_px: 16.0, color: [0, 0, 0], dark: false };
        let (info, _, bytes) = render(&spec).expect("render");
        assert!(info.width > 30.0 && info.height > 20.0, "{info:?}");
        assert!(info.depth > 0.0 && info.depth < info.height);
        assert!(bytes > 0);
    }

    #[test]
    fn renders_mermaid_flowchart_with_text() {
        let svg = mermaid_svg("flowchart TD\n  A[开始] --> B{判断}\n  B --> C[结束]", false).expect("svg");
        assert!(!svg.contains("<foreignObject"), "resvg 管线不应保留 foreignObject");
        assert!(svg.contains("开始"));
        let spec = EmbedSpec { kind: EmbedKind::Mermaid, source: "flowchart TD\n  A[开始] --> B[结束]".into(), em_px: 16.0, color: [0, 0, 0], dark: true };
        assert!(render(&spec).is_ok());
    }

    /// 目视检查用：`NOMO_EMBED_DUMP=<目录> NOMO_CORPUS=<测试集> cargo test --release dump_ -- --ignored`
    /// 把七类 Mermaid 样例与若干公式栅格化为 PNG。
    #[test]
    #[ignore]
    fn dump_samples_for_review() {
        let (Some(out), Some(corpus)) = (std::env::var_os("NOMO_EMBED_DUMP"), std::env::var_os("NOMO_CORPUS")) else { return };
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        let text = std::fs::read_to_string(std::path::Path::new(&corpus).join("04_代码公式与图表/07_七类Mermaid图表.md")).unwrap();
        let mut sources = Vec::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            match (&mut current, line.trim_start()) {
                (None, trimmed) if trimmed.starts_with("```mermaid") => current = Some(String::new()),
                (Some(block), trimmed) if trimmed.starts_with("```") => {
                    sources.push((EmbedKind::Mermaid, std::mem::take(block)));
                    current = None;
                }
                (Some(block), _) => {
                    block.push_str(line);
                    block.push('\n');
                }
                _ => {}
            }
        }
        sources.push((EmbedKind::Math { display: true }, r"\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}".into()));
        sources.push((EmbedKind::Math { display: true }, r"\begin{bmatrix}1&2\\3&4\end{bmatrix}".into()));
        for (index, (kind, source)) in sources.into_iter().enumerate() {
            for dark in [false, true] {
                let spec = EmbedSpec { kind, source: source.clone(), em_px: 16.0, color: if dark { [215, 220, 225] } else { [32, 36, 40] }, dark };
                let (_, image, _) = render(&spec).unwrap_or_else(|error| panic!("{index}: {error}"));
                let size = image.size(0);
                let mut data = image.as_bytes(0).unwrap().to_vec();
                for pixel in data.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let buffer = RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, data).unwrap();
                buffer.save(out.join(format!("{index}-{}.png", if dark { "dark" } else { "light" }))).unwrap();
            }
        }
    }

    #[test]
    fn renders_cjk_text_with_system_font_outlines() {
        let spec = EmbedSpec { kind: EmbedKind::Math { display: false }, source: r"E = mc^2 \text{（质能方程）}".into(), em_px: 16.0, color: [0, 0, 0], dark: false };
        let (info, image, bytes) = render(&spec).expect("中文公式应使用可用系统字体");
        assert!(info.width > 100.0 && info.height > 10.0 && bytes > 0, "{info:?}");
        assert!(info.depth >= 0.0 && info.depth < info.height);
        assert!(image.as_bytes(0).unwrap().chunks_exact(4).any(|pixel| pixel[3] != 0));
        let (svg, _) = math_svg(&spec.source, false, [0, 0, 0]).unwrap();
        assert!(svg.contains("<path") && !svg.contains("<text"));
    }

    #[test]
    fn cjk_fraction_scripts_and_spaces_keep_real_metrics() {
        let measure = |source: &str| {
            render(&EmbedSpec { kind: EmbedKind::Math { display: true }, source: source.into(), em_px: 16.0, color: [32, 36, 40], dark: false }).unwrap().0
        };
        let base = measure(r"\text{中文}");
        let fraction = measure(r"\frac{\text{中文}}{\text{分母}}");
        let scripts = measure(r"\text{中文}_{\text{下}}^{\text{上}}");
        let spaced = measure("\\text{中　文}");
        assert!(fraction.height > base.height && fraction.depth > 0.0);
        assert!(scripts.height > base.height && scripts.width > base.width);
        assert!(spaced.width > base.width);
        assert!(measure(r"\pmb{中文}").width >= base.width);
        assert!(measure(r"\hat{\text{中}}").height > measure(r"\text{中}").height);
    }

    #[test]
    fn math_still_reports_missing_fonts_and_unsupported_commands() {
        let error = math_svg("\\text{\u{10ffff}}", false, [0, 0, 0]).unwrap_err();
        assert!(error.contains("缺少可用字形"), "{error}");
        assert!(math_svg(r"\nomoUnknownCommand{x}", false, [0, 0, 0]).is_err());
        assert!(math_svg(&"x".repeat(64 * 1024 + 1), false, [0, 0, 0]).unwrap_err().contains("预算"));
    }
}
