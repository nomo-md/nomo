//! 轮询文件身份；弱身份卷在观察到变更时记录摘要，覆盖前重新校验。
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct ExternalChange {
    pub path: PathBuf,
    pub revision: u64,
    pub deleted: bool,
    pub token: String,
    content_digest: Option<[u8; 32]>,
}
pub(super) fn identity(path: &std::path::Path) -> TextDocumentResult<Option<BaselineIdentity>> {
    match File::open(path) {
        Ok(file) => BaselineIdentity::read_file(path, &file).map(Some),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(cause) => Err(cause.into()),
    }
}
fn token(value: &Option<BaselineIdentity>) -> String { format!("{:x}", Sha256::digest(serde_json::to_vec(value).expect("file identity"))) }
impl ExternalChange {
    pub(super) fn capture(path: &std::path::Path, revision: u64, current: &Option<BaselineIdentity>) -> TextDocumentResult<Self> {
        let content_digest = current.as_ref().is_some_and(BaselineIdentity::needs_content_check)
            .then(|| digest_file(path)).transpose()?;
        Ok(Self { path: path.into(), revision, deleted: current.is_none(), token: token(current), content_digest })
    }

    pub(super) fn verify(&self, path: &std::path::Path) -> TextDocumentResult<()> {
        if path != self.path || token(&identity(path)?) != self.token
            || matches!(self.content_digest, Some(expected) if digest_file(path)? != expected) {
            return Err(error("external-change", "文件再次发生变化，请重新选择处理方式；当前编辑仍然保留"));
        }
        Ok(())
    }
}
impl MarkdownDocumentSessionManager {
    pub(crate) fn external_change(&self, owner: &str, document_id: &str, previous: Option<ExternalChange>) -> TextDocumentResult<Option<ExternalChange>> {
        let session = self.session(document_id)?;
        let (path, expected, revision) = { let session = lock(&session)?; session.owner(owner)?; (session.manifest.path.clone(), session.manifest.identity.clone(), session.manifest.revision) };
        let Some(path) = path else { return Ok(None); };
        let current = identity(&path)?;
        if current == expected { return Ok(None); }
        // 已提示的同一磁盘状态继续使用当时的摘要；实际覆盖前才再次读取。
        if let Some(mut previous) = previous.filter(|old| old.path == path && old.token == token(&current)) {
            previous.revision = revision;
            return Ok(Some(previous));
        }
        ExternalChange::capture(&path, revision, &current).map(Some)
    }
    pub(crate) fn save_after_external(&self, owner: &str, snapshot_id: &str, change: &ExternalChange, snapshot_before_save: bool) -> TextDocumentResult<MarkdownStatus> {
        self.save_impl(owner, snapshot_id, Some(change.path.clone()), true, snapshot_before_save, Some(change))
    }
}
