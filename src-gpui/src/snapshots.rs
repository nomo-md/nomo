//! 保存前复制磁盘旧版本；正式会话与开发宿主共用同一快照策略。
use std::path::Path;

/// 每个文件保留的快照数量上限，超出后删除最旧的。
const MAX_SNAPSHOTS_PER_FILE: usize = 20;

/// 把磁盘上的旧版本复制到 `gpui-snapshots/<文件名>/<时间戳>.md`。
pub(crate) fn write_snapshot(path: &Path) -> std::io::Result<()> {
    let root = crate::settings::snapshot_dir().ok_or_else(|| std::io::Error::other("无法确定快照目录"))?;
    let name = path.file_name().map_or_else(|| "untitled".into(), |name| name.to_string_lossy().into_owned());
    let dir = root.join(name);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis());
    std::fs::copy(path, dir.join(format!("{stamp}.md")))?;
    let mut existing: Vec<_> = std::fs::read_dir(&dir)?.filter_map(Result::ok).map(|entry| entry.path()).collect();
    if existing.len() > MAX_SNAPSHOTS_PER_FILE {
        existing.sort();
        for old in &existing[..existing.len() - MAX_SNAPSHOTS_PER_FILE] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(())
}
