use std::{path::PathBuf, sync::Mutex};

// 保持剪贴板实例存活，让 Linux 等需要应用持续提供数据的平台也能稍后粘贴。
#[derive(Default)]
pub(crate) struct FileClipboard(Mutex<Option<arboard::Clipboard>>);

/// 复制已保存的文件引用，供系统文件管理器及聊天软件粘贴为附件。
#[tauri::command]
pub(crate) fn copy_file_to_clipboard(
    path: String,
    state: tauri::State<'_, FileClipboard>,
) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.is_file() {
        return Err("File does not exist or is not an absolute file path".into());
    }
    let mut clipboard = state.0.lock().map_err(|error| error.to_string())?;
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
    }
    clipboard
        .as_mut()
        .ok_or("Clipboard is unavailable")?
        .set()
        .file_list(&[path])
        .map_err(|error| error.to_string())
}
