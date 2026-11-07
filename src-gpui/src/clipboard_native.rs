//! macOS/Linux 双格式写入；仅保留一个应用级所有者，正文不进入视图缓存。
use gpui::App;

#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "clipboard_linux.rs"]
mod platform;

pub(super) const MAX_BYTES: usize = 64 * 1024 * 1024;

struct RichClipboard(arboard::Clipboard);
impl gpui::Global for RichClipboard {}

pub(super) fn validate(text: &str, html: &str) -> Result<(), String> {
    if text.contains('\0') || html.contains('\0') {
        return Err("所选内容包含空字符，正文已保留".into());
    }
    // 包含 macOS 后端附加的 UTF-8 HTML 外壳；Linux 所有者最多保存这一份双格式内容。
    if text
        .len()
        .checked_add(html.len())
        .and_then(|size| size.checked_add(platform::HTML_OVERHEAD))
        .is_none_or(|size| size > MAX_BYTES)
    {
        return Err("剪贴板文本与 HTML 合计超过 64 MiB，正文已保留".into());
    }
    Ok(())
}

pub(super) fn write(text: &str, html: &str, cx: &mut App) -> Result<(), String> {
    validate(text, html)?;
    if cx.try_global::<RichClipboard>().is_none() {
        let clipboard = arboard::Clipboard::new()
            .map_err(|error| format!("无法打开系统剪贴板：{error}；正文已保留"))?;
        cx.set_global(RichClipboard(clipboard));
        // X11 必须在写入返回后继续服务数据请求；退出前释放，交还系统剪贴板管理器。
        // 只注册一次，不持有文档、窗口或输入控件，也不轮询剪贴板。
        cx.on_app_quit(|cx| {
            cx.remove_global::<RichClipboard>();
            async {}
        })
        .detach();
    }
    cx.global_mut::<RichClipboard>()
        .0
        .set_html(html, Some(text))
        .map_err(|error| format!("无法复制富文本：{error}；正文已保留"))?;
    // arboard 的公共契约不足以证明两种格式均已写入；有界回读两者后才允许剪切删除。
    platform::confirm(text, html).map_err(|error| format!("{error}；正文已保留"))
}

pub(super) fn read() -> Result<Option<String>, String> {
    platform::read()
}
