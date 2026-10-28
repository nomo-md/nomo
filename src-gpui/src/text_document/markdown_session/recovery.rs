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
    pub change: Option<ExternalChange>,
    _lease: Arc<SessionLease>,
}
impl RecoveryCandidate {
    pub(crate) fn dismiss(&self)->Result<(),String> {
        let parent=self.root.canonicalize().map_err(|error|error.to_string())?;
        let target=self.root.join(&self.document_id).canonicalize().map_err(|error|error.to_string())?;
        if target.parent()!=Some(parent.as_path()) || target.file_name()!=Some(std::ffi::OsStr::new(&self.document_id)) {return Err("恢复目录身份不匹配，未清理任何资料".into());}
        // 仅在磁盘版本已成功挂载后释放用户明确放弃的私有草稿，绝不删除 manifest.path 指向的原文件。
        fs::remove_dir_all(target).map_err(|error|error.to_string())
    }
    pub(crate) fn discover(parent: &Path, current: &Path) -> TextDocumentResult<Vec<Self>> {
        let mut candidates = Vec::new();
        let entries = match fs::read_dir(parent) { Ok(entries) => entries, Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(candidates), Err(cause) => return Err(cause.into()) };
        for entry in entries {
            let entry = entry?;
            if entry.path() == current || !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() { continue; }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name.split('-').next().and_then(|value| value.parse::<u32>().ok()) else { continue; };
            if process_active(pid) { continue; }
            let lease = match SessionLease::acquire(&entry.path()) { Ok(lease) => Arc::new(lease), Err(_) => continue };
            for document in fs::read_dir(entry.path())?.take(256) {
                let document = document?;
                if !document.file_type()?.is_dir() || document.file_type()?.is_symlink() { continue; }
                let id = document.file_name().to_string_lossy().into_owned();
                if !id.starts_with("md-") || !id.bytes().all(|value| value.is_ascii_alphanumeric() || value == b'-') { continue; }
                let path = document.path().join("manifest.json");
                let file = match File::open(&path) { Ok(file) => file, Err(_) => continue };
                if file.metadata()?.len() > 1024 * 1024 { continue; }
                let manifest: Manifest = match serde_json::from_reader(file) { Ok(value) => value, Err(cause) => { crate::app_logger::warn("Recovery", &format!("恢复元数据无效，已保留：{cause}")); continue; } };
                if manifest.event.is_none() { continue; }
                let change=manifest.path.as_ref().and_then(|path|external::identity(path).ok().filter(|current|*current!=manifest.identity).map(|current|ExternalChange{path:path.clone(),revision:manifest.revision,deleted:current.is_none(),token:external::token(&current)}));
                let conflict=change.is_some();
                candidates.push(Self { root: entry.path(), document_id: id, path: manifest.path, conflict, change, _lease: lease.clone() });
                if candidates.len() == 32 { return Ok(candidates); }
            }
        }
        Ok(candidates)
    }
}
impl MarkdownDocumentSessionManager {
    pub(crate) fn recovery_candidates(&self) -> TextDocumentResult<Vec<RecoveryCandidate>> {
        RecoveryCandidate::discover(self.root.parent().ok_or_else(|| error("recovery-root", "会话目录缺少父目录"))?, &self.root)
    }
    pub(crate) fn adopt_recovery(&self, owner: &str, candidate: &RecoveryCandidate) -> TextDocumentResult<MarkdownStatus> {
        let target = self.root.join(&candidate.document_id);
        // 同卷原子移动，不读取或复制正文；失败保留源恢复目录。
        if !target.exists() { fs::rename(candidate.root.join(&candidate.document_id), &target)?; }
        self.recover(owner, &candidate.document_id)
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
