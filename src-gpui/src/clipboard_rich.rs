//! Windows CF_HTML 使用 UTF-8 字节偏移；分配成功后才清空剪贴板。
use std::ffi::c_void;
#[path = "clipboard_images.rs"]
mod images;
pub(super) use images::read_images;
pub(super) use images::write_png;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(memory: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(memory: *mut c_void) -> i32;
    fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    fn GlobalSize(memory: *mut c_void) -> usize;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetActiveWindow() -> *mut c_void;
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn CloseClipboard() -> i32;
}
struct Memory(*mut c_void);
impl Drop for Memory { fn drop(&mut self) { unsafe { GlobalFree(self.0); } } }
struct Open;
impl Drop for Open { fn drop(&mut self) { unsafe { CloseClipboard(); } } }
fn error(stage: &str) -> String { format!("{stage}：{}；正文已保留", std::io::Error::last_os_error()) }
fn allocate(bytes: &[u8]) -> Result<Memory, String> {
    unsafe {
        let memory = Memory(GlobalAlloc(2, bytes.len()));
        if memory.0.is_null() { return Err(error("分配剪贴板内存失败")); }
        let target = GlobalLock(memory.0);
        if target.is_null() { return Err(error("访问剪贴板内存失败")); }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        GlobalUnlock(memory.0);
        Ok(memory)
    }
}
fn format() -> u32 {
    let name: Vec<u16> = "HTML Format".encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}
pub(super) fn write(text: &str, fragment: &str) -> Result<(), String> {
    if text.contains('\0') || fragment.contains('\0') { return Err("所选内容包含空字符，正文已保留".into()); }
    let prefix = "<html><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let header = |start: usize, end: usize, from: usize, to: usize| format!(
        "Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{from:010}\r\nEndFragment:{to:010}\r\n");
    let start = header(0, 0, 0, 0).len();
    let from = start + prefix.len(); let to = from + fragment.len(); let end = to + suffix.len();
    let html = format!("{}{prefix}{fragment}{suffix}\0", header(start, end, from, to));
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|v| v.to_le_bytes()).collect();
    let unicode = allocate(&bytes)?; let markup = allocate(html.as_bytes())?;
    unsafe {
        let owner = GetActiveWindow(); let html_format = format();
        if owner.is_null() || html_format == 0 { return Err(error("剪贴板格式不可用")); }
        if OpenClipboard(owner) == 0 { return Err(error("无法打开剪贴板")); }
        let _open = Open;
        if EmptyClipboard() == 0 { return Err(error("无法清空剪贴板")); }
        if SetClipboardData(13, unicode.0).is_null() { return Err(error("无法复制文本")); }
        std::mem::forget(unicode);
        if SetClipboardData(html_format, markup.0).is_null() { return Err(error("无法复制富文本")); }
        std::mem::forget(markup);
    }
    Ok(())
}
pub(super) fn read() -> Option<String> {
    unsafe {
        let format = format();
        if format == 0 || OpenClipboard(GetActiveWindow()) == 0 { return None; }
        let _open = Open;
        let memory = GetClipboardData(format);
        if memory.is_null() { return None; }
        let size = GlobalSize(memory);
        if size == 0 || size > 64 * 1024 * 1024 { return None; }
        let data = GlobalLock(memory);
        if data.is_null() { return None; }
        let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size);
        let result = (|| {
            let header = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
            let offset = |key: &str| header.lines().find_map(|line| line.strip_prefix(key)?.trim().parse::<usize>().ok());
            if let (Some(from), Some(to)) = (offset("StartFragment:"), offset("EndFragment:")) {
                if from <= to && to <= bytes.len() { return std::str::from_utf8(&bytes[from..to]).ok().map(str::to_owned); }
            }
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            let text = std::str::from_utf8(&bytes[..end]).ok()?;
            let from = text.find("<!--StartFragment-->")? + "<!--StartFragment-->".len();
            let to = text[from..].find("<!--EndFragment-->")? + from;
            Some(text[from..to].into())
        })();
        GlobalUnlock(memory);
        result
    }
}
