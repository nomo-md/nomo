//! 导出媒体逐项读取，不建立媒体缓存；网络图片保留链接，不隐式下载。
use super::{MAX_RESOURCE_BYTES, MAX_RESOURCE_TOTAL, html::{Renderer, color}};
use crate::nomo_mark::image_properties::ImageProperties;
use std::{fs::File, io::Read, path::Path};

impl Renderer<'_, '_> {
    pub(super) fn link(&self, value: &str) -> Option<String> {
        let value = value.trim();
        if value.chars().any(char::is_control) { return None; }
        if value.starts_with('#') || value.is_empty() { return Some(value.into()); }
        if value.starts_with("//") { return Some(format!("https:{value}")); }
        if let Ok(url) = url::Url::parse(value) {
            if matches!(url.scheme(), "http" | "https" | "mailto" | "file") { return Some(url.to_string()); }
            // Windows 盘符不是 URL 协议。
            if !Path::new(value).is_absolute() { return None; }
        }
        if Path::new(value).is_absolute() { return url::Url::from_file_path(value).ok().map(String::from); }
        let base = self.path.and_then(|p| url::Url::from_file_path(p).ok())?;
        base.join(&value.replace('\\', "/")).ok().filter(|url| url.scheme() == "file").map(String::from)
    }
    pub(super) fn image(&mut self, props: &ImageProperties) -> Result<(), String> {
        self.output.check()?;
        self.resource_count += 1;
        if self.resource_count > 2048 { return Err("图片数量超过 HTML 导出预算。".into()); }
        enum Source { Url(String), Bytes(&'static str, Vec<u8>) }
        let source = (|| -> Result<Source, String> {
            let src = props.src.trim();
            if src.starts_with("//") && !src.chars().any(char::is_control) {
                self.report.warn("网络图片保留链接，查看时需要联网");
                return Ok(Source::Url(format!("https:{src}")));
            }
            if let Ok(url) = url::Url::parse(src) {
                if matches!(url.scheme(), "http" | "https") {
                    self.report.warn("网络图片保留链接，查看时需要联网");
                    return Ok(Source::Url(url.into()));
                }
            }
            if let Some(data) = src.strip_prefix("data:") {
                let (header, body) = data.split_once(',').ok_or("无效图片 data URL")?;
                if !matches!(header, "image/png;base64" | "image/jpeg;base64" | "image/gif;base64" | "image/webp;base64" | "image/bmp;base64" | "image/svg+xml;base64")
                    || body.len() > MAX_RESOURCE_BYTES * 4 / 3 + 4 || !body.len().is_multiple_of(4)
                    || !body.trim_end_matches('=').bytes().all(|b| b.is_ascii_alphanumeric() || b"+/".contains(&b))
                    || body.len() - body.trim_end_matches('=').len() > 2 { return Err("图片 data URL 超出支持范围".into()); }
                self.resource_bytes += body.len() * 3 / 4;
                if self.resource_bytes > MAX_RESOURCE_TOTAL { return Err("图片总量超出预算".into()); }
                return Ok(Source::Url(src.into()));
            }
            let path = crate::images::resolve_source(src, self.path.and_then(Path::parent)).ok_or("无法定位本地图片")?;
            let file = File::open(path).map_err(|e| e.to_string())?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() > MAX_RESOURCE_BYTES as u64 { return Err("图片不是普通文件或超过 8 MiB".into()); }
            let mut bytes = Vec::new();
            file.take(MAX_RESOURCE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if bytes.len() > MAX_RESOURCE_BYTES { return Err("图片超过 8 MiB".into()); }
            let mime = match image::guess_format(&bytes) {
                Ok(image::ImageFormat::Png) => "image/png", Ok(image::ImageFormat::Jpeg) => "image/jpeg",
                Ok(image::ImageFormat::Gif) => "image/gif", Ok(image::ImageFormat::WebP) => "image/webp",
                Ok(image::ImageFormat::Bmp) => "image/bmp",
                _ => { crate::images::validate_svg(&bytes)?; "image/svg+xml" }
            };
            self.resource_bytes += bytes.len();
            if self.resource_bytes > MAX_RESOURCE_TOTAL { return Err("图片总量超出 32 MiB".into()); }
            Ok(Source::Bytes(mime, bytes))
        })();
        let source = match source {
            Ok(source) => source,
            Err(_) => {
                self.report.warn("部分本地图片缺失、格式不支持或超过媒体预算，已保留说明");
                self.output.raw("<span class=\"resource-missing\" title=\"")?; self.output.attr(&props.src)?;
                self.output.raw("\">[图片：")?; self.output.text(if props.alt.is_empty() { &props.src } else { &props.alt })?;
                return self.output.raw("]</span>");
            }
        };
        self.output.raw("<img src=\"")?;
        match source { Source::Url(url) => self.output.attr(&url)?, Source::Bytes(mime, bytes) => self.output.data(mime, &bytes)? }
        self.output.raw("\" alt=\"")?; self.output.attr(&props.alt)?; self.output.raw("\" title=\"")?; self.output.attr(&props.title)?;
        self.output.raw("\" style=\"")?;
        if let Some(width) = crate::image_import::normalize_width(&props.width) { self.output.raw("width:")?; self.output.raw(&width)?; self.output.raw(";")?; }
        self.output.raw(match props.align.as_str() {
            "center" => "display:block;margin-left:auto;margin-right:auto", "left" => "display:block;margin-right:auto",
            "right" => "display:block;margin-left:auto", _ => ""
        })?;
        self.output.raw("\">")
    }
    pub(super) fn math(&mut self, source: &str, display: bool) -> Result<(), String> {
        self.output.check()?;
        self.embed_count += 1;
        let color = gpui::Rgba::from(self.appearance.theme.fg);
        let rendered = if self.embed_count > 256 { Err("渲染数量超过预算".into()) } else {
            crate::embeds::math_svg(source, display, [(color.r * 255.0) as u8, (color.g * 255.0) as u8, (color.b * 255.0) as u8])
        };
        let class = if display { "math-block" } else { "math-inline" };
        self.output.raw(&format!("<span class=\"{class}\">"))?;
        match rendered {
            Ok((svg, _)) if svg.len() <= MAX_RESOURCE_BYTES => {
                self.output.raw("<img alt=\"")?; self.output.attr(source)?; self.output.raw("\" src=\"")?;
                self.output.data("image/svg+xml", svg.as_bytes())?; self.output.raw("\">")?;
            }
            _ => { self.report.warn("部分公式或图表无法渲染，已保留源码"); self.output.raw("<code>")?; self.output.text(source)?; self.output.raw("</code>")?; }
        }
        self.output.raw("</span>")
    }
    pub(super) fn code(&mut self, source: &str, language: &str) -> Result<(), String> {
        self.output.check()?;
        if language.eq_ignore_ascii_case("mermaid") {
            self.embed_count += 1;
            let svg = if self.embed_count > 256 { Err("渲染数量超过预算".into()) } else {
                crate::embeds::export_mermaid(source, self.appearance.theme.dark, self.appearance.theme.mermaid)
            };
            if let Ok(svg) = svg.and_then(|svg| if svg.len() <= MAX_RESOURCE_BYTES { Ok(svg) } else { Err("图表输出超过预算".into()) }) {
                self.output.raw("<span class=\"mermaid\"><img alt=\"")?; self.output.attr(source)?;
                self.output.raw("\" src=\"")?; self.output.data("image/svg+xml", svg.as_bytes())?;
                return self.output.raw("\"></span>\n");
            }
            self.report.warn("部分公式或图表无法渲染，已保留源码");
        }
        self.output.raw("<section class=\"code-block\"><div class=\"code-language\">")?;
        self.output.text(if language.is_empty() { "text" } else { language })?;
        self.output.raw("</div><pre><code>")?;
        // 导出不填充编辑视图的 thread-local LRU，避免一次性文档在后台线程常驻。
        let spans = (source.len() <= 256 * 1024).then(|| crate::markdown::highlight::code_block_uncached(source, language));
        for (index, raw) in source.split_inclusive('\n').enumerate() {
            self.output.check()?;
            let line = raw.trim_end_matches(['\r', '\n']);
            if self.appearance.line_numbers { self.output.raw(&format!("<span class=\"line-number\" aria-hidden=\"true\">{}</span>", index + 1))?; }
            let mut at = 0;
            if let Some(spans) = spans.as_ref().and_then(|lines| lines.get(index)) {
                for (range, style) in spans {
                    if range.start < at || range.end > line.len() { return Err("代码高亮范围无效。".into()); }
                    self.output.text(&line[at..range.start])?;
                    self.output.raw(&format!("<span style=\"color:{};font-weight:{};font-style:{}\">", color(style.color.resolve(&self.appearance.theme)), if style.bold { "700" } else { "400" }, if style.italic { "italic" } else { "normal" }))?;
                    self.output.text(&line[range.clone()])?; self.output.raw("</span>")?; at = range.end;
                }
            }
            self.output.text(&line[at..])?;
            if raw.ends_with('\n') { self.output.raw("\n")?; }
        }
        self.output.raw("</code></pre></section>\n")
    }
}
