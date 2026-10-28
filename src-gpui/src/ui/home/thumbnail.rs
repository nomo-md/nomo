//! 固定尺寸的文档页首缩略图。后台串行读取 32 KiB，复用原生 SVG/图片栅格化，不创建编辑器。
use super::library;
use image::{ImageEncoder, RgbaImage};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use sha2::{Digest, Sha256};
use std::{fs::File, io::{Cursor, Read}, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}, time::UNIX_EPOCH};

pub(super) const WIDTH: u32 = 448;
pub(super) const HEIGHT: u32 = 384;
const HEAD_BYTES: u64 = 32 * 1024;
const PAGE_WIDTH: f32 = WIDTH as f32;
const PAGE_HEIGHT: f32 = HEIGHT as f32;
const RENDER_VERSION: &[u8] = b"nomo-home-preview-3";

pub(super) struct Thumbnail {
    pub pixels: RgbaImage,
    pub title: String,
}

/// 使用稳定的目标主题，避免外壳颜色过渡的每一帧都重新生成位图。
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Palette {
    dark: bool,
    surface: u32,
    fg: u32,
    heading: u32,
    muted: u32,
    border: u32,
    quote_bg: u32,
    quote_fg: u32,
    quote_border: u32,
    code_bg: u32,
    code_fg: u32,
    table_bg: u32,
    table_border: u32,
    link: u32,
    inline_code_bg: u32,
    inline_code_fg: u32,
    pub font_family: String,
    mono_family: String,
}

impl Palette {
    pub(super) fn new(theme: &crate::theme::Theme) -> Self {
        let color = |value: gpui::Hsla| {
            let rgba = value.to_rgb();
            ((rgba.r * 255.0).round() as u32) << 16
                | ((rgba.g * 255.0).round() as u32) << 8
                | (rgba.b * 255.0).round() as u32
        };
        Self {
            dark: theme.dark, surface: color(theme.surface), fg: color(theme.fg),
            heading: color(theme.heading), muted: color(theme.muted), border: color(theme.border),
            quote_bg: color(theme.sidebar_active), quote_fg: color(theme.quote_fg), quote_border: color(theme.quote_border),
            code_bg: color(theme.code_bg), code_fg: color(theme.code_fg),
            table_bg: color(theme.table_header_bg), table_border: color(theme.table_border),
            link: color(theme.link), inline_code_bg: color(theme.inline_code_bg), inline_code_fg: color(theme.inline_code_fg),
            font_family: crate::theme::font_for(crate::theme::Family::Text, false, false).family.to_string(),
            mono_family: crate::theme::font_for(crate::theme::Family::Mono, false, false).family.to_string(),
        }
    }

