//! 只枚举退出进程的有界恢复元数据；正文和撤销文件继续由原会话按需恢复。
use super::*;
use std::path::Path;

pub(crate) struct SessionLease(pub File);
impl SessionLease {
    pub(crate) fn acquire(root: &Path) -> TextDocumentResult<Self> {
        fs::create_dir_all(root)?;
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || cache_cleanup::is_link(&meta) { return Err(error("session-root", "会话目录不是普通目录")); }
        match fs::symlink_metadata(root.join(".active")) {
            Ok(meta) if !meta.is_file() || cache_cleanup::is_link(&meta) => return Err(error("session-lock", "会话锁不是普通文件")),
            Err(cause) if cause.kind() != std::io::ErrorKind::NotFound => return Err(cause.into()),
            _ => {},
        }
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
    _lease: Option<Arc<SessionLease>>,
    legacy: Option<crate::legacy_records::Draft>,
}
impl RecoveryCandidate {
    pub(super) fn kept(root: &Path, manifest: &Manifest) -> Self {
        Self { root: root.to_owned(), document_id: manifest.document_id.clone(), path: manifest.path.clone(),
            conflict: manifest.legacy_unconfirmed, verified: true,
            modified: std::time::SystemTime::now(), change: None, _lease: None, legacy: None }
    }
    pub(crate) fn is_legacy(&self) -> bool {self.legacy.is_some()}
    pub(crate) fn label(&self) -> String {
        let name=self.legacy.as_ref().map(|draft|format!("旧版 · {}",draft.title)).unwrap_or_else(||self.path.as_ref().and_then(|path|path.file_name()).map(|name|name.to_string_lossy().into_owned()).unwrap_or_else(||"未命名".into()));
        let minutes=self.modified.elapsed().unwrap_or_default().as_secs()/60;
        let age=if minutes<1 {"刚刚".into()}else if minutes<60 {format!("约 {minutes} 分钟前")}else if minutes<1440 {format!("约 {} 小时前",minutes/60)}else{format!("约 {} 天前",minutes/1440)};
        format!("{name} · {age}")
    }
    pub(crate) fn dismiss(&self)->Result<(),String> {
        if let Some(draft)=&self.legacy {
            let _lease=SessionLease::acquire(&draft.root.join("gpui-legacy-records")).map_err(|e|e.to_string())?;
            return draft.mark_handled();
        }
        let parent=self.root.canonicalize().map_err(|error|error.to_string())?;
        let target=self.root.join(&self.document_id).canonicalize().map_err(|error|error.to_string())?;
        if target.parent()!=Some(parent.as_path()) || target.file_name()!=Some(std::ffi::OsStr::new(&self.document_id)) {return Err("恢复目录身份不匹配，未清理任何资料".into());}
        // 仅在磁盘版本已成功挂载后释放用户明确放弃的私有草稿，绝不删除 manifest.path 指向的原文件。
        fs::remove_dir_all(target).map_err(|error|error.to_string())
    }
    pub(crate) fn discover(parent: &Path, current: &Path, cancel:&std::sync::atomic::AtomicBool) -> TextDocumentResult<Vec<Self>> {
        let mut candidates = Vec::new();
        match fs::symlink_metadata(parent) {
            Ok(meta) if !meta.is_dir() || cache_cleanup::is_link(&meta) => return Err(error("recovery-root", "恢复目录不是普通目录")),
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(candidates),
            Err(cause) => return Err(cause.into()),
            _ => {},
        }
        let mut cleanup_budget = 65_536;
        let entries = match fs::read_dir(parent) { Ok(entries) => entries, Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(candidates), Err(cause) => return Err(cause.into()) };
        for (index, entry) in entries.enumerate() {
            if index == 4096 {
                crate::app_logger::warn("Recovery", "旧会话目录达到扫描上限，剩余资料已保留");
                break;
            }
            super::super::markdown_search::check_cancel(cancel)?;
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            if entry.path() == current || !meta.is_dir() || cache_cleanup::is_link(&meta) { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !cache_cleanup::numbered_name(&name, "", 2) { continue; }
            let Some(pid) = name.split('-').next().and_then(|value| value.parse::<u32>().ok()) else { continue; };
            if process_active(pid) { continue; }
            let lease = match SessionLease::acquire(&entry.path()) { Ok(lease) => Arc::new(lease), Err(_) => continue };
            if cleanup_budget > 0 {
                if let Err(cause) = cache_cleanup::clean(&entry.path(), &mut cleanup_budget, cancel) {
                    super::super::markdown_search::check_cancel(cancel)?;
                    crate::app_logger::warn("CacheCleanup", &format!("旧索引未能完全清理，已保留剩余文件：{cause}"));
                }
            }
            // 恢复列表仍最多 32 项，但清理继续遍历后续旧进程目录。
            if candidates.len() == 32 { continue; }
            for document in fs::read_dir(entry.path())?.take(256) {
                super::super::markdown_search::check_cancel(cancel)?;
                let document = document?;
                let meta = fs::symlink_metadata(document.path())?;
                if !meta.is_dir() || cache_cleanup::is_link(&meta) { continue; }
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
                let (conflict, change) = recovery_change(manifest.path.as_deref(), manifest.revision, &manifest.identity, manifest.legacy_unconfirmed);
                candidates.push(Self { root: entry.path(), document_id: id, path: manifest.path, conflict, verified, change, modified: file_modified(&path), _lease: Some(lease.clone()), legacy: None });
                if candidates.len() == 32 { break; }
            }
        }
        candidates.sort_by_key(|candidate|std::cmp::Reverse(candidate.modified));
        Ok(candidates)
    }
}
fn file_modified(path:&Path)->std::time::SystemTime {
    fs::metadata(path).and_then(|metadata|metadata.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

fn recovery_change(path: Option<&Path>, revision: u64, expected: &Option<BaselineIdentity>, unconfirmed: bool) -> (bool, Option<ExternalChange>) {
    let Some(path) = path else { return (false, None); };
    let observation = external::identity(path).and_then(|current| {
        if !unconfirmed && &current == expected { return Ok(None); }
        ExternalChange::capture(path, revision, &current).map(Some)
    });
    match observation {
        Ok(change) => (change.is_some(), change),
        Err(cause) => {
            crate::app_logger::warn("Recovery", &format!("无法核对草稿的磁盘版本，保留恢复资料：{cause}"));
            (true, None)
        }
    }
}

/// 按片段和 64 KiB 比较缓冲检查保存点，不挂载会话、清空历史或建立正文镜像。
fn has_unsaved_changes(root:&Path,manifest:&Manifest,cancel:&std::sync::atomic::AtomicBool)->TextDocumentResult<bool> {
    if manifest.legacy_unconfirmed {return Ok(true);}
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
    pub(crate) fn process_directory(&self) -> &str {
        self.root.file_name().and_then(|name| name.to_str()).unwrap_or_default()
    }
    pub(crate) fn refresh_recovery_location(&self,candidate:&mut RecoveryCandidate) {
        if self.root.join(&candidate.document_id).is_dir() {candidate.root=self.root.clone();candidate.legacy=None;}
    }
    pub(crate) fn recovery_candidates(&self, kept: std::collections::BTreeMap<String, Vec<String>>, cancel:&std::sync::atomic::AtomicBool) -> TextDocumentResult<(Vec<RecoveryCandidate>,Option<String>)> {
        let parent=self.root.parent().ok_or_else(|| error("recovery-root", "会话目录缺少父目录"))?;
        let mut candidates=RecoveryCandidate::discover(parent, &self.root, cancel)?;
        // 布局明确引用的草稿按目录和 ID 定位，不受意外退出扫描的 32 项上限影响。
        let mut known: std::collections::HashSet<_> = candidates.iter().map(|c| c.document_id.clone()).collect();
        for (process, ids) in kept {
            if !cache_cleanup::numbered_name(&process, "", 2) { continue; }
            let root = parent.join(process);
            if root == self.root || !fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir() && !cache_cleanup::is_link(&m)) { continue; }
            let lease = candidates.iter().find(|c| c.root == root).and_then(|c| c._lease.clone())
                .or_else(|| SessionLease::acquire(&root).ok().map(Arc::new));
            let Some(lease) = lease else { continue; };
            for id in ids {
                super::super::markdown_search::check_cancel(cancel)?;
                if known.contains(&id) || !id.starts_with("md-") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { continue; }
                let directory = root.join(&id);
                if !fs::symlink_metadata(&directory).is_ok_and(|m| m.is_dir() && !cache_cleanup::is_link(&m)) { continue; }
                let path = directory.join("manifest.json");
                if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !cache_cleanup::is_link(&m) && m.len() <= 1024 * 1024) { continue; }
                let Some(manifest) = File::open(&path).ok().and_then(|f| serde_json::from_reader::<_, Manifest>(f).ok()) else { continue; };
                if manifest.version != 1 || manifest.document_id != id || manifest.event.is_none() { continue; }
                known.insert(id);
                let mut candidate = RecoveryCandidate::kept(&root, &manifest);
                (candidate.conflict, candidate.change) = recovery_change(manifest.path.as_deref(), manifest.revision, &manifest.identity, manifest.legacy_unconfirmed);
                candidate._lease = Some(lease.clone());
                candidate.modified = file_modified(&path);
                candidates.push(candidate);
            }
        }
        let Some(data_root)=parent.parent().filter(|_|parent.file_name()==Some(std::ffi::OsStr::new("gpui-sessions"))) else {return Ok((candidates,None));};
        let (drafts,warning)=match crate::legacy_records::drafts(data_root) {Ok(result)=>result,Err(error)=>(Vec::new(),Some(error))};
        for draft in drafts {
            super::super::markdown_search::check_cancel(cancel)?;
            if candidates.iter().any(|candidate|candidate.document_id==draft.id) {continue;}
            let (_, change) = recovery_change(draft.path.as_deref(), 1, &None, true);
            candidates.push(RecoveryCandidate{root:data_root.into(),document_id:draft.id.clone(),path:draft.path.clone(),conflict:draft.path.is_some(),verified:true,modified:draft.modified,change,_lease:None,legacy:Some(draft)});
        }
        candidates.sort_by_key(|candidate|std::cmp::Reverse(candidate.modified));
        Ok((candidates,warning))
    }
    pub(crate) fn dismiss_recovery(&self,candidate:&RecoveryCandidate)->TextDocumentResult<()> {
        let _paths=lock(&self.path_operations)?;
        if lock(&self.sessions)?.contains_key(&candidate.document_id) {return Err(error("recovery-active","草稿正在文档中使用，未删除恢复资料"));}
        candidate.dismiss().map_err(|cause|error("recovery-dismiss",cause))
    }
    pub(crate) fn adopt_recovery(&self, owner: &str, candidate: &RecoveryCandidate, as_copy: bool, cancelled: impl Fn() -> bool) -> TextDocumentResult<MarkdownStatus> {
        let _paths=lock(&self.path_operations)?;
        if cancelled() {return Err(error("request-cancelled", "恢复已取消"));}
        if let Some(draft)=&candidate.legacy {return self.adopt_legacy_under_path_lock(owner,draft,as_copy,&cancelled);}
        let target = self.root.join(&candidate.document_id);
        // 同卷原子移动，不读取或复制正文；失败保留源恢复目录。
        if !target.exists() { fs::rename(candidate.root.join(&candidate.document_id), &target)?; }
        self.recover_under_path_lock(owner, &candidate.document_id, as_copy)
    }
}
#[cfg(windows)]
fn process_active(pid: u32) -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, STILL_ACTIVE}, System::Threading::{OpenProcess, GetExitCodeProcess, PROCESS_QUERY_LIMITED_INFORMATION}};
    unsafe { let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid); if handle.is_null() { return std::io::Error::last_os_error().raw_os_error() != Some(87); } let mut code = 0; let active = GetExitCodeProcess(handle, &mut code) == 0 || code == STILL_ACTIVE as u32; CloseHandle(handle); active }
}
#[cfg(unix)]
fn process_active(pid: u32) -> bool {
    unsafe extern "C" {fn kill(pid:i32,signal:i32)->i32;}
    if pid == 0 || pid > i32::MAX as u32 { return true; }
    unsafe {kill(pid as i32,0)==0 || std::io::Error::last_os_error().raw_os_error()!=Some(3)}
}
