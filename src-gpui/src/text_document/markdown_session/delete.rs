use super::*;
use std::{collections::HashSet, io::ErrorKind, path::Path};

pub(crate) struct DeleteDocument {
    pub member: SessionMember,
    pub path: PathBuf,
    pub discard: bool,
    pub copy_to: Option<PathBuf>,
}

pub(crate) struct DeleteReport {
    pub closed: HashSet<String>,
    pub missing: HashSet<PathBuf>,
    pub failure: Option<std::io::Error>,
}

impl MarkdownDocumentSessionManager {
    /// 用户同意的修订与路径须同时匹配。磁盘删除和会话关闭之间不能插入打开或保存。
    pub(crate) fn delete_path(
        &self,
        path: &Path,
        directory: bool,
        documents: Vec<DeleteDocument>,
        observed_paths: Vec<PathBuf>,
        validate_target: impl Fn() -> TextDocumentResult<()>,
    ) -> TextDocumentResult<DeleteReport> {
        let _paths = lock(&self.path_operations)?;
        validate_target()?;
        let plans: HashMap<_, _> = documents
            .iter()
            .map(|document| (document.member.document_id.as_str(), document))
            .collect();
        let mut handles: Vec<_> = lock(&self.sessions)?
            .iter()
            .map(|(id, handle)| (id.clone(), handle.clone()))
            .collect();
        handles.sort_by(|a, b| a.0.cmp(&b.0));

        let mut affected = Vec::new();
        for (id, handle) in &handles {
            let session = lock(handle)?;
            if session.closed || !session.manifest.path.as_ref().is_some_and(|open| open.starts_with(path)) {
                continue;
            }
            let plan = plans.get(id.as_str()).filter(|plan| {
                session.manifest.path.as_ref() == Some(&plan.path) && (!session.dirty || plan.discard)
            }).ok_or_else(|| error("delete-changed", "受影响的文档已变化"))?;
            plan.member.validate(&session)?;
            self.require_released_snapshots(id, "delete", "文档仍有正在使用的完整快照")?;
            affected.push((session, *plan));
        }

        // 先核对所有目的地。原子写入副本，原会话的路径、保存点和撤销历史保持有效。
        let mut destinations = HashSet::new();
        let copies = affected.iter().filter_map(|(session, plan)| {
            plan.copy_to.as_ref().map(|destination| (session, destination))
        }).map(|(session, destination)| {
            let destination = copy_destination(destination, path)?;
            if !destinations.insert(destination.clone()) {
                return Err(error("delete-copy-exists", "多个副本使用了同一个文件名"));
            }
            Ok((session, destination))
        }).collect::<TextDocumentResult<Vec<_>>>()?;
        if copies.len() != documents.iter().filter(|plan| plan.copy_to.is_some()).count() {
            return Err(error("delete-changed", "需要另存的文档已变化"));
        }
        for (session, destination) in copies {
            let snapshot = CapturedSnapshot {
                document_id: session.manifest.document_id.clone(),
                content: session.current.clone(),
                root: session.root.clone(),
                source_map: session.manifest.source_map.clone(),
            };
            crate::file_system::write_new_file_atomically_with(&destination, |output| {
                write_snapshot(&snapshot, output).map_err(|cause| cause.to_string())
            }).map_err(|cause| error("delete-copy-failed", cause))?;
        }

        validate_target()?;
        let failure = if directory { fs::remove_dir_all(path) } else { fs::remove_file(path) }.err();
        // remove_dir_all 可能删除部分子项后失败，只有实际消失的文档可以关闭。
        let removed = |open: &Path| failure.is_none() || path_missing(open);
        let mut missing: HashSet<_> = observed_paths.into_iter().filter(|open| removed(open)).collect();
        let mut closed = HashSet::new();
        for (session, plan) in &mut affected {
            if removed(&plan.path) {
                session.closed = true;
                session.cleanup.remove = true;
                self.release_document_services(session);
                closed.insert(plan.member.document_id.clone());
                missing.insert(plan.path.clone());
            }
        }
        lock(&self.sessions)?.retain(|id, _| !closed.contains(id));
        Ok(DeleteReport { closed, missing, failure })
    }
}

fn path_missing(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|cause| cause.kind() == ErrorKind::NotFound)
}

fn copy_destination(destination: &Path, deleting: &Path) -> TextDocumentResult<PathBuf> {
    let parent = destination.parent().ok_or_else(|| error("delete-copy-failed", "副本路径缺少父目录"))?;
    let name = destination.file_name().ok_or_else(|| error("delete-copy-failed", "副本路径缺少文件名"))?;
    let destination = parent.canonicalize().map_err(|cause| error("delete-copy-failed", cause.to_string()))?.join(name);
    if destination.starts_with(deleting) {
        return Err(error("delete-copy-inside", "副本须保存到删除范围之外"));
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => Err(error("delete-copy-exists", "副本路径已存在")),
        Err(cause) if cause.kind() == ErrorKind::NotFound => Ok(destination),
        Err(cause) => Err(error("delete-copy-failed", cause.to_string())),
    }
}
