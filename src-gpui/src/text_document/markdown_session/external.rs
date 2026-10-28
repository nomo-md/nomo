//! 外部变更只读取文件身份；明确覆盖仍在原保存事务中二次校验，不缓存正文。
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct ExternalChange {
    pub path: PathBuf,
    pub revision: u64,
    pub deleted: bool,
    pub token: String,
}
pub(super) fn identity(path: &std::path::Path) -> TextDocumentResult<Option<BaselineIdentity>> {
    match fs::metadata(path) {
        Ok(_) => BaselineIdentity::read(path).map(Some),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(cause) => Err(cause.into()),
    }
}
pub(super) fn token(value: &Option<BaselineIdentity>) -> String { format!("{:x}", Sha256::digest(serde_json::to_vec(value).expect("file identity"))) }
impl ExternalChange {
    pub(super) fn verify(&self, path: &std::path::Path) -> TextDocumentResult<()> {
        if path != self.path || token(&identity(path)?) != self.token {
            return Err(error("external-change", "文件再次发生变化，请重新选择处理方式；当前编辑仍然保留"));
        }
        Ok(())
    }
}
impl MarkdownDocumentSessionManager {
    pub(crate) fn external_change(&self, owner: &str, document_id: &str) -> TextDocumentResult<Option<ExternalChange>> {
        let session = self.session(document_id)?;
        let (path, expected, revision) = { let session = lock(&session)?; session.owner(owner)?; (session.manifest.path.clone(), session.manifest.identity.clone(), session.manifest.revision) };
        let Some(path) = path else { return Ok(None); };
        let current = identity(&path)?;
        if current == expected { return Ok(None); }
        Ok(Some(ExternalChange { path, revision, deleted: current.is_none(), token: token(&current) }))
    }
    pub(crate) fn save_after_external(&self, owner: &str, snapshot_id: &str, change: &ExternalChange, snapshot_before_save: bool) -> TextDocumentResult<MarkdownStatus> {
        self.save_impl(owner, snapshot_id, Some(change.path.clone()), true, snapshot_before_save, Some(change))
    }
}
