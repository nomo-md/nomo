//! 已打开文档的路径迁移：不改正文、窗口或历史，使用持久化意图恢复跨文件提交。
use super::*;
use std::path::{Path, PathBuf};

const INTENT: &str = "pending-rename.json";

#[derive(Serialize, Deserialize)]
struct RenameIntent {
    old_path: PathBuf,
    new_path: PathBuf,
    observed: BaselineIdentity,
    baseline_unchanged: bool,
}

pub(crate) struct RenameOutcome {
    pub warning: Option<String>,
}

fn same_file_id(before: &BaselineIdentity, after: &BaselineIdentity) -> bool {
    before.file_id.as_ref().zip(after.file_id.as_ref()).is_some_and(|(a, b)| a == b)
}

fn same_file(before: &BaselineIdentity, after: &BaselineIdentity) -> bool {
    same_file_id(before, after)
        && before.physical_len == after.physical_len
        && before.modified_nanos == after.modified_nanos
}

// 大小写不敏感的文件系统可能让旧、新路径同时指向同一对象；以实际目录项区分。
fn old_entry_exists(old: &Path, new: &Path) -> TextDocumentResult<Option<bool>> {
    let mut parent = PathBuf::new();
    for (old_part, new_part) in old.components().zip(new.components()) {
        if old_part == new_part {
            parent.push(old_part.as_os_str());
            continue;
        }
        let mut old_found = false;
        let mut new_found = false;
        for entry in fs::read_dir(parent)? {
            let name = entry?.file_name();
            old_found |= name == old_part.as_os_str();
            new_found |= name == new_part.as_os_str();
        }
        return Ok(match (old_found, new_found) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        });
    }
    Ok(None)
}

fn remap(path: &Path, source: &Path, destination: &Path) -> Option<PathBuf> {
    let suffix = path.strip_prefix(source).ok()?;
    Some(if suffix.as_os_str().is_empty() { destination.to_path_buf() } else { destination.join(suffix) })
}

fn remove_intent(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理重命名意图失败：{error}")),
    }
}

fn intent_exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

impl MarkdownDocumentSessionManager {
    /// 与所有打开和保存操作串行；文档内容和历史继续由原会话持有。
    pub(crate) fn rename_path(
        &self,
        source: &Path,
        destination: &Path,
        rename: impl FnOnce() -> Result<(), String>,
    ) -> Result<RenameOutcome, String> {
        let _paths = lock(&self.path_operations).map_err(|error| error.message)?;
        let mut handles: Vec<_> = lock(&self.sessions).map_err(|error| error.message)?
            .iter().map(|(id, handle)| (id.clone(), handle.clone())).collect();
        handles.sort_by(|a, b| a.0.cmp(&b.0));
        let mut affected = Vec::new();
        for (_, handle) in &handles {
            let session = lock(handle).map_err(|error| error.message)?;
            if session.closed { continue; }
            if let Some(path) = &session.manifest.path {
                if path.starts_with(source) { affected.push(handle.clone()); }
                else if path == destination || path.starts_with(destination) {
                    return Err("目标路径已由另一个文档会话使用。".into());
                }
            }
        }
        // 与批量会话移交使用相同顺序，避免同时持有多个会话锁时死锁。
        let mut sessions = affected.iter().map(|handle| lock(handle).map_err(|error| error.message)).collect::<Result<Vec<_>, _>>()?;
        let mut plans = Vec::new();
        for (index, session) in sessions.iter_mut().enumerate() {
            if session.closed { continue; }
            let Some(old_path) = session.manifest.path.as_ref() else { continue; };
            let Some(new_path) = remap(old_path, source, destination) else { continue; };
            // 前次保存/重命名若留下意图，先把当前有效 manifest 持久化再清理。
            for name in ["pending-save.json", INTENT] {
                let intent = session.root.join(name);
                if intent_exists(&intent).map_err(|error| format!("检查会话意图失败：{error}"))? {
                    session.persist(&session.manifest).map_err(|error| error.message)?;
                    remove_intent(&intent)?;
                }
            }
            let observed = BaselineIdentity::read(old_path).map_err(|error| format!("读取原文件状态失败：{}", error.message))?;
            if observed.file_id.is_none() { return Err("无法确认文件身份，不能安全重命名已打开的文档。".into()); }
            let baseline_unchanged = session.manifest.identity.as_ref() == Some(&observed);
            plans.push((index, RenameIntent { old_path: old_path.clone(), new_path, observed, baseline_unchanged }));
        }
        let mut written: Vec<PathBuf> = Vec::new();
        for (index, intent) in &plans {
            let path = sessions[*index].root.join(INTENT);
            let bytes = match serde_json::to_vec(intent) {
                Ok(bytes) => bytes,
                Err(error) => {
                    for path in &written { let _ = remove_intent(path); }
                    return Err(format!("序列化重命名意图失败：{error}"));
                }
            };
            if let Err(error) = crate::file_system::write_file_atomically(&path, &bytes) {
                for path in &written { let _ = remove_intent(path); }
                return Err(format!("保存重命名意图失败：{error}"));
            }
            written.push(path);
        }
        if let Err(error) = rename() {
            for path in &written { let _ = remove_intent(path); }
            return Err(error);
        }
        let mut warnings = Vec::new();
        for (index, intent) in plans {
            let session = &mut sessions[index];
            let after = BaselineIdentity::read(&intent.new_path);
            let mut manifest = session.manifest.clone();
            manifest.path = Some(intent.new_path.clone());
            if intent.baseline_unchanged && after.as_ref().is_ok_and(|identity| same_file(&intent.observed, identity)) {
                manifest.identity = after.as_ref().ok().cloned();
            } else if let Some(identity) = &mut manifest.identity {
                // 不将此前已发生的外部修改误认作新保存基线。
                identity.path = intent.new_path.to_string_lossy().into_owned();
            }
            let persisted = session.persist(&manifest);
            session.manifest = manifest;
            let displayed = intent.new_path.to_string_lossy().into_owned();
            for (_, status) in session.ack.values_mut() { status.path = Some(displayed.clone()); }
            for (_, result) in &mut session.search_ack { result.status.path = Some(displayed.clone()); }
            for (_, _, status) in &mut session.toc_ack { status.path = Some(displayed.clone()); }
            let problem = match persisted {
                Err(error) => Some(format!("文件已重命名，会话路径持久化失败：{}", error.message)),
                Ok(()) if after.is_err() => Some("文件已重命名，但无法读取新文件状态；保存前请检查文件。".to_string()),
                Ok(()) if after.as_ref().is_ok_and(|identity| !same_file(&intent.observed, identity)) => {
                    let _ = remove_intent(&session.root.join(INTENT));
                    Some("文件重命名期间发生外部变化，保存前请检查文件。".to_string())
                }
                Ok(()) => remove_intent(&session.root.join(INTENT)).err(),
            };
            if let Some(problem) = problem {
                session.persistence_warning = Some(problem.clone());
                warnings.push(problem);
            }
        }
        Ok(RenameOutcome { warning: (!warnings.is_empty()).then(|| warnings.join("；")) })
    }
}

