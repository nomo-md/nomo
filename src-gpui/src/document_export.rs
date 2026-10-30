//! 用户触发的原生导出：固定修订、单任务、有限临时正文与流式原子输出。
//! 不创建编辑 Buffer，不改变保存点；快照、资源和任务结束后立即释放。
use crate::{nomo_mark::{native_port::NativeWindowPort, session_window::WindowRequest},
    settings::Settings, text_document::markdown_session::SnapshotHandle, theme::{self, Family, Theme}};
use gpui::App;
use std::{fs::File, io::{BufWriter, Write}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

mod html;
mod resources;
mod model;
mod media;
mod pdf;
mod pdf_fonts;
mod word;
#[cfg(test)] mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format { Html, Pdf, Word }
impl Format {
    pub(crate) fn extension(self) -> &'static str { match self { Self::Html => "html", Self::Pdf => "pdf", Self::Word => "docx" } }
    pub(crate) fn label(self) -> &'static str { match self { Self::Html => "HTML", Self::Pdf => "PDF", Self::Word => "Word" } }
}

// 只由用户触发；排版、字体和资源均随这次任务释放。
struct Job<'a> { request: &'a WindowRequest, started: Instant }
impl Job<'_> {
    fn check(&self) -> Result<(), String> {
        self.request.check().map_err(|e| e.to_string())?;
        if self.started.elapsed() > Duration::from_secs(120) { return Err("导出超过 120 秒，已取消；已有输出文件未改动。".into()); }
        Ok(())
    }
}

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RESOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESOURCE_TOTAL: usize = 32 * 1024 * 1024;

static EXPORT_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) struct ExportPermit;
impl ExportPermit {
    pub(crate) fn acquire() -> Result<Self, String> {
        EXPORT_ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self).map_err(|_| "已有导出任务，请等待完成或关闭该文档取消。".into())
    }
}
impl Drop for ExportPermit { fn drop(&mut self) { EXPORT_ACTIVE.store(false, Ordering::Release); } }

pub(crate) struct Snapshot {
    port: NativeWindowPort,
    handle: SnapshotHandle,
    pub(crate) path: Option<PathBuf>,
    released: AtomicBool,
}
impl Snapshot {
    pub(crate) fn capture(port: NativeWindowPort, revision: u64, path: Option<PathBuf>) -> Result<Arc<Self>, String> {
        let handle = port.manager.capture(&port.owner, &port.document_id).map_err(|e| e.to_string())?;
        let snapshot = Arc::new(Self { port, handle, path, released: AtomicBool::new(false) });
        if snapshot.handle.revision != revision { return Err("文档已变化，请重新导出。".into()); }
        if snapshot.handle.byte_length > MAX_SOURCE_BYTES { return Err("导出暂支持 16 MiB 以内的 Markdown；原文未改动。".into()); }
        Ok(snapshot)
    }
    pub(crate) fn id(&self) -> &str { &self.handle.snapshot_id }
    pub(crate) fn release(&self) -> Result<(), String> {
        if !self.released.swap(true, Ordering::AcqRel) {
            if let Err(error) = self.port.manager.release(&self.port.owner, &self.handle.snapshot_id) {
                self.released.store(false, Ordering::Release);
                return Err(error.to_string());
            }
        }
        Ok(())
    }
    fn read(&self, request: &WindowRequest, started: Instant) -> Result<String, String> {
        let mut source = String::with_capacity(self.handle.byte_length as usize);
        let mut offset = 0;
        while offset < self.handle.byte_length {
            request.check().map_err(|e| e.to_string())?;
            if started.elapsed() > Duration::from_secs(60) { return Err("读取导出快照超时，已取消。".into()); }
            let part = self.port.manager.read_snapshot(&self.port.owner, &self.handle.snapshot_id, offset,
                (offset + 64 * 1024).min(self.handle.byte_length)).map_err(|e| e.to_string())?;
            if part.from_byte != offset || part.to_byte <= offset || part.revision != self.handle.revision {
                return Err("导出快照不完整，请重新导出。".into());
            }
            offset = part.to_byte;
            source.push_str(&part.text);
        }
        // 原文只在这一次后台任务中临时存在；释放会话快照后不阻塞关闭与撤销。
        self.release()?;
        Ok(source)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) { if let Err(error) = self.release() { eprintln!("释放导出快照失败：{error}"); } }
}

pub(crate) struct Appearance {
    theme: Theme,
    font: String,
    mono: String,
    size: f32,
    line_height: f32,
    width: f32,
    justify: bool,
    line_numbers: bool,
}
impl Appearance {
    pub(crate) fn capture(cx: &App) -> Self {
        let settings = cx.global::<Settings>();
        Self { theme: cx.global::<Theme>().clone(),
            font: theme::font_for(Family::Text, false, false).family.to_string(),
            mono: theme::font_for(Family::Code, false, false).family.to_string(),
            size: settings.font_size, line_height: settings.line_height,
            width: settings.content_width_percent, justify: settings.typography,
            line_numbers: settings.code_line_numbers }
    }
}