    fn fingerprint(&self, hash: &mut Sha256) {
        hash.update([u8::from(self.dark)]);
        for color in [self.surface, self.fg, self.heading, self.muted, self.border, self.quote_bg,
            self.quote_fg, self.quote_border, self.code_bg, self.code_fg, self.table_bg, self.table_border,
            self.link, self.inline_code_bg, self.inline_code_fg] {
            hash.update(color.to_le_bytes());
        }
        hash.update(self.font_family.as_bytes());
        hash.update([0]);
        hash.update(self.mono_family.as_bytes());
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct InlineStyle { bold: bool, italic: bool, strike: bool, code: bool, link: bool }

struct Run { text: String, style: InlineStyle }

#[derive(Default)]
struct RichText { runs: Vec<Run>, chars: usize }

impl RichText {
    fn push(&mut self, text: &str, style: InlineStyle) {
        for ch in text.chars().take(4000usize.saturating_sub(self.chars)) {
            self.push_char(ch, style);
        }
    }

    fn push_char(&mut self, ch: char, style: InlineStyle) {
        if self.runs.last().is_none_or(|run| run.style != style) {
            // 只保留有限页首样式，恶意交替标记不会产生无界 tspan。
            if self.runs.len() >= 256 { return; }
            self.runs.push(Run { text: String::new(), style });
        }
        self.runs.last_mut().unwrap().text.push(ch);
        self.chars += 1;
    }

    fn plain(&self) -> String { self.runs.iter().map(|run| run.text.as_str()).collect() }
    fn is_blank(&self) -> bool { self.runs.iter().all(|run| run.text.trim().is_empty()) }
}

enum Block {
    Text { text: RichText, heading: u8, quote: bool, code: bool },
    Image { path: Option<PathBuf>, alt: String },
    Diagram(String),
    Math(String),
    Row { cells: Vec<RichText>, header: bool },
    Rule,
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("预览请求已取消".into()) } else { Ok(()) }
}

pub(super) fn load(path: &Path, palette: &Palette, cancel: &AtomicBool) -> Result<Thumbnail, String> {
    check(cancel)?;
    if !std::fs::metadata(path).map_err(|_| "文件已移动或无法访问")?.is_file() { return Err("此记录已不再是文件".into()); }
    let file = File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "文件已移动或删除".to_string(),
        std::io::ErrorKind::PermissionDenied => "没有读取此文件的权限".to_string(),
        _ => format!("无法读取文件：{e}"),
    })?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() { return Err("此记录已不再是文件".into()); }
    let mut bytes = Vec::new();
    file.take(HEAD_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    check(cancel)?;
    let text = decode_head(&bytes);
    let markdown = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"));
    let mut blocks = if markdown { parse(&text, path) } else {
        let mut content = RichText::default();
        content.push(&text, InlineStyle::default());
        vec![Block::Text { text: content, heading: 0, quote: false, code: true }]
    };
    // 标题复用同一次有界读取，不另开文件、不保留正文；页首标题不在位图中重复。
    let title = if matches!(blocks.first(), Some(Block::Text { heading: 1..=6, quote: false, code: false, .. })) {
        let Block::Text { text, .. } = blocks.remove(0) else { unreachable!() };
        text.plain().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect()
    } else { super::document_name(path).chars().take(160).collect() };
    let mut hash = Sha256::new();
    hash.update(RENDER_VERSION);
    palette.fingerprint(&mut hash);
    hash.update(&bytes);
    hash.update(metadata.len().to_le_bytes());
    hash.update(metadata.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos()).to_le_bytes());
    // 图片变化也会使文档预览失效；只检查页首至多两个本地资源。
    for block in &blocks {
        if let Block::Image { path: Some(path), .. } = block {
            hash.update(path.to_string_lossy().as_bytes());
            if let Ok(meta) = std::fs::metadata(path) {
                hash.update(meta.len().to_le_bytes());
                hash.update(meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos()).to_le_bytes());
            }
        }
    }
    let fingerprint = format!("{:x}", hash.finalize());
    if let Some(png) = library::cached_thumbnail(path, &fingerprint)? {
        let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(WIDTH);
        limits.max_image_height = Some(HEIGHT);
        limits.max_alloc = Some(u64::from(WIDTH * HEIGHT * 8));
        reader.limits(limits);
        if let Ok(image) = reader.decode() {
            if image.width() == WIDTH && image.height() == HEIGHT { check(cancel)?; return Ok(Thumbnail { pixels: image.into_rgba8(), title }); }
        }
    }
    check(cancel)?;
    let image = render(&blocks, palette, cancel)?;
    check(cancel)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(image.as_raw(), WIDTH, HEIGHT, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    check(cancel)?;
    library::store_thumbnail(path, &fingerprint, &png)?;
    Ok(Thumbnail { pixels: image, title })
}

fn decode_head(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) { return encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned(); }
    if bytes.starts_with(&[0xfe, 0xff]) { return encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned(); }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(error) if error.error_len().is_none() => String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned(),
        Err(_) => encoding_rs::GBK.decode(bytes).0.into_owned(),
    }
}

fn local_image(document: &Path, destination: &str) -> Option<PathBuf> {
    let base = url::Url::from_file_path(document).ok()?;
    let url = base.join(destination).ok()?;
    if url.scheme() != "file" || url.host_str().is_some_and(|host| !host.is_empty() && host != "localhost") { return None; }
    let path = url.to_file_path().ok()?;
    // 预览不主动访问网络图片、UNC 共享或 data URL。
    if path.to_string_lossy().starts_with("\\\\") && !path.to_string_lossy().starts_with("\\\\?\\") { return None; }
    Some(path)
}

