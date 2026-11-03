//! 离线媒体逐张解码，每处排版与可复用的 PNG 分开保存。
use super::{Appearance, ExportReport, Job, MAX_OUTPUT_BYTES, MAX_RESOURCE_BYTES, model::*};
use base64::Engine;
use image::{ImageDecoder, ImageEncoder, RgbaImage};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::File, io::{Cursor, Read, Seek, SeekFrom, Write}, ops::Range, path::Path, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tempfile::SpooledTempFile;

pub(super) struct Placement {
    pub image: usize, pub width: f32, pub height: f32, pub alt: String,
    pub block: bool, pub align: String,
}

pub(super) struct PreparedMedia {
    pub placements: Vec<Placement>,
    images: Vec<Range<u64>>,
    data: SpooledTempFile,
}

impl Default for PreparedMedia {
    fn default() -> Self {
        Self { placements: Vec::new(), images: Vec::new(), data: tempfile::spooled_tempfile(4 * 1024 * 1024) }
    }
}

impl PreparedMedia {
    pub fn image_count(&self) -> usize { self.images.len() }

    fn store_png(&mut self, pixels: RgbaImage) -> Result<usize, String> {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, pixels.width(), pixels.height(), image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("导出图片编码失败：{e}"))?;
        drop(pixels);
        let start = self.images.last().map_or(0, |range| range.end);
        let end = start + png.len() as u64;
        if end > MAX_OUTPUT_BYTES { return Err("导出图片编码后合计超过 128 MiB，已有输出文件未改动。".into()); }
        self.data.write_all(&png).map_err(|e| format!("导出图片暂存失败：{e}"))?;
        let id = self.images.len();
        self.images.push(start..end);
        Ok(id)
    }

    pub fn png_len(&self, id: usize) -> usize {
        let range = &self.images[id];
        (range.end - range.start) as usize
    }

    pub fn copy_png(&mut self, id: usize, output: &mut impl Write, job: &Job) -> Result<(), String> {
        self.data.seek(SeekFrom::Start(self.images[id].start)).map_err(|e| format!("读取暂存导出图片失败：{e}"))?;
        let mut remaining = self.png_len(id);
        let mut buffer = [0; 64 * 1024];
        while remaining > 0 {
            job.check()?;
            let chunk = remaining.min(buffer.len());
            self.data.read_exact(&mut buffer[..chunk]).map_err(|e| format!("读取暂存导出图片失败：{e}"))?;
            output.write_all(&buffer[..chunk]).map_err(|e| format!("写入导出图片失败：{e}"))?;
            remaining -= chunk;
        }
        Ok(())
    }
}

