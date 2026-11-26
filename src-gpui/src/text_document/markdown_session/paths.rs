use super::*;
use std::path::Path;

#[cfg(windows)]
const UNRECOGNIZED_VOLUME: i32 = windows_sys::Win32::Foundation::ERROR_UNRECOGNIZED_VOLUME as i32;

pub(super) struct DocumentPath {
    pub path: PathBuf,
    pub open_alias: Option<PathBuf>,
}

impl DocumentPath {
    fn new(path: PathBuf) -> Self {
        Self { path, open_alias: None }
    }
}

impl MarkdownDocumentSessionManager {
    pub(crate) fn resolve_document_path(&self, path: &Path) -> TextDocumentResult<PathBuf> {
        let resolved = self.existing_document_path(path)?;
        Ok(resolved.open_alias.unwrap_or(resolved.path))
    }

    fn existing_document_path(&self, path: &Path) -> TextDocumentResult<DocumentPath> {
        match fs::canonicalize(path) {
            Ok(path) => Ok(DocumentPath::new(path)),
            #[cfg(windows)]
            Err(cause) if cause.raw_os_error() == Some(UNRECOGNIZED_VOLUME) => self.virtual_volume_path(path),
            Err(cause) => Err(cause.into()),
        }
    }

    #[cfg(windows)]
    fn virtual_volume_path(&self, path: &Path) -> TextDocumentResult<DocumentPath> {
        let mut path = std::path::absolute(path)?;
        // 原子保存替换的是目录项。末级为符号链接时须先找到目标，避免用正文替换链接。
        for _ in 0..64 {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path)?;
                path.pop();
                path.push(target);
                match fs::canonicalize(&path) {
                    Ok(resolved) => return Ok(DocumentPath::new(resolved)),
                    Err(cause) if cause.raw_os_error() == Some(UNRECOGNIZED_VOLUME) => {}
                    Err(cause) => return Err(cause.into()),
                }
                continue;
            }
            let open_alias = if metadata.is_file() { self.open_virtual_alias(&path)? } else { None };
            return Ok(DocumentPath { path, open_alias });
        }
        Err(error("invalid-document-path", "文件链接层级过多，无法确定保存目标"))
    }

    #[cfg(windows)]
    fn open_virtual_alias(&self, path: &Path) -> TextDocumentResult<Option<PathBuf>> {
        let handles: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        if handles.is_empty() { return Ok(None); }
        let identity = BaselineIdentity::read(path)?;
        let Some(id) = identity.reliable_file_id() else { return Ok(None); };
        for handle in handles {
            let session = lock(&handle)?;
            let manifest = &session.manifest;
            if session.closed || manifest.identity.as_ref().and_then(BaselineIdentity::reliable_file_id) != Some(id) {
                continue;
            }
            let candidate = manifest.path.clone();
            drop(session);
            let Some(candidate) = candidate else { continue; };
            if candidate == path { break; }
            // 所记录的旧路径可能已被外部程序替换或删除。
            if BaselineIdentity::read(&candidate).is_ok_and(|other| other.reliable_file_id() == Some(id)) {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    pub(super) fn save_target_path(&self, target: &Path) -> TextDocumentResult<DocumentPath> {
        match fs::symlink_metadata(target) {
            Ok(_) => self.existing_document_path(target),
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                let name = target.file_name().ok_or_else(|| error("invalid-save-path", "保存路径缺少文件名"))?;
                let absolute = std::path::absolute(target)?;
                let parent = absolute.parent().ok_or_else(|| error("invalid-save-path", "保存路径缺少父目录"))?;
                Ok(DocumentPath::new(self.existing_document_path(parent)?.path.join(name)))
            }
            Err(cause) => Err(cause.into()),
        }
    }
}
