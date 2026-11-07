//! 原版原子写入实现；UI 适配层共用，不改变替换或编码策略。
use std::{fs::{self,OpenOptions},io::Write,path::{Path,PathBuf},process,time::{SystemTime,UNIX_EPOCH}};

pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_file_atomically_with(path, |writer| writer.write_all(bytes).map_err(|error| error.to_string()))
}

/// 完整快照流式写入；仅在编码、写入和同步全部成功后替换目标。
pub(crate) fn write_file_atomically_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    write_file_atomically_checked(path, write, || Ok(()))
}

/// 导出可在临时文件同步完成后、替换目标前再次检查取消状态。
pub(crate) fn write_file_atomically_checked(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
    before_replace: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temp_path = unique_temp_path(parent, path);

    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| format!("创建临时保存文件失败：{error}"))?;

    if let Err(error) = write(&mut temp_file) {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(format!("写入临时保存文件失败：{error}"));
    }
    if let Err(error) = temp_file.sync_all() {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(format!("同步临时保存文件失败：{error}"));
    }
    drop(temp_file);

    if let Err(error) = before_replace() {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("保存文件失败：{error}"));
    }

    sync_parent_dir(parent);
    Ok(())
}

fn unique_temp_path(parent: &Path, target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("document.md");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(".{file_name}.{}.{}.tmp", process::id(), nonce))
}

#[cfg(target_os = "windows")]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let mut from: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    from.push(0);
    let mut to: Vec<u16> = target_path.as_os_str().encode_wide().collect();
    to.push(0);

    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    fs::rename(temp_path, target_path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_dir(parent: &Path) {
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_parent_dir(_parent: &Path) {}
