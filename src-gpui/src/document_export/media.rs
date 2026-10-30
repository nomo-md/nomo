//! 离线媒体解析。单个输入 8 MiB、单次解码 64 MiB、保留位图合计 32 MiB。
use super::{Appearance, ExportReport, Job, MAX_RESOURCE_BYTES, MAX_RESOURCE_TOTAL, model::*};
use base64::Engine;
use image::{ImageDecoder, RgbaImage};
use std::{fs::File, io::{Cursor, Read}, path::Path, sync::{Arc, atomic::{AtomicBool, Ordering}}};

pub(super) struct Asset {
    pub pixels: RgbaImage, pub width: f32, pub height: f32, pub alt: String,
    pub block: bool, pub align: String,
}
pub(super) fn link(value: &str, path: Option<&Path>) -> Option<String> {
    let value = value.trim();
    if value.chars().any(char::is_control) { return None; }
    if value.starts_with('#') || value.is_empty() { return Some(value.into()); }
    if value.starts_with("//") { return Some(format!("https:{value}")); }
    if let Ok(url) = url::Url::parse(value) {
        if matches!(url.scheme(), "http" | "https" | "mailto" | "file") { return Some(url.into()); }
        if !Path::new(value).is_absolute() { return None; }
    }
    if Path::new(value).is_absolute() { return url::Url::from_file_path(value).ok().map(String::from); }
    url::Url::from_file_path(path?).ok()?.join(&value.replace('\\', "/")).ok().filter(|url| url.scheme() == "file").map(String::from)
}
pub(super) fn prepare(model: &mut Model, path: Option<&Path>, appearance: &Appearance, job: &Job, report: &mut ExportReport) -> Result<Vec<Asset>, String> {
    let mut state = Media { assets: Vec::new(), bytes: 0, count: 0, enhancements: 0, path, appearance, job, report };
    state.blocks(&mut model.blocks)?;
    for blocks in model.footnotes.values_mut() { state.blocks(blocks)?; }
    Ok(state.assets)
}
struct Media<'a> { assets: Vec<Asset>, bytes: usize, count: usize, enhancements: usize, path: Option<&'a Path>, appearance: &'a Appearance, job: &'a Job<'a>, report: &'a mut ExportReport }
impl Media<'_> {
    fn blocks(&mut self, blocks: &mut [Block]) -> Result<(), String> {
        for block in blocks {
            self.job.check()?;
            match block {
                Block::Paragraph(p) => self.runs(&mut p.runs)?,
                Block::Table(t) => { for c in &mut t.cells { self.runs(&mut c.runs)?; } },
                Block::Code { text, language } if language.eq_ignore_ascii_case("mermaid") => {
                    self.enhancements += 1;
                    let image = if self.enhancements <= 256 && text.len() <= 64 * 1024 {
                        crate::embeds::export_mermaid(text, self.appearance.theme.dark, self.appearance.theme.mermaid)
                            .and_then(|svg| self.svg(svg.as_bytes(), true, "Mermaid 图表".into(), String::new()))
                    } else { Err("增强渲染超过预算".into()) };
                    self.job.check()?;
                    match image {
                        Ok(asset) => { let i = self.keep(asset)?; *block = Block::Paragraph(Paragraph { runs: vec![Run { content: Content::Raster(i), style: Style::default() }], ..Default::default() }); },
                        Err(_) => self.report.warn("部分 Mermaid 图表未能渲染，已保留源码"),
                    }
                }, _ => {}
            }
        }
        Ok(())
    }
    fn keep(&mut self, asset: Asset) -> Result<usize, String> {
        self.bytes += asset.pixels.len();
        if self.bytes > MAX_RESOURCE_TOTAL { return Err("导出位图总量超过 32 MiB，已有输出文件未改动。".into()); }
        let id = self.assets.len(); self.assets.push(asset); Ok(id)
    }
    fn runs(&mut self, runs: &mut [Run]) -> Result<(), String> {
        for run in runs {
            self.job.check()?;
            let result = match &run.content {
                Content::Image(props) => {
                    self.count += 1;
                    if self.count > 2048 { return Err("图片数量超过导出预算。".into()); }
                    let result = self.image(&props.src, &props.alt, &props.width, &props.align);
                    if result.is_err() {
                        self.report.warn("部分图片无法嵌入，已保留替代文字和可用链接；网络图片不自动下载");
                        run.style.link = link(&props.src, self.path);
                    }
                    Some(result)
                },
                Content::Math(tex, display) => {
                    self.enhancements += 1;
                    let result = if tex.len() <= 64 * 1024 && self.enhancements <= 256 {
                        crate::embeds::math_svg(tex, *display, super::pdf::rgb_bytes(self.appearance.theme.fg)).and_then(|(svg, _)| self.svg(svg.as_bytes(), *display, tex.clone(), String::new()))
                    } else { Err("公式增强渲染超过预算".into()) };
                    if result.is_err() { self.report.warn("部分公式未能渲染，已保留公式源码"); }
                    Some(result)
                }, _ => None,
            };
            self.job.check()?;
            if let Some(result) = result {
                run.content = match result {
                    Ok(asset) => Content::Raster(self.keep(asset)?),
                    Err(_) => Content::Text(match &run.content { Content::Image(p) => format!("[图片：{}]", if p.alt.is_empty() { "无法嵌入" } else { &p.alt }), _ => run.label().to_owned() }),
                };
            }
        }
        Ok(())
    }
    fn image(&self, source: &str, alt: &str, width: &str, align: &str) -> Result<Asset, String> {
        let source = source.trim();
        if source.starts_with("//") || url::Url::parse(source).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) { return Err("网络图片不自动下载".into()); }
        let bytes = if let Some(data) = source.strip_prefix("data:") {
            let (header, content) = data.split_once(',').ok_or("无效图片 data URL")?;
            if !matches!(header, "image/png;base64" | "image/jpeg;base64" | "image/gif;base64" | "image/webp;base64" | "image/bmp;base64" | "image/svg+xml;base64")
                || content.len() > MAX_RESOURCE_BYTES * 4 / 3 + 4 { return Err("图片 data URL 超过预算".into()); }
            base64::engine::general_purpose::STANDARD.decode(content).map_err(|e| e.to_string())?
        } else {
            let path = crate::images::resolve_source(source, self.path.and_then(Path::parent)).ok_or("无法定位图片")?;
            let file = File::open(path).map_err(|e| e.to_string())?;
            let meta = file.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > MAX_RESOURCE_BYTES as u64 { return Err("图片不是普通文件或超过预算".into()); }
            let mut bytes = Vec::new(); file.take(MAX_RESOURCE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?; bytes
        };
        if bytes.len() > MAX_RESOURCE_BYTES { return Err("图片超过 8 MiB".into()); }
        self.job.check()?;
        let mut asset = match image::guess_format(&bytes) {
            Ok(format) => {
                let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
                let mut limits = image::Limits::default(); limits.max_alloc = Some(64 * 1024 * 1024);
                limits.max_image_width = Some(16_384); limits.max_image_height = Some(16_384); reader.limits(limits);
                let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
                let (w, h) = decoder.dimensions();
                if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16 * 1024 * 1024 || decoder.total_bytes() > 64 * 1024 * 1024 { return Err("图片解码超过预算".into()); }
                let image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
                let image = if w.max(h) > 2048 { image.resize(2048, 2048, image::imageops::FilterType::Triangle) } else { image };
                Asset { pixels: image.into_rgba8(), width: w as f32 * 0.75, height: h as f32 * 0.75, alt: alt.into(), block: true, align: align.into() }
            },
            Err(_) => self.svg(&bytes, true, alt.into(), align.into())?,
        };
        if let Some(value) = crate::nomo_mark::image_properties::ImageWidth::parse(width) {
            let w = value.resolve(481.9, 0.75).min(481.9);
            asset.height *= w / asset.width; asset.width = w;
        }
        Ok(asset)
    }
    fn svg(&self, bytes: &[u8], block: bool, alt: String, align: String) -> Result<Asset, String> {
        if bytes.len() > MAX_RESOURCE_BYTES { return Err("SVG 超过预算".into()); }
        let refused = Arc::new(AtomicBool::new(false));
        let (a, b) = (refused.clone(), refused.clone());
        let options = resvg::usvg::Options { fontdb: crate::embeds::font_database(),
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(move |_, _, _| { a.store(true, Ordering::Relaxed); None }),
                resolve_string: Box::new(move |_, _| { b.store(true, Ordering::Relaxed); None }),
            }, ..Default::default() };
        let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
        if refused.load(Ordering::Relaxed) { return Err("SVG 引用了外部媒体".into()); }
        let size = tree.size();
        if size.width().max(size.height()) > 32_768.0 || size.width() * size.height() > 16_777_216.0 { return Err("SVG 尺寸超过预算".into()); }
        let scale = (2048.0 / size.width().max(size.height())).min(2.0);
        let (w, h) = ((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("无法分配 SVG 位图")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        // PDF 与 OOXML 都使用非预乘 RGBA；不能复用 GPUI 的 BGRA 纹理。
        let mut bytes = pixmap.take();
        for px in bytes.chunks_exact_mut(4) { let alpha = u32::from(px[3]); if alpha > 0 { for c in &mut px[..3] { *c = ((u32::from(*c) * 255 + alpha / 2) / alpha).min(255) as u8; } } }
        let pixels = RgbaImage::from_raw(w, h, bytes).ok_or("SVG 像素长度错误")?;
        Ok(Asset { pixels, width: size.width() * 0.75, height: size.height() * 0.75, alt, block, align })
    }
}
