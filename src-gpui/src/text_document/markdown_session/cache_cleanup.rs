//! 只回收已取得旧进程租约的派生索引，绝不删除会话正文、历史或恢复元数据。
use std::{fs, io, path::Path, sync::atomic::{AtomicBool, Ordering}};

pub(super) fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { meta.is_symlink() }
}

pub(super) fn numbered_name(name: &str, prefix: &str, count: usize) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else { return false; };
    let mut parts = rest.split('-');
    (0..count).all(|_| parts.next().is_some_and(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit())))
        && parts.next().is_none()
}

fn step(budget: &mut usize, cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "旧索引清理已取消"));
    }
    if *budget == 0 {
        return Err(io::Error::other("旧索引清理达到本次扫描上限，剩余文件留待下次启动"));
    }
    *budget -= 1;
    Ok(())
}

fn structure_file(name: &str) -> bool {
    [".sqlite", ".sqlite-journal", ".sqlite-wal", ".sqlite-shm"].iter().any(|suffix| {
        name.strip_suffix(suffix).is_some_and(|stem| numbered_name(stem, "structure-", 3))
    })
}

/// 调用者必须在整个操作期间持有该进程目录的 SessionLease。
pub(super) fn clean(root: &Path, budget: &mut usize, cancel: &AtomicBool) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        step(budget, cancel)?;
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if is_link(&meta) { continue; }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if meta.is_file() && structure_file(&name) {
            fs::remove_file(path)?;
        } else if meta.is_dir() && numbered_name(&name, "md-", 3) {
            let stats = path.join("writing-stats");
            match fs::symlink_metadata(&stats) {
                Ok(meta) if meta.is_dir() && !is_link(&meta) => clean_stats(&stats, 0, budget, cancel)?,
                Ok(_) => {},
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn clean_stats(root: &Path, depth: usize, budget: &mut usize, cancel: &AtomicBool) -> io::Result<()> {
    // 不使用递归删除：逐项拒绝符号链接和 Windows junction，限制深度及总扫描量。
    if depth >= 8 { return Err(io::Error::other("统计缓存目录过深，已保留")); }
    for entry in fs::read_dir(root)? {
        step(budget, cancel)?;
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if is_link(&meta) { continue; }
        if meta.is_dir() { clean_stats(&path, depth + 1, budget, cancel)?; }
        else if meta.is_file() { fs::remove_file(path)?; }
    }
    // 含链接或未知对象的目录保持原样；只移除已经清空的派生目录。
    match fs::remove_dir(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(error),
    }
}