/// 恢复时依据文件身份判断意图是在磁盘移动前还是移动后中断。
pub(super) fn recover_rename(root: &Path, manifest: &mut Manifest) -> TextDocumentResult<Option<PathBuf>> {
    let path = root.join(INTENT);
    if !intent_exists(&path)? { return Ok(None); }
    let intent: RenameIntent = serde_json::from_reader(File::open(&path)?)
        .map_err(|error| super::error("rename-recovery-corrupt", error.to_string()))?;
    if manifest.path.as_ref() != Some(&intent.old_path) && manifest.path.as_ref() != Some(&intent.new_path) {
        return Err(super::error("rename-recovery-conflict", "重命名意图与文档路径不一致，恢复资料已保留"));
    }
    let read_identity = |path: &Path| -> TextDocumentResult<Option<BaselineIdentity>> {
        if !intent_exists(path)? { return Ok(None); }
        Ok(Some(BaselineIdentity::read(path)?))
    };
    let old = read_identity(&intent.old_path)?.filter(|identity| same_file_id(&intent.observed, identity));
    let new = read_identity(&intent.new_path)?.filter(|identity| same_file_id(&intent.observed, identity));
    let (resolved, identity) = match (old, new) {
        (Some(identity), None) => (intent.old_path.clone(), identity),
        (None, Some(identity)) => (intent.new_path.clone(), identity),
        (Some(old), Some(new)) => match old_entry_exists(&intent.old_path, &intent.new_path)? {
            Some(true) => (intent.old_path.clone(), old),
            Some(false) => (intent.new_path.clone(), new),
            None => return Err(super::error("rename-recovery-conflict", "无法确定文件重命名状态，恢复资料已保留")),
        },
        _ => return Err(super::error("rename-recovery-conflict", "无法确定文件重命名状态，恢复资料已保留")),
    };
    manifest.path = Some(resolved.clone());
    if intent.baseline_unchanged && same_file(&intent.observed, &identity) { manifest.identity = Some(identity); }
    else if let Some(identity) = &mut manifest.identity { identity.path = resolved.to_string_lossy().into_owned(); }
    Ok(Some(path))
}

/// 新保存会替换文件身份；先完成仍待持久化的路径意图，避免恢复时误判。
pub(super) fn settle_pending_rename(session: &Session) -> TextDocumentResult<()> {
    let path = session.root.join(INTENT);
    if intent_exists(&path)? {
        session.persist(&session.manifest)?;
        fs::remove_file(path)?;
    }
    Ok(())
}
