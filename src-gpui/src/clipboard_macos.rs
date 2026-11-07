//! NSPasteboard 原生字符串先检查 UTF-8 长度，再创建 Rust 副本。
use super::MAX_BYTES;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};

// 与锁定的 arboard 3.6.1 macOS HTML 外壳一致，避免将系统自动添加的编码声明判为失败。
const HTML_PREFIX: &str = "<html><head><meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\"></head><body>";
const HTML_SUFFIX: &str = "</body></html>";
pub(super) const HTML_OVERHEAD: usize = HTML_PREFIX.len() + HTML_SUFFIX.len();
const MAX_ITEMS: usize = 32;

fn first_string(pasteboard: &NSPasteboard, html: bool) -> Result<Option<String>, String> {
    let items = pasteboard
        .pasteboardItems()
        .ok_or("无法读取系统剪贴板项目")?;
    if items.len() > MAX_ITEMS {
        return Err("剪贴板项目超过 32 项".into());
    }
    // 只读取首个匹配项目，避免 stringForType 在 pasteboard 层合并全部项目。
    let format = unsafe {
        if html {
            NSPasteboardTypeHTML
        } else {
            NSPasteboardTypeString
        }
    };
    for item in items.iter() {
        if let Some(value) = item.stringForType(format) {
            if value.len() > MAX_BYTES {
                return Err("剪贴板文本超过 64 MiB 输入预算".into());
            }
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

pub(super) fn read() -> Result<Option<String>, String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let generation = pasteboard.changeCount();
    let html = first_string(&pasteboard, true)?;
    if generation != pasteboard.changeCount() {
        return Err("读取期间剪贴板内容已改变".into());
    }
    Ok(html.filter(|html| !html.is_empty()))
}

pub(super) fn confirm(text: &str, html: &str) -> Result<(), String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let generation = pasteboard.changeCount();
    if first_string(&pasteboard, false)?.as_deref() != Some(text) {
        return Err("剪贴板纯文本写入未确认".into());
    }
    let markup = first_string(&pasteboard, true)?.ok_or("剪贴板 HTML 写入未确认")?;
    let fragment = markup
        .strip_prefix(HTML_PREFIX)
        .and_then(|body| body.strip_suffix(HTML_SUFFIX));
    if fragment != Some(html) || generation != pasteboard.changeCount() {
        return Err("剪贴板 HTML 写入未确认或内容已改变".into());
    }
    Ok(())
}
