//! 文本剪贴板写入须先确认成功，剪切才允许删除原文。
use gpui::App;

#[cfg(not(target_os = "windows"))]
pub(crate) fn write_text(text: &str, cx: &App) -> Result<(), String> {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));
    if cx.read_from_clipboard().and_then(|item| item.text()).as_deref() == Some(text) {
        Ok(())
    } else {
        Err("剪贴板写入未确认，正文已保留".into())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn write_text(text: &str, _: &App) -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetActiveWindow() -> *mut c_void;
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }
    struct Memory(*mut c_void);
    impl Drop for Memory { fn drop(&mut self) { unsafe { GlobalFree(self.0); } } }
    struct Clipboard;
    impl Drop for Clipboard { fn drop(&mut self) { unsafe { CloseClipboard(); } } }
    let error = |stage| format!("{stage}：{}；正文已保留", std::io::Error::last_os_error());
    if text.contains('\0') { return Err("所选文本含空字符，无法无损复制；正文已保留".into()); }
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = Memory(GlobalAlloc(2, std::mem::size_of_val(wide.as_slice())));
        if memory.0.is_null() { return Err(error("分配剪贴板内存失败")); }
        let target = GlobalLock(memory.0);
        if target.is_null() { return Err(error("访问剪贴板内存失败")); }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
        GlobalUnlock(memory.0);
        // OpenClipboard(NULL) 后 EmptyClipboard 会清空所有者，SetClipboardData 随之失败。
        // 沿用文件复制入口的应用线程活动窗口，不创建隐藏常驻窗口或后台重试任务。
        let owner = GetActiveWindow();
        if owner.is_null() { return Err("没有可用的应用窗口，正文已保留".into()); }
        if OpenClipboard(owner) == 0 { return Err(error("无法打开剪贴板")); }
        let _clipboard = Clipboard;
        if EmptyClipboard() == 0 { return Err(error("无法清空剪贴板")); }
        if SetClipboardData(13, memory.0).is_null() { return Err(error("无法写入剪贴板")); }
        // 系统成功接管内存后，不再立即重新打开剪贴板与其他监听程序竞争。
        std::mem::forget(memory);
    }
    Ok(())
}
