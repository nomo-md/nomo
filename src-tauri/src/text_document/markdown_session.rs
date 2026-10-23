//! Markdown 会话：完整源文本、历史及快照独立于 WebView 生命周期。
use super::{
    chunk_reader::{AddedStore, DocumentSnapshot, OriginalFile},
    edit_journal::BaselineIdentity,
    markdown_codec::{self, MarkdownSourceMap},
    markdown_history::{
        apply_tree_edits, extract_pieces, DiskHistory, HistoryRecord, MarkdownSelection, TreeEdit,
    },
    piece_tree::{Piece, PieceSource, PieceTree},
    TextDocumentError, TextDocumentResult,
};
use crate::{file_system::write_file_atomically_with, models::MarkdownEncoding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}
fn error(code: &'static str, message: impl Into<String>) -> TextDocumentError {
    TextDocumentError::new(code, message)
}
fn lock<T>(mutex: &Mutex<T>) -> TextDocumentResult<std::sync::MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| error("lock-poisoned", "Markdown 会话锁已损坏"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownEdit {
    pub(crate) from_byte: u64,
    pub(crate) to_byte: u64,
    pub(crate) inserted_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownTransaction {
    #[serde(default)]
    pub(crate) derived: bool,
    pub(crate) document_id: String,
    pub(crate) base_revision: u64,
    pub(crate) operation_id: String,
    pub(crate) history_group_id: String,
    pub(crate) edits: Vec<MarkdownEdit>,
    pub(crate) selection_before: Option<MarkdownSelection>,
    pub(crate) selection_after: Option<MarkdownSelection>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownStatus {
    pub(crate) has_derived_edits: bool,
    pub(crate) document_id: String,
    pub(crate) revision: u64,
    pub(crate) byte_length: u64,
    pub(crate) dirty: bool,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    pub(crate) selection: Option<MarkdownSelection>,
    pub(crate) encoding: MarkdownEncoding,
    pub(crate) path: Option<String>,
    pub(crate) persistence_warning: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownWindow {
    pub(crate) document_id: String,
    pub(crate) revision: u64,
    pub(crate) from_byte: u64,
    pub(crate) to_byte: u64,
    pub(crate) text: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotHandle {
    pub(crate) snapshot_id: String,
    pub(crate) document_id: String,
    pub(crate) revision: u64,
    pub(crate) byte_length: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    derived: bool,
    version: u32,
    document_id: String,
    path: Option<PathBuf>,
    identity: Option<BaselineIdentity>,
    source_map: MarkdownSourceMap,
    revision: u64,
    event: Option<u64>,
    undo: Option<u64>,
    redo: Option<u64>,
    selection: Option<MarkdownSelection>,
    saved_tree: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct PendingSave {
    path: PathBuf,
    saved_tree: String,
    digest: [u8; 32],
}
struct Session {
    root: PathBuf,
    owner: String,
    manifest: Manifest,
    current: DocumentSnapshot,
    saved: DocumentSnapshot,
    history: DiskHistory,
    dirty: bool,
    ack: HashMap<String, ([u8; 32], MarkdownStatus)>,
    ack_order: VecDeque<String>,
    closed: bool,
    save_gate: Arc<Mutex<()>>,
    active_group: Option<(String, String)>,
    persistence_warning: Option<String>,
    // 最后一个字段最后释放，此时正文句柄及历史文件均已关闭，Windows 也能删除目录。
    cleanup: SessionCleanup,
}
struct SessionCleanup {
    root: PathBuf,
    remove: bool,
}
impl Drop for SessionCleanup {
    fn drop(&mut self) {
        if self.remove {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                crate::app_logger::warn(
                    "MarkdownDocument",
                    &format!("清理已关闭会话失败：{error}"),
                );
                // 存储已提交成功；短暂的文件占用只重试清理，不能变成保存失败。
                let root = self.root.clone();
                std::thread::spawn(move || {
                    for delay in [100, 500, 2000] {
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                        match fs::remove_dir_all(&root) {
                            Ok(()) => return,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                            Err(error) => crate::app_logger::warn("MarkdownDocument", &format!("重试清理已关闭会话失败：{error}")),
                        }
                    }
                });
            }
        }
    }
}
impl Session {
    fn status(&self) -> MarkdownStatus {
        MarkdownStatus {
            has_derived_edits: self.manifest.derived,
            document_id: self.manifest.document_id.clone(),
            revision: self.manifest.revision,
            byte_length: self.current.len(),
            dirty: self.dirty,
            can_undo: self.manifest.undo.is_some(),
            can_redo: self.manifest.redo.is_some(),
            selection: self.manifest.selection.clone(),
            encoding: self.manifest.source_map.encoding,
            path: self
                .manifest
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            persistence_warning: self.persistence_warning.clone(),
        }
    }
    fn owner(&self, owner: &str) -> TextDocumentResult<()> {
        if self.closed {
            return Err(error("document-closed", "文档会话已关闭"));
        }
        if self.owner != owner {
            return Err(error("document-owner-conflict", "文档已移交给另一个窗口"));
        }
        Ok(())
    }
    fn persist(&self, manifest: &Manifest) -> TextDocumentResult<()> {
        let bytes =
            serde_json::to_vec(manifest).map_err(|e| error("manifest-serialize", e.to_string()))?;
        crate::file_system::write_file_atomically(&self.root.join("manifest.json"), &bytes)
            .map_err(|e| error("manifest-write", e))
    }
    fn publish(
        &mut self,
        record: HistoryRecord,
        tree: PieceTree,
        undo: HistoryPointer,
        redo: HistoryPointer,
    ) -> TextDocumentResult<MarkdownStatus> {
        let mut candidate = self.current.clone();
        candidate.tree = tree.clone();
        let dirty = !snapshots_equal(&candidate, &self.saved)?;
        self.current.added.sync()?;
        let offset = self.history.append(&record)?;
        let mut manifest = self.manifest.clone();
        manifest.revision += 1;
        manifest.event = Some(offset);
        manifest.undo = undo.resolve(offset);
        manifest.redo = redo.resolve(offset);
        manifest.selection = record.selection_after;
        // 游标持久化成功才确认事务。任何失败均保留旧树、旧历史以及可重试输入。
        self.persist(&manifest)?;
        self.current.tree = tree;
        self.current.revision = manifest.revision;
        self.manifest = manifest;
        self.dirty = dirty;
        Ok(self.status())
    }
}
enum HistoryPointer {
    This,
    Existing(Option<u64>),
}
impl HistoryPointer {
    fn resolve(self, offset: u64) -> Option<u64> {
        match self {
            Self::This => Some(offset),
            Self::Existing(value) => value,
        }
    }
}

#[derive(Clone)]
struct CapturedSnapshot {
    document_id: String,
    content: DocumentSnapshot,
    root: PathBuf,
    source_map: MarkdownSourceMap,
}
#[derive(Clone)]
pub(crate) struct MarkdownDocumentSessionManager {
    root: PathBuf,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>,
    snapshots: Arc<Mutex<HashMap<String, CapturedSnapshot>>>,
    path_operations: Arc<Mutex<()>>,
    writing_stats: super::writing_stats::WritingStatsService,
}
impl MarkdownDocumentSessionManager {
    pub(crate) fn new(root: PathBuf) -> TextDocumentResult<Self> {
        if let Err(error) = super::writing_stats::WritingStatsService::clean_stale(&root) {
            crate::app_logger::warn("WritingStats", &format!("清理遗留统计文件失败：{}", error.message));
        }
        Ok(Self {
            root,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            snapshots: Arc::new(Mutex::new(HashMap::new())),
            path_operations: Arc::new(Mutex::new(())),
            writing_stats: Default::default(),
        })
    }
    fn session(&self, document_id: &str) -> TextDocumentResult<Arc<Mutex<Session>>> {
        lock(&self.sessions)?
            .get(document_id)
            .cloned()
            .ok_or_else(|| error("document-closed", "Markdown 会话不存在或已关闭"))
    }
    pub(crate) fn open(
        &self,
        owner: &str,
        path: Option<PathBuf>,
        text: Option<String>,
    ) -> TextDocumentResult<MarkdownStatus> {
        self.open_with_policy(owner, path, text, false)
    }
    /// 重新加载先捕获独立的磁盘基线，替换成功后再释放旧会话；仍校验同路径的窗口归属。
    pub(crate) fn open_with_policy(
        &self,
        owner: &str,
        path: Option<PathBuf>,
        text: Option<String>,
        fresh: bool,
    ) -> TextDocumentResult<MarkdownStatus> {
        let _path_operation = lock(&self.path_operations)?;
        if path.is_some() == text.is_some() {
            return Err(error(
                "invalid-open",
                "打开文档必须且只能指定路径或初始正文",
            ));
        }
        let path = path.map(fs::canonicalize).transpose()?;
        if let Some(path) = &path {
            let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
            for session in sessions {
                let session = lock(&session)?;
                if session.manifest.path.as_ref() == Some(path) {
                    session.owner(owner)?;
                    if !fresh {
                        return Ok(session.status());
                    }
                }
            }
        }
        let document_id = id("md");
        let root = self.root.join(&document_id);
        // 注册尚未使用的服务不创建磁盘缓存，也不把缓存目录故障变成应用启动故障。
        fs::create_dir_all(&self.root)?;
        fs::create_dir(&root)?;
        let mut unpublished = SessionCleanup {
            root: root.clone(),
            remove: true,
        };
        let raw_path = root.join("raw");
        let identity = if let Some(path) = &path {
            let mut source = File::open(path)?;
            let before = BaselineIdentity::read_file(path, &source)?;
            let mut output = File::create(&raw_path)?;
            std::io::copy(&mut source, &mut output)?;
            output.sync_all()?;
            if before != BaselineIdentity::read_file(path, &source)?
                || before != BaselineIdentity::read(path)?
            {
                return Err(error(
                    "source-changed",
                    "文件在读取期间发生变化，请重新打开",
                ));
            }
            Some(before)
        } else {
            let mut output = File::create(&raw_path)?;
            output.write_all(text.unwrap_or_default().as_bytes())?;
            output.sync_all()?;
            None
        };
        let map = if path.is_some() {
            markdown_codec::prepare_working_source(&raw_path, &root.join("working"))?
        } else {
            markdown_codec::convert(&raw_path, &root.join("working"), MarkdownEncoding::Utf8)?
        };
        let converted_path = root.join("working");
        let (working_path, body_offset) = map.working_source(&raw_path, &converted_path);
        let original = Arc::new(OriginalFile::open(working_path, body_offset)?);
        let added = Arc::new(AddedStore::open_persistent(root.join("added"))?);
        let tree = PieceTree::from_original(map.utf8_len);
        let content = DocumentSnapshot {
            revision: 0,
            tree: tree.clone(),
            original,
            added,
        };
        let manifest = Manifest {
            derived: false,
            version: 1,
            document_id: document_id.clone(),
            path,
            identity,
            source_map: map,
            revision: 0,
            event: None,
            undo: None,
            redo: None,
            selection: None,
            saved_tree: None,
        };
        let session = Session {
            history: DiskHistory::open(&root.join("history"))?,
            cleanup: SessionCleanup {
                root: root.clone(),
                remove: false,
            },
            root,
            owner: owner.into(),
            manifest,
            current: content.clone(),
            saved: content,
            dirty: false,
            ack: HashMap::new(),
            ack_order: VecDeque::new(),
            closed: false,
            save_gate: Arc::new(Mutex::new(())),
            active_group: None,
            persistence_warning: None,
        };
        session.persist(&session.manifest)?;
        let status = session.status();
        lock(&self.sessions)?.insert(document_id, Arc::new(Mutex::new(session)));
        unpublished.remove = false;
        Ok(status)
    }
    pub(crate) fn recover(
        &self,
        owner: &str,
        document_id: &str,
    ) -> TextDocumentResult<MarkdownStatus> {
        let _path_operation = lock(&self.path_operations)?;
        if !document_id.starts_with("md-")
            || !document_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(error("invalid-recovery", "恢复文档身份无效"));
        }
        if let Ok(session) = self.session(document_id) {
            let session = lock(&session)?;
            session.owner(owner)?;
            return Ok(session.status());
        }
        let root = self.root.join(document_id);
        match fs::metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(error("recovery-missing", "文档恢复资料已不存在"));
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let mut manifest: Manifest =
            serde_json::from_reader(File::open(root.join("manifest.json"))?)
                .map_err(|e| error("recovery-corrupt", e.to_string()))?;
        if manifest.version != 1 || manifest.document_id != document_id {
            return Err(error("recovery-version", "恢复资料版本不匹配"));
        }
        // 目标文件和私有保存点不能跨两个文件原子提交；持久化意图负责弥合崩溃窗口。
        let pending_path = root.join("pending-save.json");
        if pending_path.exists() {
            let pending: PendingSave = serde_json::from_reader(File::open(&pending_path)?)
                .map_err(|e| error("save-recovery-corrupt", e.to_string()))?;
            if !valid_saved_tree_name(&pending.saved_tree) {
                return Err(error("save-recovery-corrupt", "保存意图的片段引用无效"));
            }
            if digest_file(&pending.path).ok() == Some(pending.digest) {
                manifest.identity = Some(BaselineIdentity::read(&pending.path)?);
                manifest.path = Some(pending.path);
                manifest.saved_tree = Some(pending.saved_tree);
            }
        }
        let raw_path = root.join("raw");
        let converted_path = root.join("working");
        let (working_path, body_offset) = manifest
            .source_map
            .working_source(&raw_path, &converted_path);
        let original = Arc::new(OriginalFile::open(working_path, body_offset)?);
        let added = Arc::new(AddedStore::open_persistent(root.join("added"))?);
        let mut history = DiskHistory::open(&root.join("history"))?;
        let tree = history.replay(
            PieceTree::from_original(manifest.source_map.utf8_len),
            manifest.event,
        )?;
        let saved_tree = if let Some(name) = &manifest.saved_tree {
            if !valid_saved_tree_name(name) {
                return Err(error("recovery-corrupt", "保存点引用无效"));
            }
            let pieces: Vec<Piece> = serde_json::from_reader(File::open(root.join(name))?)
                .map_err(|e| error("recovery-corrupt", e.to_string()))?;
            apply_tree_edits(
                PieceTree::from_original(0),
                &[TreeEdit {
                    from: 0,
                    to: 0,
                    pieces,
                }],
            )?
        } else {
            PieceTree::from_original(manifest.source_map.utf8_len)
        };
        let current = DocumentSnapshot {
            revision: manifest.revision,
            tree,
            original: original.clone(),
            added: added.clone(),
        };
        let saved = DocumentSnapshot {
            revision: 0,
            tree: saved_tree,
            original,
            added,
        };
        markdown_codec::write_encoded(
            current.reader(),
            &mut std::io::sink(),
            MarkdownEncoding::Utf8,
        )?;
        File::open(root.join("raw"))?;
        let dirty = !snapshots_equal(&current, &saved)?;
        // 正文恢复不承诺重启后撤销；日志仍保留供正文 replay，新的历史从当前正文开始。
        manifest.undo = None;
        manifest.redo = None;
        let session = Session {
            cleanup: SessionCleanup {
                root: root.clone(),
                remove: false,
            },
            root,
            owner: owner.into(),
            manifest,
            current,
            saved,
            history,
            dirty,
            ack: HashMap::new(),
            ack_order: VecDeque::new(),
            closed: false,
            save_gate: Arc::new(Mutex::new(())),
            active_group: None,
            persistence_warning: None,
        };
        session.persist(&session.manifest)?;
        if pending_path.exists() {
            fs::remove_file(pending_path)?;
        }
        let status = session.status();
        lock(&self.sessions)?.insert(document_id.into(), Arc::new(Mutex::new(session)));
        Ok(status)
    }
    pub(crate) fn status(
        &self,
        owner: &str,
        document_id: &str,
    ) -> TextDocumentResult<MarkdownStatus> {
        let session = self.session(document_id)?;
        let session = lock(&session)?;
        session.owner(owner)?;
        Ok(session.status())
    }
    pub(crate) async fn writing_stats(
        &self, owner: String, query: super::writing_stats::StatsQuery,
    ) -> TextDocumentResult<super::writing_stats::StatsResult> {
        let handle = self.session(&query.document_id)?;
        let (snapshot, root) = {
            let session = lock(&handle)?;
            session.owner(&owner)?;
            if session.current.revision != query.revision {
                return Err(TextDocumentError::revision_conflict(query.revision, session.current.revision));
            }
            (session.current.clone(), session.root.clone())
        };
        let document_id = query.document_id.clone();
        let revision = query.revision;
        let result = self.writing_stats.request(owner.clone(), query, snapshot, root, Box::new(handle)).await?;
        let current = self.status(&owner, &document_id)?;
        if current.revision != revision { return Err(TextDocumentError::revision_conflict(revision, current.revision)); }
        Ok(result)
    }
    pub(crate) fn cancel_writing_stats(&self, owner: &str, request_id: Option<u64>) {
        self.writing_stats.cancel(owner, request_id);
    }
    pub(crate) fn read_window(
        &self,
        owner: &str,
        document_id: &str,
        revision: u64,
        from: u64,
        to: u64,
    ) -> TextDocumentResult<MarkdownWindow> {
        let session = self.session(document_id)?;
        let session = lock(&session)?;
        session.owner(owner)?;
        if revision != session.current.revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                session.current.revision,
            ));
        }
        read_window(document_id, &session.current, from, to)
    }
    pub(crate) fn apply(
        &self,
        owner: &str,
        transaction: MarkdownTransaction,
    ) -> TextDocumentResult<MarkdownStatus> {
        let handle = self.session(&transaction.document_id)?;
        let mut session = lock(&handle)?;
        session.owner(owner)?;
        if transaction.operation_id.is_empty() || transaction.history_group_id.is_empty() {
            return Err(error("invalid-transaction", "事务缺少操作身份或历史分组"));
        }
        let fingerprint: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&transaction)
                .map_err(|e| error("invalid-transaction", e.to_string()))?,
        )
        .into();
        if let Some((old, result)) = session.ack.get(&transaction.operation_id) {
            if old != &fingerprint {
                return Err(error("operation-conflict", "同一操作身份携带了不同内容"));
            }
            return Ok(result.clone());
        }
        if transaction.base_revision != session.current.revision {
            return Err(TextDocumentError::revision_conflict(
                transaction.base_revision,
                session.current.revision,
            ));
        }
        let mut edits = transaction.edits;
        edits.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
        for (index, edit) in edits.iter().enumerate() {
            if edit.from_byte > edit.to_byte
                || !boundary(&session.current, edit.from_byte)?
                || !boundary(&session.current, edit.to_byte)?
                || (index > 0
                    && (edit.from_byte < edits[index - 1].to_byte
                        || edit.from_byte == edits[index - 1].from_byte))
            {
                return Err(error(
                    "invalid-edit-range",
                    "事务范围重叠、越界或切开了 Unicode/CRLF",
                ));
            }
        }
        validate_selection(&session.current, transaction.selection_before.as_ref())?;
        let mut forward = Vec::new();
        let mut inverse = Vec::new();
        for edit in edits.iter().rev() {
            if edit.to_byte - edit.from_byte == edit.inserted_text.len() as u64
                && session
                    .current
                    .read_range(edit.from_byte, edit.inserted_text.len())?
                    == edit.inserted_text.as_bytes()
            {
                continue;
            }
            let old_pieces = extract_pieces(&session.current.tree, edit.from_byte, edit.to_byte)?;
            let bytes = edit.inserted_text.as_bytes();
            let pieces = if bytes.is_empty() {
                vec![]
            } else {
                vec![Piece::added(
                    session.current.added.append(bytes)?,
                    bytes.len() as u64,
                    bytes.iter().filter(|b| **b == b'\n').count() as u64,
                )]
            };
            forward.push(TreeEdit {
                from: edit.from_byte,
                to: edit.to_byte,
                pieces,
            });
            inverse.insert(
                0,
                TreeEdit {
                    from: edit.from_byte,
                    to: edit.from_byte + bytes.len() as u64,
                    pieces: old_pieces,
                },
            );
        }
        let tree = apply_tree_edits(session.current.tree.clone(), &forward)?;
        let mut candidate = session.current.clone();
        candidate.tree = tree.clone();
        validate_selection(&candidate, transaction.selection_after.as_ref())?;
        let group = session
            .active_group
            .as_ref()
            .filter(|(requested, _)| requested == &transaction.history_group_id)
            .map(|(_, group)| group.clone())
            .unwrap_or_else(|| id("group"));
        let record = HistoryRecord {
            previous_event: session.manifest.event,
            group: group.clone(),
            parent_undo: session.manifest.undo,
            previous_redo: session.manifest.redo,
            edits: forward,
            inverse,
            group_entries: vec![],
            selection_before: transaction.selection_before,
            selection_after: transaction.selection_after,
        };
        let result = if record.edits.is_empty() {
            session.manifest.selection = record.selection_after;
            session.status()
        } else {
            let undo = session.manifest.undo;
            let redo = session.manifest.redo;
            let previous_derived = session.manifest.derived;
            session.manifest.derived |= transaction.derived;
            let result = session.publish(
                record,
                tree,
                if transaction.derived {
                    HistoryPointer::Existing(undo)
                } else {
                    HistoryPointer::This
                },
                HistoryPointer::Existing(if transaction.derived { redo } else { None }),
            );
            if result.is_err() {
                session.manifest.derived = previous_derived;
            }
            let result = result?;
            if !transaction.derived {
                session.active_group = Some((transaction.history_group_id, group));
            }
            result
        };
        session.ack.insert(
            transaction.operation_id.clone(),
            (fingerprint, result.clone()),
        );
        if result.revision != transaction.base_revision {
            self.writing_stats.invalidate(&transaction.document_id, false);
        }
        session.ack_order.push_back(transaction.operation_id);
        if session.ack_order.len() > 128 {
            if let Some(key) = session.ack_order.pop_front() {
                session.ack.remove(&key);
            }
        }
        Ok(result)
    }
    pub(crate) fn history(
        &self,
        owner: &str,
        document_id: &str,
        revision: u64,
        redo: bool,
    ) -> TextDocumentResult<MarkdownStatus> {
        let handle = self.session(document_id)?;
        let mut session = lock(&handle)?;
        session.owner(owner)?;
        if revision != session.current.revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                session.current.revision,
            ));
        }
        session.active_group = None;
        let mut patches = Vec::new();
        let (entries, undo, next_redo, selection) = if redo {
            let Some(offset) = session.manifest.redo else {
                return Ok(session.status());
            };
            let record = session.history.read(offset)?;
            let mut selection = session.manifest.selection.clone();
            for entry in record.group_entries.iter().rev() {
                let entry = session.history.read(*entry)?;
                patches.extend(entry.edits);
                selection = entry.selection_after;
            }
            (
                record.group_entries.clone(),
                record.group_entries.first().copied(),
                HistoryPointer::Existing(record.previous_redo),
                selection,
            )
        } else {
            let Some(mut offset) = session.manifest.undo else {
                return Ok(session.status());
            };
            let group = session.history.read(offset)?.group;
            let mut entries = Vec::new();
            let (parent, selection) = loop {
                let entry = session.history.read(offset)?;
                patches.extend(entry.inverse);
                entries.push(offset);
                let parent = entry.parent_undo;
                if let Some(previous) = parent {
                    if session.history.read(previous)?.group == group {
                        offset = previous;
                        continue;
                    }
                }
                break (parent, entry.selection_before);
            };
            (entries, parent, HistoryPointer::This, selection)
        };
        // 派生目录改变了旧补丁的坐标。历史仍指向原始用户事件，从不可变片段恢复该
        // 事件的完整树，再由前端物化它的目录；不把旧坐标套在新目录上，也不丢失 redo。
        let tree = if session.manifest.derived {
            let target = if redo {
                entries.first().copied()
            } else {
                match entries.last() {
                    Some(offset) => session.history.read(*offset)?.previous_event,
                    None => None,
                }
            };
            let base_length = session.manifest.source_map.utf8_len;
            let restored = session
                .history
                .replay(PieceTree::from_original(base_length), target)?;
            patches = vec![TreeEdit {
                from: 0,
                to: session.current.len(),
                pieces: extract_pieces(&restored, 0, restored.len())?,
            }];
            restored
        } else {
            apply_tree_edits(session.current.tree.clone(), &patches)?
        };
        let record = HistoryRecord {
            previous_event: session.manifest.event,
            group: if redo { "redo" } else { "undo" }.into(),
            parent_undo: session.manifest.undo,
            previous_redo: session.manifest.redo,
            edits: patches,
            inverse: vec![],
            group_entries: entries,
            selection_before: session.manifest.selection.clone(),
            selection_after: selection,
        };
        let result = session.publish(record, tree, HistoryPointer::Existing(undo), next_redo)?;
        self.writing_stats.invalidate(document_id, false);
        Ok(result)
    }
    pub(crate) fn capture(
        &self,
        owner: &str,
        document_id: &str,
    ) -> TextDocumentResult<SnapshotHandle> {
        self.capture_version(owner, document_id, false)
    }
    pub(crate) fn capture_original(
        &self,
        owner: &str,
        document_id: &str,
    ) -> TextDocumentResult<SnapshotHandle> {
        self.capture_version(owner, document_id, true)
    }
    fn capture_version(
        &self,
        owner: &str,
        document_id: &str,
        original: bool,
    ) -> TextDocumentResult<SnapshotHandle> {
        let session = self.session(document_id)?;
        let session = lock(&session)?;
        session.owner(owner)?;
        let mut content = session.current.clone();
        if original {
            content.tree = PieceTree::from_original(session.manifest.source_map.utf8_len);
            content.revision = 0;
        }
        let snapshot_id = id("snapshot");
        let handle = SnapshotHandle {
            snapshot_id: snapshot_id.clone(),
            document_id: document_id.into(),
            revision: content.revision,
            byte_length: content.len(),
        };
        lock(&self.snapshots)?.insert(
            snapshot_id,
            CapturedSnapshot {
                document_id: document_id.into(),
                content,
                root: session.root.clone(),
                source_map: session.manifest.source_map.clone(),
            },
        );
        Ok(handle)
    }
    pub(crate) fn read_snapshot(
        &self,
        owner: &str,
        snapshot_id: &str,
        from: u64,
        to: u64,
    ) -> TextDocumentResult<MarkdownWindow> {
        let snapshot = lock(&self.snapshots)?
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| error("snapshot-released", "文档快照已经释放"))?;
        let handle = self.session(&snapshot.document_id)?;
        let session = lock(&handle)?;
        session.owner(owner)?;
        let result = read_window(&snapshot.document_id, &snapshot.content, from, to);
        drop(snapshot);
        drop(session);
        result
    }
    pub(crate) fn release(&self, owner: &str, snapshot_id: &str) -> TextDocumentResult<()> {
        let document_id = lock(&self.snapshots)?
            .get(snapshot_id)
            .map(|snapshot| snapshot.document_id.clone());
        if let Some(document_id) = document_id {
            self.status(owner, &document_id)?;
        }
        lock(&self.snapshots)?.remove(snapshot_id);
        Ok(())
    }
    pub(crate) fn save(
        &self,
        owner: &str,
        snapshot_id: &str,
        target: PathBuf,
        overwrite_external: bool,
    ) -> TextDocumentResult<MarkdownStatus> {
        let _path_operation = lock(&self.path_operations)?;
        let target = if target.exists() {
            fs::canonicalize(target)?
        } else {
            let parent = target
                .parent()
                .ok_or_else(|| error("invalid-save-path", "保存路径缺少父目录"))?;
            let name = target
                .file_name()
                .ok_or_else(|| error("invalid-save-path", "保存路径缺少文件名"))?;
            fs::canonicalize(parent)?.join(name)
        };
        let snapshot = lock(&self.snapshots)?
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| error("snapshot-released", "文档快照已经释放"))?;
        let handle = self.session(&snapshot.document_id)?;
        let sessions: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        for other in sessions {
            let other = lock(&other)?;
            if other.manifest.document_id != snapshot.document_id
                && other.manifest.path.as_ref() == Some(&target)
            {
                return Err(error("target-in-use", "目标文件正在另一个文档会话中编辑"));
            }
        }
        let gate = lock(&handle)?.save_gate.clone();
        let _saving = lock(&gate)?;
        let expected = {
            let session = lock(&handle)?;
            session.owner(owner)?;
            if session.manifest.path.as_ref() == Some(&target) {
                session.manifest.identity.clone()
            } else {
                None
            }
        };
        let verify = || -> TextDocumentResult<()> {
            if let Some(expected) = &expected {
                if !overwrite_external
                    && BaselineIdentity::read(&target).ok().as_ref() != Some(expected)
                {
                    return Err(error(
                        "external-change",
                        "文件已被外部修改，当前编辑仍然保留",
                    ));
                }
            }
            Ok(())
        };
        verify()?;
        // 编码、保存点和意图均先成功落盘，再碰目标文件。写入失败保留原文和历史。
        let mut digest = DigestWriter(Sha256::new());
        write_snapshot(&snapshot, &mut digest)?;
        let saved_tree = id("saved");
        let pieces = extract_pieces(&snapshot.content.tree, 0, snapshot.content.len())?;
        let bytes =
            serde_json::to_vec(&pieces).map_err(|e| error("save-baseline", e.to_string()))?;
        crate::file_system::write_file_atomically(&snapshot.root.join(&saved_tree), &bytes)
            .map_err(|e| error("save-baseline", e))?;
        let pending = PendingSave {
            path: target.clone(),
            saved_tree: saved_tree.clone(),
            digest: digest.0.finalize().into(),
        };
        let pending_bytes =
            serde_json::to_vec(&pending).map_err(|e| error("save-intent", e.to_string()))?;
        crate::file_system::write_file_atomically(
            &snapshot.root.join("pending-save.json"),
            &pending_bytes,
        )
        .map_err(|e| error("save-intent", e))?;
        write_file_atomically_with(&target, |output| {
            write_snapshot(&snapshot, output).map_err(|e| e.to_string())?;
            verify().map_err(|e| e.to_string())
        })
        .map_err(|e| error("save-failed", e))?;
        let identity = BaselineIdentity::read(&target)?;
        let mut session = lock(&handle)?;
        session.owner(owner)?;
        let mut manifest = session.manifest.clone();
        manifest.path = Some(target);
        manifest.identity = Some(identity);
        let old_saved_tree = manifest.saved_tree.replace(saved_tree);
        let persisted = session.persist(&manifest);
        session.manifest = manifest;
        session.saved = snapshot.content;
        session.dirty = !snapshots_equal(&session.current, &session.saved)?;
        session.persistence_warning = match persisted {
            Ok(()) => fs::remove_file(session.root.join("pending-save.json"))
                .err()
                .map(|e| format!("文件已保存，保存意图清理待重试：{e}")),
            Err(e) => Some(format!("文件已完整保存，保存点将从持久化意图恢复：{e}")),
        };
        if session.persistence_warning.is_none() {
            if let Some(old) = old_saved_tree {
                if let Err(error) = fs::remove_file(session.root.join(old)) {
                    crate::app_logger::warn(
                        "MarkdownDocument",
                        &format!("清理已替换保存点失败：{error}"),
                    );
                }
            }
        }
        Ok(session.status())
    }
    pub(crate) fn transfer_owner(
        &self,
        document_id: &str,
        source: &str,
        target: &str,
    ) -> TextDocumentResult<()> {
        let handle = self.session(document_id)?;
        let gate = lock(&handle)?.save_gate.clone();
        let _saving = lock(&gate)?;
        let mut session = lock(&handle)?;
        session.owner(source)?;
        if lock(&self.snapshots)?
            .values()
            .any(|snapshot| snapshot.document_id == document_id)
        {
            return Err(error("document-busy", "请先完成文档的快照任务"));
        }
        session.owner = target.into();
        self.writing_stats.invalidate(document_id, true);
        Ok(())
    }
    /// 多文档归属和工作区提交共享同一临界区；持久化失败时全部仍归源窗口。
    pub(crate) fn transfer_owners_with<T>(
        &self,
        document_ids: &[String],
        source: &str,
        target: &str,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut ids = document_ids.to_vec();
        ids.sort();
        ids.dedup();
        let handles = ids
            .iter()
            .map(|id| self.session(id).map_err(|e| e.message))
            .collect::<Result<Vec<_>, _>>()?;
        let gates = handles
            .iter()
            .map(|handle| {
                lock(handle)
                    .map(|s| s.save_gate.clone())
                    .map_err(|e| e.message)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let _saving = gates
            .iter()
            .map(|gate| lock(gate).map_err(|e| e.message))
            .collect::<Result<Vec<_>, _>>()?;
        let mut sessions = handles
            .iter()
            .map(|handle| lock(handle).map_err(|e| e.message))
            .collect::<Result<Vec<_>, _>>()?;
        for session in &sessions {
            session.owner(source).map_err(|e| e.message)?;
        }
        if lock(&self.snapshots)
            .map_err(|e| e.message)?
            .values()
            .any(|snapshot| ids.contains(&snapshot.document_id))
        {
            return Err("请先完成文档快照任务再移交".into());
        }
        let result = commit()?;
        for session in &mut sessions {
            session.owner = target.into();
            self.writing_stats.invalidate(&session.manifest.document_id, true);
        }
        Ok(result)
    }
    /// 释放本窗口的内存映射，但保留恢复资料。已移交的会话由目标窗口继续持有。
    pub(crate) fn detach(&self, owner: &str, document_id: &str) -> TextDocumentResult<()> {
        let Some(handle) = lock(&self.sessions)?.get(document_id).cloned() else {
            return Ok(());
        };
        let gate = lock(&handle)?.save_gate.clone();
        let _saving = lock(&gate)?;
        {
            let mut session = lock(&handle)?;
            if session.owner != owner {
                return Ok(());
            }
            if lock(&self.snapshots)?
                .values()
                .any(|snapshot| snapshot.document_id == document_id)
            {
                return Err(error("document-busy", "文档仍有正在使用的完整快照"));
            }
            session.closed = true;
            self.writing_stats.invalidate(document_id, true);
        }
        lock(&self.sessions)?.remove(document_id);
        Ok(())
    }
    /// WebView 已销毁，前端不再有机会释放快照。等保存结束后释放该窗口资源，保留磁盘恢复资料。
    pub(crate) fn detach_owner(&self, owner: &str) -> TextDocumentResult<()> {
        self.writing_stats.cancel(owner, None);
        let _path_operation = lock(&self.path_operations)?;
        let handles: Vec<_> = lock(&self.sessions)?.values().cloned().collect();
        for handle in handles {
            let gate = lock(&handle)?.save_gate.clone();
            let _saving = lock(&gate)?;
            let mut session = lock(&handle)?;
            // 交接提交与这里使用同一会话锁，销毁源窗口不会释放已经交给目标的正文。
            if session.owner != owner || session.closed {
                continue;
            }
            session.closed = true;
            let document_id = session.manifest.document_id.clone();
            self.writing_stats.invalidate(&document_id, true);
            lock(&self.snapshots)?.retain(|_, snapshot| snapshot.document_id != document_id);
            lock(&self.sessions)?.remove(&document_id);
        }
        Ok(())
    }
    /// 组合成员全部通过归属、快照和保存校验后才共同释放，避免部分关闭。
    pub(crate) fn close_many(&self, owner: &str, mut documents: Vec<(String, bool)>) -> TextDocumentResult<()> {
        documents.sort_by(|left, right| left.0.cmp(&right.0));
        if documents.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(error("invalid-close-group", "组合关闭包含重复文档"));
        }
        let _paths = lock(&self.path_operations)?;
        let handles = documents.iter().map(|(id, _)| self.session(id)).collect::<TextDocumentResult<Vec<_>>>()?;
        let gates = handles.iter().map(|handle| Ok(lock(handle)?.save_gate.clone())).collect::<TextDocumentResult<Vec<_>>>()?;
        let _saving = gates.iter().map(|gate| lock(gate)).collect::<TextDocumentResult<Vec<_>>>()?;
        let mut sessions = handles.iter().map(|handle| lock(handle)).collect::<TextDocumentResult<Vec<_>>>()?;
        {
            let snapshots = lock(&self.snapshots)?;
            for ((id, discard), session) in documents.iter().zip(sessions.iter()) {
                session.owner(owner)?;
                if snapshots.values().any(|snapshot| &snapshot.document_id == id) {
                    return Err(error("document-busy", "组合文档仍有正在使用的完整快照"));
                }
                if session.dirty && !discard { return Err(error("unsaved-document", "组合文档尚有未保存内容")); }
            }
        }
        for session in &mut sessions {
            session.closed = true; session.cleanup.remove = true;
            self.writing_stats.invalidate(&session.manifest.document_id, true);
        }
        let mut registry = lock(&self.sessions)?;
        for (id, _) in documents { registry.remove(&id); }
        drop(registry); drop(sessions);
        Ok(())
    }

    pub(crate) fn close(
        &self,
        owner: &str,
        document_id: &str,
        discard: bool,
    ) -> TextDocumentResult<()> {
        let session = self.session(document_id)?;
        let gate = lock(&session)?.save_gate.clone();
        let _saving = lock(&gate)?;
        {
            let mut session = lock(&session)?;
            session.owner(owner)?;
            if lock(&self.snapshots)?
                .values()
                .any(|snapshot| snapshot.document_id == document_id)
            {
                return Err(error("document-busy", "文档仍有正在使用的完整快照"));
            }
            if session.dirty && !discard {
                return Err(error("unsaved-document", "文档尚有未保存内容"));
            }
            session.closed = true;
            session.cleanup.remove = true;
            self.writing_stats.invalidate(document_id, true);
        }
        lock(&self.sessions)?.remove(document_id);
        drop(session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_group_keeps_every_member_when_a_snapshot_is_busy() {
        let (root, manager) = fixture();
        let first = manager.open("main", None, Some("first".into())).unwrap();
        let second = manager.open("main", None, Some("second".into())).unwrap();
        let snapshot = manager.capture("main", &second.document_id).unwrap();
        let group = vec![(first.document_id.clone(), true), (second.document_id.clone(), true)];
        assert!(manager.close_many("main", group.clone()).is_err());
        assert!(manager.status("main", &first.document_id).is_ok());
        assert!(manager.status("main", &second.document_id).is_ok());
        manager.release("main", &snapshot.snapshot_id).unwrap();
        manager.close_many("main", group).unwrap();
        assert!(manager.status("main", &first.document_id).is_err());
        assert!(manager.status("main", &second.document_id).is_err());
        drop(manager); fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn fresh_open_reads_the_new_baseline_and_survives_closing_the_old_session() {
        let (root, manager) = fixture();
        let path = root.join("document.md");
        fs::write(&path, "旧正文").unwrap();
        let old = manager.open("main", Some(path.clone()), None).unwrap();
        let snapshot = manager.capture("main", &old.document_id).unwrap();
        fs::write(&path, "磁盘新正文").unwrap();
        let fresh = manager
            .open_with_policy("main", Some(path.clone()), None, true)
            .unwrap();
        assert_ne!(fresh.document_id, old.document_id);
        assert_eq!(content(&manager, &old), "旧正文");
        assert_eq!(content(&manager, &fresh), "磁盘新正文");
        assert!(manager
            .open_with_policy("other", Some(path), None, true)
            .is_err());
        manager.release("main", &snapshot.snapshot_id).unwrap();
        manager.close("main", &old.document_id, true).unwrap();
        manager.detach("main", &fresh.document_id).unwrap();
        let recovered = manager.recover("main", &fresh.document_id).unwrap();
        assert_eq!(content(&manager, &recovered), "磁盘新正文");
        manager.close("main", &fresh.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn only_an_absent_recovery_directory_is_classified_as_missing() {
        let (root, manager) = fixture();
        assert_eq!(
            manager.recover("main", "md-missing").unwrap_err().code,
            "recovery-missing"
        );
        let damaged = manager.root.join("md-damaged");
        fs::create_dir_all(&damaged).unwrap();
        assert_eq!(
            manager.recover("main", "md-damaged").unwrap_err().code,
            "io-error"
        );
        fs::write(damaged.join("manifest.json"), "invalid").unwrap();
        assert_eq!(
            manager.recover("main", "md-damaged").unwrap_err().code,
            "recovery-corrupt"
        );
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn detaching_releases_memory_without_deleting_recovery_or_the_new_owner() {
        let (root, manager) = fixture();
        let initial = manager.open("main", None, Some("original".into())).unwrap();
        let edited = edit(&manager, &initial, "one", "one", 0, 0, "new ");
        manager.detach("main", &edited.document_id).unwrap();
        assert!(manager.status("main", &edited.document_id).is_err());
        let recovered = manager.recover("main", &edited.document_id).unwrap();
        assert_eq!(content(&manager, &recovered), "new original");
        assert!(recovered.dirty);
        manager
            .transfer_owner(&edited.document_id, "main", "target")
            .unwrap();
        manager.detach("main", &edited.document_id).unwrap();
        assert!(manager.status("target", &edited.document_id).is_ok());
        manager.close("target", &edited.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn destroyed_window_releases_snapshots_and_preserves_recovery_and_transferred_documents() {
        let (root, manager) = fixture();
        let initial = manager.open("main", None, Some("original".into())).unwrap();
        let edited = edit(&manager, &initial, "one", "one", 0, 0, "new ");
        let orphaned = manager.capture("main", &edited.document_id).unwrap();
        let moved = manager.open("main", None, Some("target".into())).unwrap();
        manager
            .transfer_owner(&moved.document_id, "main", "target")
            .unwrap();
        let retained = manager.capture("target", &moved.document_id).unwrap();

        manager.detach_owner("main").unwrap();
        assert!(manager
            .read_snapshot("main", &orphaned.snapshot_id, 0, orphaned.byte_length)
            .is_err());
        assert_eq!(
            manager
                .read_snapshot("target", &retained.snapshot_id, 0, retained.byte_length)
                .unwrap()
                .text,
            "target"
        );
        let restored = manager.recover("replacement", &edited.document_id).unwrap();
        assert!(restored.dirty);
        assert_eq!(
            manager
                .read_window(
                    "replacement",
                    &restored.document_id,
                    restored.revision,
                    0,
                    restored.byte_length
                )
                .unwrap()
                .text,
            "new original"
        );
        manager
            .close("replacement", &restored.document_id, true)
            .unwrap();
        manager.release("target", &retained.snapshot_id).unwrap();
        manager.close("target", &moved.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn derived_edits_do_not_add_history_or_break_later_undo_redo() {
        let (root, manager) = fixture();
        let initial = manager
            .open("main", None, Some("toc\n# A\nbody".into()))
            .unwrap();
        let title = edit(&manager, &initial, "title", "title", 6, 7, "B");
        let derived = manager
            .apply(
                "main",
                MarkdownTransaction {
                    derived: true,
                    document_id: title.document_id.clone(),
                    base_revision: title.revision,
                    operation_id: "toc".into(),
                    history_group_id: "toc".into(),
                    edits: vec![MarkdownEdit {
                        from_byte: 0,
                        to_byte: 3,
                        inserted_text: "long toc".into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap();
        let body = edit(
            &manager,
            &derived,
            "body",
            "body",
            derived.byte_length,
            derived.byte_length,
            "!",
        );
        let undo_body = manager
            .history("main", &body.document_id, body.revision, false)
            .unwrap();
        assert_eq!(content(&manager, &undo_body), "long toc\n# B\nbody");
        let undo_title = manager
            .history("main", &body.document_id, undo_body.revision, false)
            .unwrap();
        assert_eq!(content(&manager, &undo_title), "toc\n# A\nbody");
        assert!(!undo_title.dirty);
        let redo_title = manager
            .history("main", &body.document_id, undo_title.revision, true)
            .unwrap();
        assert_eq!(content(&manager, &redo_title), "toc\n# B\nbody");
        let redo_body = manager
            .history("main", &body.document_id, redo_title.revision, true)
            .unwrap();
        assert_eq!(content(&manager, &redo_body), "long toc\n# B\nbody!");
        manager.close("main", &body.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_handoff_keeps_all_owners_on_failure_and_preserves_history_on_commit() {
        let (root, manager) = fixture();
        let first = manager.open("main", None, Some("one".into())).unwrap();
        let first = edit(&manager, &first, "one", "one", 0, 0, "new ");
        let second = manager.open("main", None, Some("two".into())).unwrap();
        let ids = vec![first.document_id.clone(), second.document_id.clone()];
        assert!(manager
            .transfer_owners_with::<()>(&ids, "main", "target", || Err("persistence failed".into()))
            .is_err());
        assert!(manager.status("main", &first.document_id).is_ok());
        assert!(manager.status("main", &second.document_id).is_ok());
        let snapshot = manager.capture("main", &first.document_id).unwrap();
        let mut committed = false;
        assert!(manager
            .transfer_owners_with(&ids, "main", "target", || {
                committed = true;
                Ok(())
            })
            .is_err());
        assert!(!committed);
        manager.release("main", &snapshot.snapshot_id).unwrap();
        manager
            .transfer_owners_with(&ids, "main", "target", || Ok(()))
            .unwrap();
        assert!(manager.status("main", &first.document_id).is_err());
        let undone = manager
            .history("target", &first.document_id, first.revision, false)
            .unwrap();
        assert!(!undone.dirty);
        assert!(manager.status("target", &second.document_id).is_ok());
        for id in ids {
            manager.close("target", &id, true).unwrap();
        }
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovers_saved_baseline_if_manifest_publication_fails_after_atomic_replace() {
        let (root, manager) = fixture();
        let status = manager.open("main", None, Some("原文".into())).unwrap();
        let changed = edit(&manager, &status, "one", "one", 0, 0, "前");
        let snapshot = manager.capture("main", &status.document_id).unwrap();
        let current = edit(
            &manager,
            &changed,
            "two",
            "two",
            changed.byte_length,
            changed.byte_length,
            "后",
        );
        let session_root = root.join("sessions").join(&status.document_id);
        let manifest_path = session_root.join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).unwrap();
        fs::remove_file(&manifest_path).unwrap();
        fs::create_dir(&manifest_path).unwrap();
        let target = root.join("saved.md");
        let saved = manager
            .save("main", &snapshot.snapshot_id, target.clone(), false)
            .unwrap();
        assert!(saved.persistence_warning.is_some());
        assert!(saved.dirty);
        assert_eq!(fs::read_to_string(&target).unwrap(), "前原文");
        manager.release("main", &snapshot.snapshot_id).unwrap();
        fs::remove_dir(&manifest_path).unwrap();
        fs::write(&manifest_path, manifest_bytes).unwrap();
        drop(manager);
        let restored = MarkdownDocumentSessionManager::new(root.join("sessions")).unwrap();
        let recovered = restored.recover("main", &status.document_id).unwrap();
        assert!(recovered.dirty);
        assert_eq!(recovered.revision, current.revision);
        assert_eq!(
            restored
                .read_window(
                    "main",
                    &status.document_id,
                    recovered.revision,
                    0,
                    recovered.byte_length
                )
                .unwrap()
                .text,
            "前原文后"
        );
        assert!(!session_root.join("pending-save.json").exists());
        restored.close("main", &status.document_id, true).unwrap();
        drop(restored);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn new_unicode_text_keeps_a_leading_bom_character_as_content() {
        let (root, manager) = fixture();
        let status = manager
            .open("main", None, Some("\u{feff}正文".into()))
            .unwrap();
        assert_eq!(status.encoding, MarkdownEncoding::Utf8);
        assert_eq!(
            manager
                .read_window("main", &status.document_id, 0, 0, status.byte_length)
                .unwrap()
                .text,
            "\u{feff}正文"
        );
        manager.close("main", &status.document_id, false).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    fn fixture() -> (PathBuf, MarkdownDocumentSessionManager) {
        let root = std::env::temp_dir().join(id("nomo-markdown"));
        fs::create_dir_all(&root).unwrap();
        let manager = MarkdownDocumentSessionManager::new(root.join("sessions")).unwrap();
        assert!(!root.join("sessions").exists());
        (root, manager)
    }
    fn edit(
        manager: &MarkdownDocumentSessionManager,
        status: &MarkdownStatus,
        operation: &str,
        group: &str,
        from: u64,
        to: u64,
        text: &str,
    ) -> MarkdownStatus {
        manager
            .apply(
                "main",
                MarkdownTransaction {
                    derived: false,
                    document_id: status.document_id.clone(),
                    base_revision: status.revision,
                    operation_id: operation.into(),
                    history_group_id: group.into(),
                    edits: vec![MarkdownEdit {
                        from_byte: from,
                        to_byte: to,
                        inserted_text: text.into(),
                    }],
                    selection_before: None,
                    selection_after: None,
                },
            )
            .unwrap()
    }
    fn content(manager: &MarkdownDocumentSessionManager, status: &MarkdownStatus) -> String {
        manager
            .read_window(
                "main",
                &status.document_id,
                status.revision,
                0,
                status.byte_length,
            )
            .unwrap()
            .text
    }
    #[test]
    fn snapshot_save_preserves_tail_and_later_input_and_recovers_after_restart() {
        let (root, manager) = fixture();
        let path = root.join("document.md");
        let original = "# 中文\r\n\r\n尾部  \n";
        fs::write(&path, original).unwrap();
        let opened = manager.open("main", Some(path.clone()), None).unwrap();
        let changed = edit(&manager, &opened, "1", "1", 2, 2, "新");
        let snapshot = manager.capture("main", &changed.document_id).unwrap();
        let later = edit(
            &manager,
            &changed,
            "2",
            "2",
            changed.byte_length,
            changed.byte_length,
            "later",
        );
        let saved = manager
            .save("main", &snapshot.snapshot_id, path.clone(), false)
            .unwrap();
        assert!(saved.dirty);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# 新中文\r\n\r\n尾部  \n"
        );
        manager.release("main", &snapshot.snapshot_id).unwrap();
        drop(manager);
        let manager = MarkdownDocumentSessionManager::new(root.join("sessions")).unwrap();
        let recovered = manager.recover("main", &later.document_id).unwrap();
        assert_eq!(
            content(&manager, &recovered),
            "# 新中文\r\n\r\n尾部  \nlater"
        );
        assert!(recovered.dirty);
        assert!(!recovered.can_undo);
        manager.close("main", &recovered.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn history_groups_survive_transfer_and_reject_old_window_writes() {
        let (root, manager) = fixture();
        let opened = manager.open("main", None, Some("a中\r\nb".into())).unwrap();
        let first = edit(&manager, &opened, "1", "composition", 1, 4, "中文");
        let second = edit(&manager, &first, "2", "composition", 1, 7, "输入");
        manager
            .transfer_owner(&opened.document_id, "main", "second")
            .unwrap();
        assert!(manager
            .history("main", &opened.document_id, second.revision, false)
            .is_err());
        let undone = manager
            .history("second", &opened.document_id, second.revision, false)
            .unwrap();
        assert!(!undone.dirty);
        assert!(!undone.can_undo);
        let redone = manager
            .history("second", &opened.document_id, undone.revision, true)
            .unwrap();
        assert_eq!(
            manager
                .read_window(
                    "second",
                    &opened.document_id,
                    redone.revision,
                    0,
                    redone.byte_length
                )
                .unwrap()
                .text,
            "a输入\r\nb"
        );
        manager.close("second", &opened.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn gbk_keeps_original_bytes_and_rejects_lossy_save_without_discarding_edits() {
        let (root, manager) = fixture();
        let path = root.join("gbk.md");
        let original = b"# \xd6\xd0\xce\xc4\r\n\r\n";
        fs::write(&path, original).unwrap();
        let opened = manager.open("main", Some(path.clone()), None).unwrap();
        let changed = edit(&manager, &opened, "1", "1", 2, 2, "😀");
        let snapshot = manager.capture("main", &opened.document_id).unwrap();
        assert!(manager
            .save("main", &snapshot.snapshot_id, path.clone(), false)
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        manager.release("main", &snapshot.snapshot_id).unwrap();
        let undone = manager
            .history("main", &opened.document_id, changed.revision, false)
            .unwrap();
        assert!(!undone.dirty);
        let snapshot = manager.capture("main", &opened.document_id).unwrap();
        manager
            .save("main", &snapshot.snapshot_id, path.clone(), false)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        manager.release("main", &snapshot.snapshot_id).unwrap();
        manager.close("main", &opened.document_id, false).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn untouched_and_undone_sources_round_trip_all_encodings_byte_for_byte() {
        let (root, manager) = fixture();
        let text = "# 标题\r\n\r\n+ 正文  \n+ 未改\r\n\n[id]: <https://example.com> \n\n  ";
        for (index, encoding) in [
            MarkdownEncoding::Utf8,
            MarkdownEncoding::Utf8Bom,
            MarkdownEncoding::Utf16LeBom,
            MarkdownEncoding::Utf16BeBom,
            MarkdownEncoding::Gbk,
        ]
        .into_iter()
        .enumerate()
        {
            let mut original = markdown_codec::bom(encoding).to_vec();
            markdown_codec::write_encoded(text.as_bytes(), &mut original, encoding).unwrap();
            let path = root.join(format!("{index}.md"));
            fs::write(&path, &original).unwrap();
            let opened = manager.open("main", Some(path.clone()), None).unwrap();
            let working = root
                .join("sessions")
                .join(&opened.document_id)
                .join("working");
            assert_eq!(
                working.exists(),
                !matches!(encoding, MarkdownEncoding::Utf8 | MarkdownEncoding::Utf8Bom)
            );
            let snapshot = manager.capture("main", &opened.document_id).unwrap();
            manager
                .save("main", &snapshot.snapshot_id, path.clone(), false)
                .unwrap();
            manager.release("main", &snapshot.snapshot_id).unwrap();
            assert_eq!(fs::read(&path).unwrap(), original);
            let offset = text.find("正文").unwrap() as u64;
            let changed = edit(&manager, &opened, "edit", "typing", offset, offset, "新");
            let snapshot = manager.capture("main", &opened.document_id).unwrap();
            manager
                .save("main", &snapshot.snapshot_id, path.clone(), false)
                .unwrap();
            manager.release("main", &snapshot.snapshot_id).unwrap();
            let mut expected = markdown_codec::bom(encoding).to_vec();
            markdown_codec::write_encoded(
                text.replacen("正文", "新正文", 1).as_bytes(),
                &mut expected,
                encoding,
            )
            .unwrap();
            assert_eq!(fs::read(&path).unwrap(), expected);
            let undone = manager
                .history("main", &opened.document_id, changed.revision, false)
                .unwrap();
            assert!(undone.dirty);
            let snapshot = manager.capture("main", &opened.document_id).unwrap();
            let saved = manager
                .save("main", &snapshot.snapshot_id, path.clone(), false)
                .unwrap();
            assert!(!saved.dirty);
            assert_eq!(fs::read(&path).unwrap(), original);
            manager.release("main", &snapshot.snapshot_id).unwrap();
            manager.close("main", &opened.document_id, false).unwrap();
        }
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_publication_keeps_old_content_and_retry_does_not_replay_orphaned_history() {
        let (root, manager) = fixture();
        let opened = manager.open("main", None, Some("完整尾部".into())).unwrap();
        let manifest = root
            .join("sessions")
            .join(&opened.document_id)
            .join("manifest.json");
        let backup = manifest.with_extension("backup");
        fs::rename(&manifest, &backup).unwrap();
        fs::create_dir(&manifest).unwrap();
        let transaction = MarkdownTransaction {
            derived: false,
            document_id: opened.document_id.clone(),
            base_revision: 0,
            operation_id: "same-operation".into(),
            history_group_id: "typing".into(),
            edits: vec![MarkdownEdit {
                from_byte: 0,
                to_byte: 0,
                inserted_text: "新".into(),
            }],
            selection_before: None,
            selection_after: None,
        };
        assert!(manager.apply("main", transaction.clone()).is_err());
        assert_eq!(content(&manager, &opened), "完整尾部");
        fs::remove_dir(&manifest).unwrap();
        fs::rename(backup, manifest).unwrap();
        let changed = manager.apply("main", transaction.clone()).unwrap();
        assert_eq!(
            manager.apply("main", transaction).unwrap().revision,
            changed.revision
        );
        assert_eq!(content(&manager, &changed), "新完整尾部");
        drop(manager);
        let manager = MarkdownDocumentSessionManager::new(root.join("sessions")).unwrap();
        let recovered = manager.recover("main", &opened.document_id).unwrap();
        assert_eq!(content(&manager, &recovered), "新完整尾部");
        manager.close("main", &opened.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_external_overwrite_and_does_not_treat_noop_as_an_edit() {
        let (root, manager) = fixture();
        let path = root.join("external.md");
        fs::write(&path, "abc").unwrap();
        let opened = manager.open("main", Some(path.clone()), None).unwrap();
        let unchanged = edit(&manager, &opened, "noop", "group", 1, 2, "b");
        assert_eq!(unchanged.revision, opened.revision);
        assert!(!unchanged.can_undo);
        let changed = edit(&manager, &opened, "edit", "group", 1, 2, "中");
        fs::write(&path, "外部完整内容").unwrap();
        let snapshot = manager.capture("main", &opened.document_id).unwrap();
        assert_eq!(
            manager
                .save("main", &snapshot.snapshot_id, path.clone(), false)
                .unwrap_err()
                .code,
            "external-change"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "外部完整内容");
        assert_eq!(content(&manager, &changed), "a中c");
        manager.release("main", &snapshot.snapshot_id).unwrap();
        manager.close("main", &opened.document_id, true).unwrap();
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
}

fn boundary(snapshot: &DocumentSnapshot, offset: u64) -> TextDocumentResult<bool> {
    if offset > snapshot.len() {
        return Ok(false);
    }
    let current = snapshot.byte_at(offset)?;
    if current.is_some_and(|byte| byte & 0xc0 == 0x80) {
        return Ok(false);
    }
    Ok(!(offset > 0 && snapshot.byte_at(offset - 1)? == Some(b'\r') && current == Some(b'\n')))
}
fn validate_selection(
    snapshot: &DocumentSnapshot,
    selection: Option<&MarkdownSelection>,
) -> TextDocumentResult<()> {
    if let Some(selection) = selection {
        if !boundary(snapshot, selection.anchor_byte)? || !boundary(snapshot, selection.head_byte)?
        {
            return Err(error("invalid-selection", "选区不是完整字符边界"));
        }
    }
    Ok(())
}
fn read_window(
    document_id: &str,
    snapshot: &DocumentSnapshot,
    from: u64,
    to: u64,
) -> TextDocumentResult<MarkdownWindow> {
    if from > to || to > snapshot.len() || to - from > 1024 * 1024 {
        return Err(error("invalid-read-range", "正文窗口越界或超过 1 MiB"));
    }
    let mut from = from;
    let mut to = to;
    while from > 0 && !boundary(snapshot, from)? {
        from -= 1;
    }
    while to < snapshot.len() && !boundary(snapshot, to)? {
        to += 1;
    }
    let text = String::from_utf8(snapshot.read_range(from, (to - from) as usize)?)
        .map_err(|e| error("invalid-working-source", e.to_string()))?;
    Ok(MarkdownWindow {
        document_id: document_id.into(),
        revision: snapshot.revision,
        from_byte: from,
        to_byte: to,
        text,
    })
}
fn snapshots_equal(a: &DocumentSnapshot, b: &DocumentSnapshot) -> TextDocumentResult<bool> {
    if a.len() != b.len() {
        return Ok(false);
    }
    let mut offset = 0;
    while offset < a.len() {
        let (left, left_relative) = a
            .tree
            .piece_at(offset)
            .ok_or_else(|| error("invalid-piece-tree", "正文片段缺失"))?;
        let (right, right_relative) = b
            .tree
            .piece_at(offset)
            .ok_or_else(|| error("invalid-piece-tree", "保存点片段缺失"))?;
        let count = (left.len() - left_relative).min(right.len() - right_relative);
        let same_source = match (&left.source, &right.source) {
            (PieceSource::Original { offset: x, .. }, PieceSource::Original { offset: y, .. }) => {
                Arc::ptr_eq(&a.original, &b.original) && x + left_relative == y + right_relative
            }
            (PieceSource::Added { offset: x, .. }, PieceSource::Added { offset: y, .. }) => {
                Arc::ptr_eq(&a.added, &b.added) && x + left_relative == y + right_relative
            }
            _ => false,
        };
        if same_source {
            offset += count;
            continue;
        }
        let end = offset + count;
        while offset < end {
            let count = (end - offset).min(64 * 1024) as usize;
            if a.read_range(offset, count)? != b.read_range(offset, count)? {
                return Ok(false);
            }
            offset += count as u64;
        }
    }
    Ok(true)
}
fn valid_saved_tree_name(name: &str) -> bool {
    name.starts_with("saved-") && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
struct DigestWriter(Sha256);
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn digest_file(path: &std::path::Path) -> TextDocumentResult<[u8; 32]> {
    let mut source = File::open(path)?;
    let mut output = DigestWriter(Sha256::new());
    std::io::copy(&mut source, &mut output)?;
    Ok(output.0.finalize().into())
}
fn write_snapshot(snapshot: &CapturedSnapshot, output: &mut impl Write) -> TextDocumentResult<()> {
    output.write_all(markdown_codec::bom(snapshot.source_map.encoding))?;
    let raw_path = snapshot.root.join("raw");
    let mut raw = File::open(&raw_path)?;
    for (piece, relative, length) in snapshot.content.tree.segments(0, snapshot.content.len())? {
        match piece.source {
            PieceSource::Original { offset, .. } => {
                let start = snapshot
                    .source_map
                    .raw_offset(&raw_path, offset + relative)?;
                let end = snapshot
                    .source_map
                    .raw_offset(&raw_path, offset + relative + length)?;
                raw.seek(SeekFrom::Start(start))?;
                let copied = std::io::copy(&mut Read::by_ref(&mut raw).take(end - start), output)?;
                if copied != end - start {
                    return Err(error("unexpected-eof", "原始快照提前结束"));
                }
            }
            PieceSource::Added { offset, .. } => {
                let mut content = snapshot.content.clone();
                content.tree = PieceTree::from_original(0).replace_all_with_added(
                    offset + relative,
                    length,
                    0,
                );
                markdown_codec::write_encoded(
                    content.reader(),
                    output,
                    snapshot.source_map.encoding,
                )?;
            }
        }
    }
    Ok(())
}
