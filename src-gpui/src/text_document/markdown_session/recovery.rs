//! 只枚举退出进程的有界恢复元数据；正文和撤销文件继续由原会话按需恢复。
use super::*;
use std::path::Path;

pub(crate) struct SessionLease(pub File);
impl SessionLease {
    pub(crate) fn acquire(root: &Path) -> TextDocumentResult<Self> {
        fs::create_dir_all(root)?;
        let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join(".active"))?;
        file.try_lock().map_err(|cause| error("session-active", cause.to_string()))?;
        Ok(Self(file))
    }
}
#[derive(Clone)]
pub(crate) struct RecoveryCandidate {
    root: PathBuf,
    pub document_id: String,
    pub path: Option<PathBuf>,
    pub conflict: bool,
    pub verified: bool,
    modified: std::time::SystemTime,
    pub change: Option<ExternalChange>,
    _lease: Arc<SessionLease>,
}
impl RecoveryCandidate {
    pub(crate) fn label(&self) -> String {
        let name=self.path.as_ref().and_then(|path|path.file_name()).map(|name|name.to_string_lossy().into_owned()).unwrap_or_else(||"未命名".into());
        let minutes=self.modified.elapsed().unwrap_or_default().as_secs()/60;
        let age=if minutes<1 {"刚刚".into()}else if minutes<60 {format!("约 {minutes} 分钟前")}else if minutes<1440 {format!("约 {} 小时前",minutes/60)}else{format!("约 {} 天前",minutes/1440)};
        format!("{name} · {age}")
    }
    pub(crate) fn dismiss(&self)->Result<(),String> {
        let parent=self.root.canonicalize().map_err(|error|error.to_string())?;
        let target=self.root.join(&self.document_id).canonicalize().map_err(|error|error.to_string())?;
        if target.parent()!=Some(parent.as_path()) || target.file_name()!=Some(std::ffi::OsStr::new(&self.document_id)) {return Err("恢复目录身份不匹配，未清理任何资料".into());}
        // 仅在磁盘版本已成功挂载后释放用户明确放弃的私有草稿，绝不删除 manifest.path 指向的原文件。
        fs::remove_dir_all(target).map_err(|error|error.to_string())
    }
    pub(crate) fn discover(parent: &Path, current: &Path, cancel:&std::sync::atomic::AtomicBool) -> TextDocumentResult<Vec<Self>> {
        let mut candidates = Vec::new();
        let entries = match fs::read_dir(parent) { Ok(entries) => entries, Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(candidates), Err(cause) => return Err(cause.into()) };
        for entry in entries {
            super::super::markdown_search::check_cancel(cancel)?;
            let entry = entry?;
            if entry.path() == current || !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name.split('-').next().and_then(|value| value.parse::<u32>().ok()) else { continue; };
            if process_active(pid) { continue; }
            let lease = match SessionLease::acquire(&entry.path()) { Ok(lease) => Arc::new(lease), Err(_) => continue };
            for document in fs::read_dir(entry.path())?.take(256) {
                super::super::markdown_search::check_cancel(cancel)?;
                let document = document?;
                if !document.file_type()?.is_dir() || document.file_type()?.is_symlink() { continue; }
                let id = document.file_name().to_string_lossy().into_owned();
                if !id.starts_with("md-") || !id.bytes().all(|value| value.is_ascii_alphanumeric() || value == b'-') { continue; }
                let path = document.path().join("manifest.json");
                let file = match File::open(&path) { Ok(file) => file, Err(_) => continue };
                if file.metadata()?.len() > 1024 * 1024 { continue; }
                let manifest: Manifest = match serde_json::from_reader(file) { Ok(value) => value, Err(cause) => { crate::app_logger::warn("Recovery", &format!("恢复元数据无效，已保留：{cause}")); continue; } };
                if manifest.event.is_none() { continue; }
                let verified=match has_unsaved_changes(&document.path(), &manifest, cancel) {
                    Ok(false) => continue,
                    Ok(true) => true,
                    Err(cause) => {crate::app_logger::warn("Recovery", &format!("无法完整核实草稿，保留手动恢复入口：{cause}"));false},
                };
                let change=manifest.path.as_ref().and_then(|path|external::identity(path).ok().filter(|current|*current!=manifest.identity).map(|current|ExternalChange{path:path.clone(),revision:manifest.revision,deleted:current.is_none(),token:external::token(&current)}));
                let conflict=change.is_some();
                candidates.push(Self { root: entry.path(), document_id: id, path: manifest.path, conflict, verified, change, modified: file_modified(&path), _lease: lease.clone() });
                if candidates.len() == 32 { candidates.sort_by_key(|candidate|std::cmp::Reverse(candidate.modified)); return Ok(candidates); }
            }
        }
        candidates.sort_by_key(|candidate|std::cmp::Reverse(candidate.modified));
        Ok(candidates)
    }
}
fn file_modified(path:&Path)->std::time::SystemTime {
    fs::metadata(path).and_then(|metadata|metadata.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// 按片段和 64 KiB 比较缓冲检查保存点，不挂载会话、清空历史或建立正文镜像。
fn has_unsaved_changes(root:&Path,manifest:&Manifest,cancel:&std::sync::atomic::AtomicBool)->TextDocumentResult<bool> {
    let raw=root.join("raw");
    let working=root.join("working");
    let (path,offset)=manifest.source_map.working_source(&raw,&working);
    let original=Arc::new(OriginalFile::open(path,offset)?);
    let added=Arc::new(AddedStore::open_read_only(root.join("added"))?);
    if fs::metadata(root.join("history"))?.len()>32*1024*1024 {return Err(error("recovery-budget","恢复历史较大，请手动恢复检查"));}
    let mut history=DiskHistory::open_read_only(&root.join("history"))?;
    let tree=history.replay_cancelled(PieceTree::from_original(manifest.source_map.utf8_len),manifest.event,cancel)?;
    let mut saved_tree_name=manifest.saved_tree.clone();
    let pending=root.join("pending-save.json");
    if pending.exists() {
        let file=File::open(pending)?;
        if file.metadata()?.len()>1024*1024 {return Err(error("recovery-budget","保存意图较大，请手动恢复检查"));}
        let pending:PendingSave=serde_json::from_reader(file).map_err(|cause|error("recovery-corrupt",cause.to_string()))?;
        if fs::metadata(&pending.path)?.len()>32*1024*1024 {return Err(error("recovery-budget","待确认保存文件较大，请手动恢复检查"));}
        if digest_file(&pending.path).ok()==Some(pending.digest) {saved_tree_name=Some(pending.saved_tree);}
    }
    let saved_tree=if let Some(name)=saved_tree_name {
        if !valid_saved_tree_name(&name) {return Err(error("recovery-corrupt","保存点引用无效"));}
        let file=File::open(root.join(name))?;
        if file.metadata()?.len()>8*1024*1024 {return Err(error("recovery-budget","保存点较大，请手动恢复检查"));}
        let pieces:Vec<Piece>=serde_json::from_reader(file).map_err(|cause|error("recovery-corrupt",cause.to_string()))?;
        apply_tree_edits(PieceTree::from_original(0),&[TreeEdit {from:0,to:0,pieces}])?
    }else{PieceTree::from_original(manifest.source_map.utf8_len)};
    let current=DocumentSnapshot {revision:manifest.revision,tree,original:original.clone(),added:added.clone()};
    let saved=DocumentSnapshot {revision:0,tree:saved_tree,original,added};
    Ok(!snapshots_equal_cancelled(&current,&saved,cancel)?)
}

impl MarkdownDocumentSessionManager {
    pub(crate) fn refresh_recovery_location(&self,candidate:&mut RecoveryCandidate) {
        if self.root.join(&candidate.document_id).is_dir() {candidate.root=self.root.clone();}
    }
    pub(crate) fn recovery_candidates(&self,cancel:&std::sync::atomic::AtomicBool) -> TextDocumentResult<Vec<RecoveryCandidate>> {
        RecoveryCandidate::discover(self.root.parent().ok_or_else(|| error("recovery-root", "会话目录缺少父目录"))?, &self.root, cancel)
    }
    pub(crate) fn dismiss_recovery(&self,candidate:&RecoveryCandidate)->TextDocumentResult<()> {
        let _paths=lock(&self.path_operations)?;
        if lock(&self.sessions)?.contains_key(&candidate.document_id) {return Err(error("recovery-active","草稿正在文档中使用，未删除恢复资料"));}
        candidate.dismiss().map_err(|cause|error("recovery-dismiss",cause))
    }
    pub(crate) fn adopt_recovery(&self, owner: &str, candidate: &RecoveryCandidate, as_copy: bool) -> TextDocumentResult<MarkdownStatus> {
        let _paths=lock(&self.path_operations)?;
        let target = self.root.join(&candidate.document_id);
        // 同卷原子移动，不读取或复制正文；失败保留源恢复目录。
        if !target.exists() { fs::rename(candidate.root.join(&candidate.document_id), &target)?; }
        self.recover_under_path_lock(owner, &candidate.document_id, as_copy)
    }
}
#[cfg(windows)]
fn process_active(pid: u32) -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, STILL_ACTIVE}, System::Threading::{OpenProcess, GetExitCodeProcess, PROCESS_QUERY_LIMITED_INFORMATION}};
    unsafe { let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid); if handle.is_null() { return std::io::Error::last_os_error().raw_os_error() == Some(5); } let mut code = 0; let active = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32; CloseHandle(handle); active }
}
#[cfg(unix)]
fn process_active(pid: u32) -> bool {
    unsafe extern "C" {fn kill(pid:i32,signal:i32)->i32;}
    unsafe {kill(pid as i32,0)==0 || std::io::Error::last_os_error().raw_os_error()==Some(1)}
}
