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
use std::sync::Arc;

#[path = "embeds_native.rs"]
mod native;
pub use native::MermaidPalette;
pub(crate) use native::{math_svg, thumbnail_mermaid, export_mermaid, font_database};
use native::{mermaid_svg_with_palette, MERMAID_FONT_FAMILY};
#[cfg(test)] use native::mermaid_svg;

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
    Loading(u64),
    Ready { info: EmbedInfo, image: Arc<RenderImage>, bytes: usize, used: Cell<u64> },
    Failed(Arc<str>),
}

#[derive(Default)]
pub struct EmbedStore {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: Cell<u64>,
    next_request: u64,
    pub generation: u64,
}

impl Global for EmbedStore {}

impl EmbedStore {
    #[cfg(feature = "automation")]
    pub(crate) fn automation_state(&self) -> serde_json::Value {
        serde_json::json!({"entries":self.entries.len(),"decodedBytes":self.bytes,
            "pending":self.entries.values().filter(|e|matches!(e,Entry::Loading(_))).count(),
            "failed":self.entries.values().filter(|e|matches!(e,Entry::Failed(_))).count(),
            "scope":"process cache; requested resources only"})
    }
    /// 查询状态；返回 None 表示尚未请求，调用方应登记到 `request`。
    pub fn state(&self, key: u64) -> Option<EmbedState> {
        Some(match self.entries.get(&key)? {
            Entry::Loading(_) => EmbedState::Pending,
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

    /// 关闭文档后不继续保留位图、失败结果或异步任务的旧登记。
    pub(crate) fn retain(&mut self, keep: impl Fn(u64) -> bool) -> Vec<Arc<RenderImage>> {
        let mut retired = Vec::new();
        let old_len = self.entries.len();
        self.entries.retain(|key, entry| {
            if keep(*key) { return true; }
            if let Entry::Ready { image, bytes, .. } = entry { self.bytes -= *bytes; retired.push(image.clone()); }
            false
        });
        if old_len != self.entries.len() { self.generation = self.generation.wrapping_add(1); }
        retired
    }
    fn complete(&mut self, key: u64, request: u64, result: Result<(EmbedInfo, RenderImage, usize), String>) -> Vec<Arc<RenderImage>> {
        if !matches!(self.entries.get(&key), Some(Entry::Loading(current)) if *current == request) { return Vec::new(); }
        self.insert(key, result)
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
        store.next_request = store.next_request.wrapping_add(1);
        let request = store.next_request;
        store.entries.insert(key, Entry::Loading(request));
        let task = cx.background_executor().spawn(async move { render(&spec) });
        cx.spawn(async move |cx| {
            let result = task.await;
            cx.update(|cx| {
                let retired = cx.global_mut::<EmbedStore>().complete(key, request, result);
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
    fn closed_resources_release_textures_and_ignore_late_render() {
        let mut store = EmbedStore::default();
        let decoded = || Ok((EmbedInfo { width: 1.0, height: 1.0, depth: 0.0 },
            RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]), 4));
        store.insert(1, decoded());
        store.insert(2, decoded());
        store.entries.insert(3, Entry::Loading(1));
        store.insert(4, Err("failed".into()));
        assert_eq!(store.retain(|key| key == 1).len(), 1);
        assert_eq!((store.entries.len(), store.bytes), (1, 4));
        store.complete(3, 1, decoded());
        assert!(store.state(3).is_none());
        store.entries.insert(3, Entry::Loading(2));
        store.complete(3, 1, Err("late failure".into()));
        assert!(matches!(store.state(3), Some(EmbedState::Pending)));
        store.complete(3, 2, decoded());
        assert_eq!(store.bytes, 8);
        assert_eq!(store.retain(|_| false).len(), 2);
        assert_eq!((store.entries.len(), store.bytes), (0, 0));
    }

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