#[derive(Default)]
pub(crate) struct ExportReport { pub(crate) warnings: Vec<&'static str> }
impl ExportReport {
    fn warn(&mut self, message: &'static str) { if !self.warnings.contains(&message) { self.warnings.push(message); } }
}

struct Output<'a> { writer: BufWriter<&'a mut File>, bytes: u64, request: &'a WindowRequest, started: Instant }
impl Output<'_> {
    fn check(&self) -> Result<(), String> {
        self.request.check().map_err(|e| e.to_string())?;
        if self.started.elapsed() > Duration::from_secs(60) { return Err("HTML 导出超过 60 秒，已取消；已有输出文件未改动。".into()); }
        Ok(())
    }
    fn raw(&mut self, value: &str) -> Result<(), String> {
        self.check()?;
        if self.bytes + value.len() as u64 > MAX_OUTPUT_BYTES { return Err("HTML 输出超过 128 MiB，已取消；已有输出文件未改动。".into()); }
        self.writer.write_all(value.as_bytes()).map_err(|e| e.to_string())?;
        self.bytes += value.len() as u64;
        Ok(())
    }
    fn text(&mut self, value: &str) -> Result<(), String> { self.escaped(value, false) }
    fn attr(&mut self, value: &str) -> Result<(), String> { self.escaped(value, true) }
    fn escaped(&mut self, mut value: &str, attribute: bool) -> Result<(), String> {
        while !value.is_empty() {
            let mut end = value.len().min(64 * 1024);
            while !value.is_char_boundary(end) { end -= 1; }
            let part = &value[..end];
            if attribute { self.raw(&html_escape::encode_double_quoted_attribute(part))?; }
            else { self.raw(&html_escape::encode_text(part))?; }
            value = &value[end..];
        }
        Ok(())
    }
    fn data(&mut self, mime: &str, bytes: &[u8]) -> Result<(), String> {
        self.raw("data:")?; self.raw(mime)?; self.raw(";base64,")?;
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        // 3 的倍数，分片不会在中间引入 base64 填充；不构造第二份完整媒体字符串。
        for input in bytes.chunks(12 * 1024) {
            let mut encoded = String::with_capacity(input.len().div_ceil(3) * 4);
            for part in input.chunks(3) {
                let bits = ((part[0] as u32) << 16) | ((part.get(1).copied().unwrap_or(0) as u32) << 8) | part.get(2).copied().unwrap_or(0) as u32;
                encoded.push(ALPHABET[(bits >> 18) as usize] as char);
                encoded.push(ALPHABET[((bits >> 12) & 63) as usize] as char);
                encoded.push(if part.len() > 1 { ALPHABET[((bits >> 6) & 63) as usize] as char } else { '=' });
                encoded.push(if part.len() > 2 { ALPHABET[(bits & 63) as usize] as char } else { '=' });
            }
            self.raw(&encoded)?;
        }
        Ok(())
    }
}

pub(crate) fn html(snapshot: Arc<Snapshot>, target: &Path, title: &str, appearance: Appearance, request: WindowRequest) -> Result<ExportReport, String> {
    if !target.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("html") || s.eq_ignore_ascii_case("htm")) {
        return Err("请使用 .html 或 .htm 扩展名。".into());
    }
    if snapshot.path.as_ref().is_some_and(|path| path == target || path.canonicalize().ok().zip(target.canonicalize().ok()).is_some_and(|(a,b)| a == b)) {
        return Err("导出路径不能覆盖当前 Markdown 原文。".into());
    }
    let started = Instant::now();
    let source = snapshot.read(&request, started)?;
    let mut report = ExportReport::default();
    crate::file_system::write_file_atomically_checked(target, |file| {
        let mut output = Output { writer: BufWriter::new(file), bytes: 0, request: &request, started };
        let mut renderer = html::Renderer::new(&mut output, &appearance, snapshot.path.as_deref(), &mut report);
        renderer.document(&source, title)?;
        output.check()?;
        output.writer.flush().map_err(|e| e.to_string())?;
        output.check()
    }, || request.check().map_err(|e| e.to_string()))?;
    Ok(report)
}

pub(crate) fn export(format: Format, snapshot: Arc<Snapshot>, target: &Path, title: &str, appearance: Appearance, request: WindowRequest) -> Result<ExportReport, String> {
    if format == Format::Html { return html(snapshot, target, title, appearance, request); }
    if !target.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(format.extension())) {
        return Err(format!("请使用 .{} 扩展名。", format.extension()));
    }
    if snapshot.path.as_ref().is_some_and(|path| path == target || path.canonicalize().ok().zip(target.canonicalize().ok()).is_some_and(|(a,b)| a == b)) {
        return Err("导出路径不能覆盖当前 Markdown 原文。".into());
    }
    let job = Job { request: &request, started: Instant::now() };
    let source = snapshot.read(&request, job.started)?;
    let mut report = ExportReport::default();
    let mut appearance = appearance;
    if format == Format::Word { appearance.theme = Theme::light(); }
    let mut model = model::parse(&source, &job, &mut report)?;
    drop(source);
    let assets = media::prepare(&mut model, snapshot.path.as_deref(), &appearance, &job, &mut report)?;
    job.check()?;
    crate::file_system::write_file_atomically_checked(target, |file| {
        match format {
            Format::Pdf => {
                let bytes = pdf::render(&model, assets, title, &appearance, snapshot.path.as_deref(), &job, &mut report)?;
                if bytes.len() as u64 > MAX_OUTPUT_BYTES { return Err("PDF 超过 128 MiB，已有输出文件未改动。".into()); }
                for chunk in bytes.chunks(64 * 1024) { job.check()?; file.write_all(chunk).map_err(|e| e.to_string())?; }
            },
            Format::Word => word::write(file, &model, assets, title, snapshot.path.as_deref(), &job, &mut report)?,
            Format::Html => unreachable!(),
        }
        job.check()
    }, || job.check())?;
    Ok(report)
}