struct Bitmap {
    pixels: RgbaImage,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct Picture { id: usize, width: f32, height: f32 }

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
enum SourceKind { Image, Math(bool), Mermaid }

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
pub(super) fn prepare(model: &mut Model, path: Option<&Path>, appearance: &Appearance, job: &Job, report: &mut ExportReport) -> Result<PreparedMedia, String> {
    let mut state = Media { prepared: PreparedMedia::default(), sources: HashMap::new(), images_by_pixels: HashMap::new(),
        count: 0, enhancements: 0, path, appearance, job, report };
    state.blocks(&mut model.blocks)?;
    for blocks in model.footnotes.values_mut() { state.blocks(blocks)?; }
    Ok(state.prepared)
}
struct Media<'a> {
    prepared: PreparedMedia,
    sources: HashMap<(SourceKind, [u8; 32]), Picture>,
    images_by_pixels: HashMap<[u8; 32], usize>,
    count: usize, enhancements: usize, path: Option<&'a Path>, appearance: &'a Appearance,
    job: &'a Job<'a>, report: &'a mut ExportReport,
}
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
                        self.picture(SourceKind::Mermaid, text, |media| {
                            let svg = crate::embeds::export_mermaid(text, media.appearance.theme.dark, media.appearance.theme.mermaid)?;
                            media.svg(svg.as_bytes())
                        })?
                    } else { None };
                    self.job.check()?;
                    match image {
                        Some(picture) => {
                            let content = self.place(picture, true, "Mermaid 图表", "", "");
                            *block = Block::Paragraph(Paragraph { runs: vec![Run { content, style: Style::default() }], ..Default::default() });
                        }
                        None => self.report.warn("部分 Mermaid 图表未能渲染，已保留源码"),
                    }
                }, _ => {}
            }
        }
        Ok(())
    }
    fn picture(&mut self, kind: SourceKind, source: &str, render: impl FnOnce(&Self) -> Result<Bitmap, String>) -> Result<Option<Picture>, String> {
        let key = (kind, Sha256::digest(source.as_bytes()).into());
        if let Some(&picture) = self.sources.get(&key) { return Ok(Some(picture)); }
        let bitmap = render(self);
        self.job.check()?;
        // 无效或不可访问的源沿用替代文字。有效图像的编码、暂存错误必须终止原子导出。
        let Ok(Bitmap { pixels, width, height }) = bitmap else { return Ok(None); };
        let mut hash = Sha256::new();
        hash.update(pixels.width().to_be_bytes());
        hash.update(pixels.height().to_be_bytes());
        hash.update(pixels.as_raw());
        let digest: [u8; 32] = hash.finalize().into();
        let id = if let Some(&id) = self.images_by_pixels.get(&digest) {
            id
        } else {
            let id = self.prepared.store_png(pixels)?;
            self.images_by_pixels.insert(digest, id);
            id
        };
        let picture = Picture { id, width, height };
        self.sources.insert(key, picture);
        Ok(Some(picture))
    }

    fn place(&mut self, picture: Picture, block: bool, alt: &str, width: &str, align: &str) -> Content {
        let width = crate::nomo_mark::image_properties::ImageWidth::parse(width)
            .map_or(picture.width, |value| value.resolve(481.9, 0.75).min(481.9));
        let id = self.prepared.placements.len();
        self.prepared.placements.push(Placement { image: picture.id, width, height: picture.height * (width / picture.width),
            block, alt: alt.into(), align: align.into() });
        Content::Raster(id)
    }
    fn runs(&mut self, runs: &mut [Run]) -> Result<(), String> {
        for run in runs {
            self.job.check()?;
            let content = match &run.content {
                Content::Image(props) => {
                    self.count += 1;
                    if self.count > 2048 { return Err("图片数量超过导出预算。".into()); }
                    let src = props.src.trim();
                    match self.picture(SourceKind::Image, src, |media| media.image(src))? {
                        Some(picture) => self.place(picture, true, &props.alt, &props.width, &props.align),
                        None => {
                            self.report.warn("部分图片无法嵌入，已保留替代文字和可用链接；网络图片不自动下载");
                            run.style.link = link(src, self.path);
                            Content::Text(format!("[图片：{}]", if props.alt.is_empty() { "无法嵌入" } else { &props.alt }))
                        }
                    }
                },
                Content::Math(tex, display) => {
                    self.enhancements += 1;
                    let result = if tex.len() <= 64 * 1024 && self.enhancements <= 256 {
                        self.picture(SourceKind::Math(*display), tex, |media| {
                            let (svg, _) = crate::embeds::math_svg(tex, *display, super::pdf::rgb_bytes(media.appearance.theme.fg))?;
                            media.svg(svg.as_bytes())
                        })?
                    } else { None };
                    match result {
                        Some(picture) => self.place(picture, *display, tex, "", ""),
                        None => { self.report.warn("部分公式未能渲染，已保留公式源码"); Content::Text(tex.clone()) }
                    }
                }, _ => continue,
            };
            self.job.check()?;
            run.content = content;
        }
        Ok(())
    }
    fn image(&self, source: &str) -> Result<Bitmap, String> {
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
        match image::guess_format(&bytes) {
            Ok(format) => {
                let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
                let mut limits = image::Limits::default(); limits.max_alloc = Some(64 * 1024 * 1024);
                limits.max_image_width = Some(16_384); limits.max_image_height = Some(16_384); reader.limits(limits);
                let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
                let (w, h) = decoder.dimensions();
                if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16 * 1024 * 1024 || decoder.total_bytes() > 64 * 1024 * 1024 { return Err("图片解码超过预算".into()); }
                let image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
                let image = if w.max(h) > 2048 { image.resize(2048, 2048, image::imageops::FilterType::Triangle) } else { image };
                Ok(Bitmap { pixels: image.into_rgba8(), width: w as f32 * 0.75, height: h as f32 * 0.75 })
            },
            Err(_) => self.svg(&bytes),
        }
    }
    fn svg(&self, bytes: &[u8]) -> Result<Bitmap, String> {
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
        Ok(Bitmap { pixels, width: size.width() * 0.75, height: size.height() * 0.75 })
    }
}