fn parse(text: &str, path: &Path) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut pending = RichText::default();
    let mut inline = InlineStyle::default();
    let mut heading = 0;
    let mut quote = 0usize;
    let mut code = false;
    let mut diagram = false;
    let mut images = 0;
    let mut image_depth = 0usize;
    let mut table = false;
    let mut row = Vec::new();
    let mut list = Vec::new();
    let flush = |blocks: &mut Vec<Block>, pending: &mut RichText, heading, quote, code, diagram| {
        if !pending.is_blank() && blocks.len() < 28 {
            let text = std::mem::take(pending);
            if diagram { blocks.push(Block::Diagram(text.plain())); }
            else { blocks.push(Block::Text { text, heading, quote, code }); }
        } else { *pending = RichText::default(); }
    };
    let options = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_MATH;
    for event in Parser::new_ext(text, options).take(4096) {
        if blocks.len() >= 28 { break; }
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram);
                image_depth += 1;
                if images < 2 { blocks.push(Block::Image { path: local_image(path, &dest_url), alt: String::new() }); images += 1; }
            },
            Event::End(TagEnd::Image) => { image_depth = image_depth.saturating_sub(1); },
            Event::Text(text) if image_depth > 0 => {
                if let Some(Block::Image { alt, .. }) = blocks.last_mut() { alt.extend(text.chars().take(100)); }
            },
            Event::Start(Tag::Strong) => inline.bold = true,
            Event::End(TagEnd::Strong) => inline.bold = false,
            Event::Start(Tag::Emphasis) => inline.italic = true,
            Event::End(TagEnd::Emphasis) => inline.italic = false,
            Event::Start(Tag::Strikethrough) => inline.strike = true,
            Event::End(TagEnd::Strikethrough) => inline.strike = false,
            Event::Start(Tag::Link { .. }) => inline.link = true,
            Event::End(TagEnd::Link) => inline.link = false,
            Event::Start(Tag::Heading { level, .. }) => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); heading = level as u8; },
            Event::End(TagEnd::Heading(_)) => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); heading = 0; },
            Event::Start(Tag::BlockQuote(_)) => { quote += 1; },
            Event::End(TagEnd::BlockQuote(_)) => { flush(&mut blocks, &mut pending, heading, true, code, diagram); quote = quote.saturating_sub(1); },
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram);
                code = true;
                diagram = matches!(kind, CodeBlockKind::Fenced(ref language) if language.trim().eq_ignore_ascii_case("mermaid"));
            },
            Event::End(TagEnd::CodeBlock) => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); code = false; diagram = false; },
            Event::Start(Tag::List(start)) => { if list.len() < 16 { list.push(start); } },
            Event::End(TagEnd::List(_)) => { list.pop(); },
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram);
                if let Some(Some(number)) = list.last_mut() { pending.push(&format!("{number}. "), inline); *number = number.saturating_add(1); } else { pending.push("• ", inline); }
            },
            Event::End(TagEnd::Item) | Event::End(TagEnd::Paragraph) if !table => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); },
            Event::Start(Tag::Table(_)) => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); table = true; },
            Event::End(TagEnd::Table) => { table = false; },
            Event::End(TagEnd::TableCell) => { if row.len() < 6 { row.push(std::mem::take(&mut pending)); } else { pending = RichText::default(); } },
            Event::End(end @ (TagEnd::TableRow | TagEnd::TableHead)) => {
                if !row.is_empty() { blocks.push(Block::Row { cells: std::mem::take(&mut row), header: matches!(end, TagEnd::TableHead) }); }
            },
            Event::DisplayMath(source) => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); blocks.push(Block::Math(source.chars().take(512).collect())); },
            Event::Code(text) => pending.push(&text, InlineStyle { code: true, ..inline }),
            Event::Text(text) | Event::InlineMath(text) => pending.push(&text, inline),
            Event::SoftBreak | Event::HardBreak => pending.push("\n", inline),
            Event::TaskListMarker(done) => pending.push(if done { "☑ " } else { "☐ " }, inline),
            Event::Rule => { flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram); blocks.push(Block::Rule); },
            _ => {},
        }
    }
    flush(&mut blocks, &mut pending, heading, quote > 0, code, diagram);
    blocks
}

