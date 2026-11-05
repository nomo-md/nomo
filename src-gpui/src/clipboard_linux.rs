//! Linux 不调用 arboard 的整块读取：Wayland 读取有界管道，X11 读取有界属性分片。
use super::MAX_BYTES;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

#[path = "clipboard_x11.rs"]
mod x11;

pub(super) const HTML_OVERHEAD: usize = 0;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
enum Format {
    Html,
    Text,
}

pub(super) fn read() -> Result<Option<String>, String> {
    let Some(mut html) = read_format(Format::Html)? else {
        return Ok(None);
    };
    // 仅粘贴时移除编码标记；写后校验必须保留用户原文中的 U+FEFF。
    if html.starts_with('\u{feff}') {
        html.drain(..3);
    }
    Ok((!html.is_empty()).then_some(html))
}

pub(super) fn confirm(text: &str, html: &str) -> Result<(), String> {
    if read_format(Format::Text)?.as_deref() != Some(text) {
        return Err("剪贴板纯文本写入未确认".into());
    }
    if read_format(Format::Html)?.as_deref() != Some(html) {
        return Err("剪贴板 HTML 写入未确认".into());
    }
    Ok(())
}

fn read_format(format: Format) -> Result<Option<String>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
        match read_wayland(format) {
            Ok(bytes) => return bytes.map(decode).transpose(),
            // 与 arboard 的后端选择一致：仅连接/协议不可用时尝试 X11；空剪贴板不读旧 X11 内容。
            Err(WaylandError::Unavailable(error)) if std::env::var_os("DISPLAY").is_some() => {
                return x11::read(format)
                    .map_err(|cause| format!("{error}；X11：{cause}"))?
                    .map(decode)
                    .transpose();
            }
            Err(WaylandError::Unavailable(error) | WaylandError::Read(error)) => return Err(error),
        }
    }
    x11::read(format)?.map(decode).transpose()
}

enum WaylandError {
    Unavailable(String),
    Read(String),
}

fn read_wayland(format: Format) -> Result<Option<Vec<u8>>, WaylandError> {
    let formats: &[MimeType<'_>] = match format {
        Format::Html => &[
            MimeType::Specific("text/html"),
            MimeType::Specific("text/html;charset=utf-8"),
        ],
        Format::Text => &[MimeType::Text],
    };
    for mime in formats {
        match paste::get_contents(ClipboardType::Regular, Seat::Unspecified, *mime) {
            Ok((pipe, _)) => return read_pipe(pipe).map(Some).map_err(WaylandError::Read),
            Err(paste::Error::NoMimeType) => continue,
            Err(paste::Error::ClipboardEmpty | paste::Error::NoSeats) => return Ok(None),
            Err(
                error @ (paste::Error::WaylandConnection(_)
                | paste::Error::SocketOpenError(_)
                | paste::Error::MissingProtocol { .. }),
            ) => {
                return Err(WaylandError::Unavailable(format!(
                    "Wayland 剪贴板不可用：{error}"
                )));
            }
            Err(error) => {
                return Err(WaylandError::Read(format!(
                    "无法读取 Wayland 剪贴板：{error}"
                )));
            }
        }
    }
    Ok(None)
}

#[repr(C)]
struct PollFd {
    fd: std::ffi::c_int,
    events: std::ffi::c_short,
    revents: std::ffi::c_short,
}
unsafe extern "C" {
    fn poll(fds: *mut PollFd, count: usize, timeout: std::ffi::c_int) -> std::ffi::c_int;
}

fn read_pipe(mut pipe: impl Read + AsRawFd) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; CHUNK_BYTES];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("读取剪贴板超时".into());
        }
        let mut descriptor = PollFd {
            fd: pipe.as_raw_fd(),
            events: 1,
            revents: 0,
        }; // POLLIN
        // pipe 是此函数独占的读端；poll 就绪后 read 不会被另一个读取者耗尽。
        let ready = unsafe { poll(&mut descriptor, 1, remaining.as_millis().max(1) as i32) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("等待剪贴板数据失败：{error}"));
        }
        if ready == 0 {
            return Err("读取剪贴板超时".into());
        }
        if descriptor.revents & (8 | 32) != 0 {
            return Err("剪贴板数据管道失效".into());
        } // POLLERR | POLLNVAL
        let length = chunk.len().min(MAX_BYTES - bytes.len() + 1);
        match pipe.read(&mut chunk[..length]) {
            Ok(0) => return Ok(bytes),
            Ok(count) => append(&mut bytes, &chunk[..count])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("读取剪贴板数据失败：{error}")),
        }
    }
}

fn append(bytes: &mut Vec<u8>, chunk: &[u8]) -> Result<(), String> {
    if chunk.len() > MAX_BYTES - bytes.len() {
        return Err("剪贴板文本超过 64 MiB 输入预算".into());
    }
    let required = bytes.len() + chunk.len();
    if required > bytes.capacity() {
        // 管道分片不一定是二次幂；普通 reserve 的翻倍可能在长度未超限时越过容量预算。
        let capacity = bytes
            .capacity()
            .saturating_mul(2)
            .max(CHUNK_BYTES)
            .max(required)
            .min(MAX_BYTES);
        bytes
            .try_reserve_exact(capacity - bytes.len())
            .map_err(|_| "无法分配剪贴板输入缓冲区")?;
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

fn decode(bytes: Vec<u8>) -> Result<String, String> {
    let utf16 = if bytes.starts_with(&[0xff, 0xfe]) {
        Some(true)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some(false)
    } else {
        None
    };
    if let Some(little_endian) = utf16 {
        if (bytes.len() - 2) % 2 != 0 {
            return Err("剪贴板 UTF-16 数据不完整".into());
        }
        let units = bytes[2..].chunks_exact(2).map(|pair| {
            let pair = [pair[0], pair[1]];
            if little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        });
        let mut text = String::new();
        for value in char::decode_utf16(units) {
            let value = value.map_err(|_| "剪贴板 UTF-16 数据无效")?;
            if value.len_utf8() > MAX_BYTES - text.len() {
                return Err("剪贴板文本超过 64 MiB 输入预算".into());
            }
            let required = text.len() + value.len_utf8();
            if required > text.capacity() {
                let capacity = text
                    .capacity()
                    .saturating_mul(2)
                    .max(CHUNK_BYTES)
                    .max(required)
                    .min(MAX_BYTES);
                text.try_reserve_exact(capacity - text.len())
                    .map_err(|_| "无法分配剪贴板文本缓冲区")?;
            }
            text.push(value);
        }
        return Ok(text);
    }
    String::from_utf8(bytes).map_err(|_| "剪贴板文本不是有效的 UTF-8 或带 BOM 的 UTF-16".into())
}