fn xml(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).collect::<String>().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn lines(text: &RichText, width: f32, size: f32, max: usize) -> Vec<RichText> {
    let mut result = Vec::new();
    let mut line = RichText::default();
    let mut used = 0.0;
    for (ch, style) in text.runs.iter().flat_map(|run| run.text.chars().map(move |ch| (ch, run.style))) {
        let advance = size * if ch.is_ascii() { 0.56 } else { 1.0 };
        if ch == '\n' || used + advance > width {
            result.push(std::mem::take(&mut line));
            used = 0.0;
            if result.len() >= max { return result; }
            if ch == '\n' { continue; }
        }
        if !ch.is_control() { line.push_char(ch, style); used += advance; }
    }
    if line.chars > 0 && result.len() < max { result.push(line); }
    result
}

fn paint_text(svg: &mut String, text: &RichText, x: f32, y: f32, size: f32, bold: bool, code: bool, color: u32, palette: &Palette) {
    let family = if code { &palette.mono_family } else { &palette.font_family };
    svg.push_str(&format!("<text x=\"{x}\" y=\"{y}\" font-family=\"{}\" font-size=\"{size}\" xml:space=\"preserve\">", xml(family)));
    for run in &text.runs {
        let style = run.style;
        let color = if style.link { palette.link } else if style.code { palette.inline_code_fg } else { color };
        let family = if style.code { &palette.mono_family } else { family };
        svg.push_str(&format!("<tspan font-family=\"{}\" font-weight=\"{}\" font-style=\"{}\" text-decoration=\"{}\" fill=\"#{color:06x}\">{}</tspan>",
            xml(family), if bold || style.bold { "700" } else { "400" }, if style.italic { "italic" } else { "normal" },
            if style.strike { "line-through" } else { "none" }, xml(&run.text)));
    }
    svg.push_str("</text>");
}

fn raster_svg(svg: &str, width: u32, height: u32, palette: &Palette) -> Result<RgbaImage, String> {
    if svg.len() > 256 * 1024 { return Err("图表超过预览预算".into()); }
    let options = resvg::usvg::Options {
        fontdb: crate::embeds::font_database(),
        font_family: palette.font_family.clone(),
        image_href_resolver: resvg::usvg::ImageHrefResolver { resolve_string: Box::new(|_, _| None), ..Default::default() },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or("预览尺寸无效")?;
    let scale = (width as f32 / tree.size().width()).min(height as f32 / tree.size().height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut data = pixmap.take();
    // 缓存 PNG 使用直通 RGBA；透明图表合成前撤销 tiny-skia 的预乘。
    for pixel in data.chunks_exact_mut(4) { if pixel[3] > 0 && pixel[3] < 255 { for i in 0..3 { pixel[i] = (u32::from(pixel[i]) * 255 / u32::from(pixel[3])).min(255) as u8; } } }
    RgbaImage::from_raw(width, height, data).ok_or_else(|| "预览位图无效".into())
}

fn image_asset(path: &Path) -> Result<RgbaImage, String> {
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() { return Err("图片不是普通文件".into()); }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 { return Err("图片超过预览预算".into()); }
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 { return Err("图片超过预览预算".into()); }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(16 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    Ok(reader.decode().map_err(|e| e.to_string())?.thumbnail(WIDTH, 160).into_rgba8())
}

fn render(blocks: &[Block], palette: &Palette, cancel: &AtomicBool) -> Result<RgbaImage, String> {
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{PAGE_WIDTH}\" height=\"{PAGE_HEIGHT}\"><rect width=\"100%\" height=\"100%\" fill=\"#{:06x}\"/>", palette.surface);
    let mut y = 0.0f32;
    let mut assets = Vec::new();
    let mut rich_assets = 0;
    if blocks.is_empty() { svg.push_str(&format!("<text x=\"0\" y=\"24\" font-size=\"18\" fill=\"#{:06x}\">暂无正文</text>", palette.muted)); }
    for block in blocks {
        check(cancel)?;
        if y > PAGE_HEIGHT - 24.0 { break; }
        match block {
            Block::Text { text, heading, quote, code } => {
                let size = match heading { 1 => 30.0, 2 => 26.0, 3..=6 => 22.0, _ => if *code { 16.0 } else { 18.0 } };
                let line_height = size * 1.55;
                let inset = if *quote || *code { 12.0 } else { 0.0 };
                let available = ((PAGE_HEIGHT - y - 8.0) / line_height).ceil().max(1.0) as usize;
                let rows = lines(text, PAGE_WIDTH - inset * 2.0, size, available.min(if *heading > 0 { 2 } else { 8 }));
                let height = rows.len() as f32 * line_height;
                if *quote || *code {
                    svg.push_str(&format!("<rect x=\"0\" y=\"{y}\" width=\"{PAGE_WIDTH}\" height=\"{}\" rx=\"8\" fill=\"#{:06x}\"/>", height + 12.0, if *quote { palette.quote_bg } else { palette.code_bg }));
                    if *quote { svg.push_str(&format!("<rect x=\"0\" y=\"{y}\" width=\"3\" height=\"{}\" fill=\"#{:06x}\"/>",height+12.0,palette.quote_border)); }
                    y += 6.0;
                }
                let color = if *heading > 0 { palette.heading } else if *code { palette.code_fg } else if *quote { palette.quote_fg } else { palette.fg };
                for line in rows {
                    paint_text(&mut svg, &line, inset, y + size, size, *heading > 0, *code, color, palette);
                    y += line_height;
                }
                y += if *heading > 0 || *quote || *code { 16.0 } else { 12.0 };
            },
            Block::Row { cells, header } => {
                let width = (PAGE_WIDTH - 2.0) / cells.len().max(1) as f32;
                for (index, cell) in cells.iter().enumerate() {
                    let x = 1.0 + index as f32 * width;
                    svg.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"32\" fill=\"#{:06x}\" stroke=\"#{:06x}\"/>",if *header { palette.table_bg } else { palette.surface },palette.table_border));
                    let label = lines(cell, width - 14.0, 16.0, 1).pop().unwrap_or_default();
                    paint_text(&mut svg, &label, x + 7.0, y + 22.0, 16.0, *header, false, palette.fg, palette);
                }
                y += 32.0;
            },
            Block::Rule => { y += 8.0; svg.push_str(&format!("<path d=\"M0 {y}H{PAGE_WIDTH}\" stroke=\"#{:06x}\"/>",palette.border)); y += 16.0; },
            Block::Image { .. } | Block::Diagram(_) | Block::Math(_) => {
                let (asset, label) = match block {
                    Block::Image { path, alt } => (path.as_ref().ok_or_else(|| "非本地图片".into()).and_then(|path| image_asset(path)), if alt.is_empty() { "图片".to_string() } else { alt.clone() }),
                    Block::Diagram(source) if rich_assets < 2 && source.len() <= 4096 => {
                        rich_assets += 1;
                        (crate::embeds::thumbnail_mermaid(source,palette.dark).and_then(|svg| raster_svg(&svg,WIDTH,144,palette)), "Mermaid 图表".into())
                    },
                    Block::Math(source) if rich_assets < 2 && source.len() <= 512 => {
                        rich_assets += 1;
                        let color = [(palette.fg >> 16) as u8, (palette.fg >> 8) as u8, palette.fg as u8];
                        (crate::embeds::math_svg(source,true,color).and_then(|(svg,_)| raster_svg(&svg,WIDTH,96,palette)), "公式".into())
                    },
                    _ => (Err("资源超过预览预算".into()), "图表 / 公式".into()),
                };
                check(cancel)?;
                match asset {
                    Ok(asset) => {
                        let height = asset.height() as f32 * PAGE_WIDTH / WIDTH as f32;
                        let x = (WIDTH.saturating_sub(asset.width()) / 2) as i64;
                        assets.push((asset,x,(y * WIDTH as f32 / PAGE_WIDTH) as i64));
                        y += height + 16.0;
                    },
                    Err(_) => {
                        svg.push_str(&format!("<rect x=\"0\" y=\"{y}\" width=\"{PAGE_WIDTH}\" height=\"64\" rx=\"8\" fill=\"#{:06x}\"/><text x=\"12\" y=\"{}\" font-size=\"16\" fill=\"#{:06x}\">{}</text>",palette.code_bg,y+37.0,palette.muted,xml(&format!("{} · 打开文档查看",label.chars().take(20).collect::<String>()))));
                        y += 80.0;
                    },
                }
            },
        }
    }
    svg.push_str("</svg>");
    check(cancel)?;
    let mut image = raster_svg(&svg,WIDTH,HEIGHT,palette)?;
    for (asset,x,y) in assets { image::imageops::overlay(&mut image,&asset,x,y); }
    Ok(image)
}
