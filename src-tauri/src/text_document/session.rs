use super::{
    chunk_reader::{
        AddedStore, ChunkCache, DocumentSnapshot, OriginalFile, DEFAULT_CACHE_CAPACITY_BYTES,
    },
    edit_journal::{
        cleanup_recovery_conflict, BaselineIdentity, EditJournal, JournalRecord,
        JournalReplacementFile, RecoveryConflict, RecoverySnapshot,
    },
    encoding,
    line_index::{
        advance_json_state, LineIndex, LineIndexBuildCheckpoint, LineIndexDelta, LineIndexEdit,
    },
    piece_tree::{Piece, PieceTree},
    ApplySegmentedEditsResult, CancelSegmentedTaskResult, CheckSegmentedExternalChangeResult,
    EventSink, FlushSegmentedJournalResult, LineEnding, OpenSegmentedDocumentResult,
    SaveSegmentedRevisionResult, SegmentedDocumentKind, SegmentedEdit, SegmentedEditBatch,
    SegmentedExternalChangeKind, SegmentedHistoryResult, SegmentedSessionStatus, SegmentedWindow,
    StartSegmentedTaskRequest, StartSegmentedTaskResult, TextDocumentError, TextDocumentResult,
    TextEncoding,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
};
#[cfg(test)]
use std::{sync::Condvar, time::Duration};

const FIRST_WINDOW_BYTES: usize = 256 * 1024;
const OPEN_PROBE_BYTES: usize = FIRST_WINDOW_BYTES + 4;
const MAX_WINDOW_BYTES: usize = 1024 * 1024;
const MAX_EDIT_TRANSACTION_BYTES: u64 = 8 * 1024 * 1024;
const HISTORY_CAPACITY_BYTES: u64 = 64 * 1024 * 1024;
const INDEX_EDIT_IDLE_MILLIS: u64 = 500;
const VALIDATION_CHUNK_BYTES: usize = 256 * 1024;
// 与前端 lexer 的 100_000 字节阈值保持同一事实源语义，避免边界区间模式不一致。
const LONG_LINE_THRESHOLD_BYTES: usize = 100_000;
#[cfg(test)]
const SAVE_WRITE_FAULT_DISABLED: u64 = u64::MAX;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
enum HistoryOperation {
    Edits(Vec<SegmentedEdit>),
    Replacement(JournalReplacementFile),
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    undo: HistoryOperation,
    redo: HistoryOperation,
    weight: u64,
}

#[derive(Debug, Default)]
struct HistoryState {
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    bytes: u64,
    owned_assets: HashSet<PathBuf>,
}

impl HistoryState {
    fn push_transaction(&mut self, undo: HistoryOperation, redo: HistoryOperation, weight: u64) {
        self.clear_redo();
        for operation in [&undo, &redo] {
            if let HistoryOperation::Replacement(replacement) = operation {
                self.owned_assets.insert(PathBuf::from(&replacement.path));
            }
        }
        self.bytes = self.bytes.saturating_add(weight);
        self.undo.push_back(HistoryEntry { undo, redo, weight });
        // 容量是软上限：最新事务即便自身超限也必须保留，保证 replace-all 仍是可撤销事务。
        while self.bytes > HISTORY_CAPACITY_BYTES && self.undo.len() > 1 {
            let Some(removed) = self.undo.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.weight);
        }
    }

    fn clear_redo(&mut self) {
        for entry in self.redo.drain(..) {
            self.bytes = self.bytes.saturating_sub(entry.weight);
        }
    }

    fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn protected_assets(&self) -> HashSet<PathBuf> {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .flat_map(|entry| [&entry.undo, &entry.redo])
            .filter_map(|operation| match operation {
                HistoryOperation::Replacement(replacement) => {
                    Some(PathBuf::from(&replacement.path))
                }
                HistoryOperation::Edits(_) => None,
            })
            .collect()
    }

    fn cleanup_candidates(&self, journal_assets: &HashSet<PathBuf>) -> Vec<PathBuf> {
        let protected = self.protected_assets();
        self.owned_assets
            .iter()
            .filter(|path| !protected.contains(*path) && !journal_assets.contains(*path))
            .cloned()
            .collect()
    }

    fn cleanup_candidates_after_history_drop(
        &self,
        journal_assets: &HashSet<PathBuf>,
    ) -> Vec<PathBuf> {
        self.owned_assets
            .iter()
            .filter(|path| !journal_assets.contains(*path))
            .cloned()
            .collect()
    }

    fn forget_assets(&mut self, assets: &[PathBuf]) {
        for asset in assets {
            self.owned_assets.remove(asset);
        }
    }

    fn all_assets(&self) -> HashSet<PathBuf> {
        self.owned_assets
            .union(&self.protected_assets())
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
enum HistoryDirection {
    Undo,
    Redo,
}

#[derive(Debug)]
struct SessionState {
    revision: u64,
    persisted_revision: u64,
    piece_tree: PieceTree,
    original: Arc<OriginalFile>,
    line_index: Arc<LineIndex>,
    initial_index_pending: bool,
    initial_index_requires_exact: bool,
    pending_index_deltas: Vec<LineIndexDelta>,
    index_exact_rebuild_required: bool,
    history: HistoryState,
}

#[derive(Debug, Clone, Copy)]
struct SessionMetadata {
    encoding: TextEncoding,
    line_ending: LineEnding,
    filesystem_readonly: bool,
    baseline_ready: bool,
    validation_complete: bool,
}

impl SessionMetadata {
    fn readonly(self) -> bool {
        self.filesystem_readonly
            || !self.baseline_ready
            || !self.validation_complete
            || self.encoding == TextEncoding::Unsupported
    }
}

pub(super) struct DocumentSession {
    session_id: String,
    path: Mutex<PathBuf>,
    document_kind: SegmentedDocumentKind,
    bom_len: u64,
    metadata: Mutex<SessionMetadata>,
    baseline_error: Mutex<Option<String>>,
    added: Arc<AddedStore>,
    state: RwLock<SessionState>,
    baseline: Mutex<BaselineIdentity>,
    // 始终对应 persisted_revision；恢复日志只引用它，不依赖可能被 replace-all 改写的 current Original。
    recovery_baseline: Mutex<Arc<OriginalFile>>,
    initial_baseline_asset_path: PathBuf,
    journal: Mutex<EditJournal>,
    journal_flush_scheduled: AtomicBool,
    save_guard: Mutex<()>,
    save_in_progress: AtomicBool,
    chunk_cache: ChunkCache,
    // pending index 的远端行号补扫在单会话内串行，并把安全检查点回写，避免快速滚动叠加 O(offset)。
    line_scan_guard: Mutex<()>,
    index_cache_path: PathBuf,
    index_cache_key: String,
    event_sink: Option<EventSink>,
    initial_index_snapshot: Mutex<Option<DocumentSnapshot>>,
    validation_generation: AtomicU64,
    index_generation: AtomicU64,
    index_worker_running: AtomicBool,
    index_cache_generation: AtomicU64,
    index_reported_bytes: AtomicU64,
    index_reported_lines: AtomicU64,
    #[cfg(test)]
    index_worker_starts: AtomicU64,
    #[cfg(test)]
    index_build_starts: AtomicU64,
    #[cfg(test)]
    initial_index_build_starts: AtomicU64,
    #[cfg(test)]
    index_chunk_delay_millis: AtomicU64,
    #[cfg(test)]
    validation_chunk_delay_millis: u64,
    #[cfg(test)]
    baseline_copy_chunk_delay_millis: u64,
    closing: AtomicBool,
    lifecycle_guard: Mutex<()>,
}

impl DocumentSession {
    fn snapshot(&self) -> TextDocumentResult<DocumentSnapshot> {
        let state = self
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok(DocumentSnapshot {
            revision: state.revision,
            tree: state.piece_tree.clone(),
            original: state.original.clone(),
            added: self.added.clone(),
        })
    }

    fn snapshot_with_index(&self) -> TextDocumentResult<(DocumentSnapshot, Arc<LineIndex>)> {
        let state = self
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok((
            DocumentSnapshot {
                revision: state.revision,
                tree: state.piece_tree.clone(),
                original: state.original.clone(),
                added: self.added.clone(),
            },
            Arc::clone(&state.line_index),
        ))
    }

    fn metadata(&self) -> TextDocumentResult<SessionMetadata> {
        self.metadata
            .lock()
            .map(|metadata| *metadata)
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档元数据锁已损坏"))
    }

    fn ensure_writable(&self) -> TextDocumentResult<SessionMetadata> {
        self.ensure_active()?;
        let metadata = self.metadata()?;
        if metadata.readonly() {
            return Err(TextDocumentError::new(
                "document-readonly",
                "当前编码或磁盘权限只允许只读打开",
            ));
        }
        Ok(metadata)
    }

    fn ensure_active(&self) -> TextDocumentResult<()> {
        if self.closing.load(Ordering::Acquire) {
            return Err(TextDocumentError::new(
                "session-closing",
                "分段文档会话正在关闭",
            ));
        }
        Ok(())
    }
}

struct ManagerInner {
    root: PathBuf,
    registry: Mutex<SessionRegistry>,
    tasks: Mutex<HashMap<String, super::task_runner::TaskControl>>,
    copy_fallback_guard: Mutex<()>,
    #[cfg(test)]
    save_test_pause: SaveTestPause,
    #[cfg(test)]
    open_probe_test_pause: SaveTestPause,
    #[cfg(test)]
    reload_cleanup_fail: AtomicBool,
    #[cfg(test)]
    validation_delay_millis: AtomicU64,
    #[cfg(test)]
    force_copy_fallback: AtomicBool,
    #[cfg(test)]
    save_write_fail_after_bytes: AtomicU64,
    #[cfg(test)]
    fail_save_baseline_prepare: AtomicBool,
    #[cfg(test)]
    fail_save_state_prepare: AtomicBool,
    #[cfg(test)]
    fail_save_journal_prune: AtomicBool,
    #[cfg(test)]
    force_baseline_copy_fallback: AtomicBool,
    #[cfg(test)]
    baseline_copy_chunk_delay_millis: AtomicU64,
}

struct SessionRegistry {
    by_id: HashMap<String, Arc<DocumentSession>>,
    by_path: HashMap<PathBuf, String>,
}

struct PathReservation {
    inner: Arc<ManagerInner>,
    path: PathBuf,
    session_id: String,
    committed: bool,
}

impl PathReservation {
    fn acquire(
        inner: Arc<ManagerInner>,
        path: PathBuf,
        session_id: String,
    ) -> TextDocumentResult<Self> {
        let mut registry = inner
            .registry
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?;
        if let Some(existing_session) = registry.by_path.get(&path) {
            return Err(TextDocumentError::new(
                "session-already-open",
                format!("文件已在分段会话中打开：{existing_session}"),
            ));
        }
        registry.by_path.insert(path.clone(), session_id.clone());
        drop(registry);
        Ok(Self {
            inner,
            path,
            session_id,
            committed: false,
        })
    }

    fn commit(mut self, session: Arc<DocumentSession>) -> TextDocumentResult<()> {
        self.inner
            .registry
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?
            .by_id
            .insert(self.session_id.clone(), session);
        self.committed = true;
        Ok(())
    }
}

impl Drop for PathReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        match self.inner.registry.lock() {
            Ok(mut registry) => {
                if registry.by_path.get(&self.path) == Some(&self.session_id) {
                    registry.by_path.remove(&self.path);
                }
            }
            Err(error) => crate::app_logger::error(
                "SegmentedDocument",
                &format!("释放失败的打开路径占位失败：{error}"),
            ),
        }
    }
}

struct SavePathReservation {
    inner: Arc<ManagerInner>,
    session_id: String,
    previous_path: PathBuf,
    target_path: PathBuf,
    inserted_target: bool,
    committed: bool,
}

impl SavePathReservation {
    fn acquire(
        inner: Arc<ManagerInner>,
        session_id: &str,
        previous_path: PathBuf,
        target_path: PathBuf,
    ) -> TextDocumentResult<Self> {
        let mut registry = inner
            .registry
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?;
        let inserted_target = match registry.by_path.get(&target_path) {
            Some(owner) if owner != session_id => {
                return Err(TextDocumentError::new(
                    "save-target-open",
                    format!("保存目标已由另一分段会话打开：{owner}"),
                ));
            }
            Some(_) => false,
            None => {
                registry
                    .by_path
                    .insert(target_path.clone(), session_id.to_string());
                true
            }
        };
        drop(registry);
        Ok(Self {
            inner,
            session_id: session_id.to_string(),
            previous_path,
            target_path,
            inserted_target,
            committed: false,
        })
    }

    /// 保存成功后一次性切换路径占位；失败时由 Drop 释放预占的目标路径。
    fn commit(mut self) -> TextDocumentResult<()> {
        let mut registry = self
            .inner
            .registry
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?;
        if self.previous_path != self.target_path
            && registry.by_path.get(&self.previous_path) == Some(&self.session_id)
        {
            registry.by_path.remove(&self.previous_path);
        }
        registry
            .by_path
            .insert(self.target_path.clone(), self.session_id.clone());
        self.committed = true;
        Ok(())
    }
}

impl Drop for SavePathReservation {
    fn drop(&mut self) {
        if self.committed || !self.inserted_target {
            return;
        }
        match self.inner.registry.lock() {
            Ok(mut registry) => {
                if registry.by_path.get(&self.target_path) == Some(&self.session_id) {
                    registry.by_path.remove(&self.target_path);
                }
            }
            Err(error) => crate::app_logger::error(
                "SegmentedDocument",
                &format!("释放失败的保存路径占位失败：{error}"),
            ),
        }
    }
}

#[cfg(test)]
struct SaveTestPause {
    enabled: AtomicBool,
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
}

#[cfg(test)]
impl SaveTestPause {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    fn enable(&self) {
        *self.entered.0.lock().expect("save test entered") = false;
        *self.released.0.lock().expect("save test released") = false;
        self.enabled.store(true, Ordering::Release);
    }

    fn pause_if_enabled(&self) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        let mut entered = self.entered.0.lock().expect("save test entered");
        *entered = true;
        self.entered.1.notify_all();
        drop(entered);
        let mut released = self.released.0.lock().expect("save test released");
        while !*released {
            released = self.released.1.wait(released).expect("save test wait");
        }
    }

    fn wait_until_entered(&self) -> bool {
        let entered = self.entered.0.lock().expect("save test entered");
        let (entered, _) = self
            .entered
            .1
            .wait_timeout_while(entered, Duration::from_secs(5), |value| !*value)
            .expect("save test timeout");
        *entered
    }

    fn release(&self) {
        self.enabled.store(false, Ordering::Release);
        let mut released = self.released.0.lock().expect("save test released");
        *released = true;
        self.released.1.notify_all();
    }
}

/// 分段文档的唯一公共 seam；commands 和测试都只通过这些方法观察行为。
#[derive(Clone)]
pub(crate) struct DocumentSessionManager {
    inner: Arc<ManagerInner>,
}

pub(super) struct TaskCommitState {
    pub(super) revision: u64,
    pub(super) byte_length: u64,
    pub(super) persisted_revision: u64,
    pub(super) dirty: bool,
}

struct PreparedHistoryOperation {
    tree: PieceTree,
    original: Arc<OriginalFile>,
    journal_edits: Vec<SegmentedEdit>,
    replacement_file: Option<JournalReplacementFile>,
    index_edits: Option<Vec<LineIndexEdit>>,
}

struct SessionCandidate {
    session: Arc<DocumentSession>,
    result: OpenSegmentedDocumentResult,
    index_ready: bool,
    persist_cache: bool,
    validation_ready: bool,
    baseline_ready: bool,
    committed: bool,
}

impl SessionCandidate {
    fn activate(mut self) -> OpenSegmentedDocumentResult {
        self.committed = true;
        if self.baseline_ready {
            if !self.validation_ready {
                start_encoding_validation(self.session.clone());
            }
            if !self.index_ready {
                start_line_index(self.session.clone(), false, self.persist_cache);
            }
        } else {
            start_baseline_materialization(
                self.session.clone(),
                self.index_ready,
                self.validation_ready,
                self.persist_cache,
            );
        }
        self.result.clone()
    }
}

impl Drop for SessionCandidate {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(error) = self.session.added.remove() {
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理未启用的候选会话失败：{error}"),
            );
        }
    }
}

impl DocumentSessionManager {
    pub(crate) fn new(root: PathBuf) -> TextDocumentResult<Self> {
        // task 目录还保存 replace/undo/json 恢复资产；启动清理只能匹配复制兜底的 `.txt`。
        if let Err(error) = cleanup_copy_fallbacks(&root.join("tasks"), None) {
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理过期复制兜底失败，继续启动分段引擎：{error}"),
            );
        }
        if let Err(error) = cleanup_session_transients(&root.join("sessions")) {
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理未发布的会话 baseline/Added 资产失败：{error}"),
            );
        }
        Ok(Self {
            inner: Arc::new(ManagerInner {
                root,
                registry: Mutex::new(SessionRegistry {
                    by_id: HashMap::new(),
                    by_path: HashMap::new(),
                }),
                tasks: Mutex::new(HashMap::new()),
                copy_fallback_guard: Mutex::new(()),
                #[cfg(test)]
                save_test_pause: SaveTestPause::new(),
                #[cfg(test)]
                open_probe_test_pause: SaveTestPause::new(),
                #[cfg(test)]
                reload_cleanup_fail: AtomicBool::new(false),
                #[cfg(test)]
                validation_delay_millis: AtomicU64::new(0),
                #[cfg(test)]
                force_copy_fallback: AtomicBool::new(false),
                #[cfg(test)]
                save_write_fail_after_bytes: AtomicU64::new(SAVE_WRITE_FAULT_DISABLED),
                #[cfg(test)]
                fail_save_baseline_prepare: AtomicBool::new(false),
                #[cfg(test)]
                fail_save_state_prepare: AtomicBool::new(false),
                #[cfg(test)]
                fail_save_journal_prune: AtomicBool::new(false),
                #[cfg(test)]
                force_baseline_copy_fallback: AtomicBool::new(false),
                #[cfg(test)]
                baseline_copy_chunk_delay_millis: AtomicU64::new(0),
            }),
        })
    }

    pub(crate) fn open_document(
        &self,
        path: PathBuf,
        event_sink: Option<EventSink>,
    ) -> TextDocumentResult<OpenSegmentedDocumentResult> {
        let requested_path = absolute_normalized_path(&path)?;
        let path = match requested_path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                document_kind_from_path(&requested_path)?;
                let journal_path = self
                    .inner
                    .root
                    .join("recovery")
                    .join(format!("{}.journal", recovery_key(&requested_path)));
                if let Some(conflict) = EditJournal::recovery_conflict_for_missing(&journal_path)? {
                    let recovered = materialize_recovery_document(
                        &requested_path,
                        &self.inner.root,
                        &conflict,
                    )?;
                    cleanup_recovery_conflict(&conflict);
                    return Err(TextDocumentError::source_missing_recovered(&recovered));
                }
                return Err(TextDocumentError::new(
                    "open-failed",
                    format!(
                        "源文件不存在且没有可用恢复资产：{}",
                        requested_path.display()
                    ),
                ));
            }
            Err(error) => {
                return Err(TextDocumentError::new(
                    "open-failed",
                    format!("解析分段文档路径失败：{error}"),
                ));
            }
        };
        let session_id = next_session_id();
        let reservation =
            PathReservation::acquire(self.inner.clone(), path.clone(), session_id.clone())?;
        let candidate = self.build_candidate(path, session_id, event_sink, true)?;
        reservation.commit(candidate.session.clone())?;
        Ok(candidate.activate())
    }

    fn build_candidate(
        &self,
        path: PathBuf,
        session_id: String,
        event_sink: Option<EventSink>,
        recover: bool,
    ) -> TextDocumentResult<SessionCandidate> {
        let document_kind = document_kind_from_path(&path)?;
        // probe、baseline 与首窗句柄必须来自同一个文件对象；不能在读完 probe 后才按路径采身份。
        let mut probe_file = File::open(&path).map_err(|error| {
            TextDocumentError::new("open-failed", format!("打开分段文档失败：{error}"))
        })?;
        let file_metadata = probe_file.metadata().map_err(|error| {
            TextDocumentError::new("open-failed", format!("读取文件状态失败：{error}"))
        })?;
        if !file_metadata.is_file() {
            return Err(TextDocumentError::new("not-a-file", "目标路径不是文件"));
        }
        let baseline = BaselineIdentity::read_file(&path, &probe_file)?;

        // 候选会话只探测首窗；构造完全成功前不会触碰 registry 或旧 recovery。
        let mut probe_bytes = Vec::with_capacity(OPEN_PROBE_BYTES);
        std::io::Read::by_ref(&mut probe_file)
            .take(OPEN_PROBE_BYTES as u64)
            .read_to_end(&mut probe_bytes)?;
        #[cfg(test)]
        self.inner.open_probe_test_pause.pause_if_enabled();
        ensure_open_identity_unchanged(&path, &probe_file, &baseline)?;
        let probe_complete = baseline.physical_len <= probe_bytes.len() as u64;
        let probe = encoding::probe(&probe_bytes, probe_complete);
        let pending_original = Arc::new(OriginalFile::from_file(
            probe_file.try_clone()?,
            probe.bom_len,
        )?);
        // from_file 采集句柄元数据后再复核一次，封住校验与首窗句柄构造之间的窄窗口。
        ensure_open_identity_unchanged(&path, &probe_file, &baseline)?;
        let baseline_asset_path = self
            .inner
            .root
            .join("sessions")
            .join(format!("{session_id}.baseline"));
        #[cfg(test)]
        let force_copy_fallback = self
            .inner
            .force_baseline_copy_fallback
            .load(Ordering::Acquire);
        #[cfg(not(test))]
        let force_copy_fallback = false;
        let immutable_original = if force_copy_fallback {
            None
        } else {
            OriginalFile::try_open_cow_immutable(&path, baseline_asset_path.clone(), probe.bom_len)?
        };
        let immutable_original = if immutable_original.is_none() && probe_complete {
            Some(OriginalFile::create_immutable_from_bytes(
                baseline_asset_path.clone(),
                &probe_bytes,
                probe.bom_len,
            )?)
        } else {
            immutable_original
        };
        let baseline_ready = immutable_original.is_some();
        let original = match immutable_original {
            Some(original) => Arc::new(original),
            // 非 COW 文件系统只用源句柄发布首窗；baseline 完成前 readonly 门禁阻止产生补丁。
            None => pending_original,
        };
        ensure_open_identity_unchanged(&path, &probe_file, &baseline)?;
        let added = Arc::new(AddedStore::create(
            self.inner
                .root
                .join("sessions")
                .join(format!("{session_id}.added")),
        )?);
        let journal_path = self
            .inner
            .root
            .join("recovery")
            .join(format!("{}.journal", recovery_key(&path)));
        let (journal, persisted_revision, recovered_records, recovery_conflict) = if recover {
            EditJournal::open(journal_path, baseline.clone())?
        } else {
            (
                EditJournal::empty(journal_path, baseline.clone()),
                0,
                Vec::new(),
                None,
            )
        };
        let recovery_conflict_path = recovery_conflict
            .as_ref()
            .map(|conflict| materialize_recovery_document(&path, &self.inner.root, conflict))
            .transpose()?;
        if let Some(conflict) = &recovery_conflict {
            cleanup_recovery_conflict(conflict);
        }
        let mut piece_tree = PieceTree::from_original(original.body_len());
        let mut revision = persisted_revision;
        for record in &recovered_records {
            if record.base_revision != revision {
                return Err(TextDocumentError::new(
                    "journal-revision-gap",
                    "恢复日志 revision 不连续",
                ));
            }
            piece_tree = replay_record(&piece_tree, &added, record)?;
            revision = record.revision;
        }
        let index_cache_path = self
            .inner
            .root
            .join("indexes")
            .join(format!("{}.json", recovery_key(&path)));
        let index_cache_key = baseline_cache_key(&baseline);
        let cached_index = if recovered_records.is_empty() {
            match LineIndex::load_cache(&index_cache_path, &index_cache_key) {
                Ok(index) => index,
                Err(error) => {
                    crate::app_logger::warn(
                        "SegmentedDocument",
                        &format!("忽略无效行索引缓存并重新扫描：{error}"),
                    );
                    None
                }
            }
        } else {
            None
        };
        let index_ready = cached_index.is_some();
        let cached_metadata = cached_index.as_ref().map(|index| {
            let encoding = if index.utf8_valid() {
                if probe.bom_len == 3 {
                    TextEncoding::Utf8Bom
                } else {
                    TextEncoding::Utf8
                }
            } else {
                TextEncoding::Unsupported
            };
            (encoding, index.line_ending())
        });
        let initial_metadata = SessionMetadata {
            encoding: cached_metadata
                .map(|metadata| metadata.0)
                .unwrap_or(probe.encoding),
            line_ending: cached_metadata
                .map(|metadata| metadata.1)
                .unwrap_or(probe.line_ending),
            filesystem_readonly: file_metadata.permissions().readonly(),
            baseline_ready,
            validation_complete: index_ready
                || probe_complete
                || probe.encoding == TextEncoding::Unsupported,
        };
        let initial_index_snapshot = (!index_ready).then(|| DocumentSnapshot {
            revision,
            tree: piece_tree.clone(),
            original: original.clone(),
            added: added.clone(),
        });
        let initial_line_index = Arc::new(
            cached_index.unwrap_or_else(|| LineIndex::pending(piece_tree.len(), revision)),
        );
        let session = Arc::new(DocumentSession {
            session_id: session_id.clone(),
            path: Mutex::new(path),
            document_kind,
            bom_len: probe.bom_len,
            metadata: Mutex::new(initial_metadata),
            baseline_error: Mutex::new(None),
            added,
            state: RwLock::new(SessionState {
                revision,
                persisted_revision,
                original: original.clone(),
                line_index: initial_line_index,
                initial_index_pending: !index_ready,
                initial_index_requires_exact: false,
                pending_index_deltas: Vec::new(),
                index_exact_rebuild_required: false,
                piece_tree,
                history: HistoryState::default(),
            }),
            baseline: Mutex::new(baseline),
            recovery_baseline: Mutex::new(original.clone()),
            initial_baseline_asset_path: baseline_asset_path,
            journal: Mutex::new(journal),
            journal_flush_scheduled: AtomicBool::new(false),
            save_guard: Mutex::new(()),
            save_in_progress: AtomicBool::new(false),
            chunk_cache: ChunkCache::new(DEFAULT_CACHE_CAPACITY_BYTES),
            line_scan_guard: Mutex::new(()),
            index_cache_path,
            index_cache_key,
            event_sink,
            initial_index_snapshot: Mutex::new(initial_index_snapshot),
            validation_generation: AtomicU64::new(0),
            index_generation: AtomicU64::new(0),
            index_worker_running: AtomicBool::new(false),
            index_cache_generation: AtomicU64::new(0),
            index_reported_bytes: AtomicU64::new(0),
            index_reported_lines: AtomicU64::new(0),
            #[cfg(test)]
            index_worker_starts: AtomicU64::new(0),
            #[cfg(test)]
            index_build_starts: AtomicU64::new(0),
            #[cfg(test)]
            initial_index_build_starts: AtomicU64::new(0),
            #[cfg(test)]
            index_chunk_delay_millis: AtomicU64::new(0),
            #[cfg(test)]
            validation_chunk_delay_millis: self
                .inner
                .validation_delay_millis
                .load(Ordering::Acquire),
            #[cfg(test)]
            baseline_copy_chunk_delay_millis: self
                .inner
                .baseline_copy_chunk_delay_millis
                .load(Ordering::Acquire),
            closing: AtomicBool::new(false),
            lifecycle_guard: Mutex::new(()),
        });
        let (first_snapshot, first_index) = session.snapshot_with_index()?;
        let first_window = read_window_from_snapshot(
            &session,
            &first_snapshot,
            &first_index,
            0,
            FIRST_WINDOW_BYTES,
            None,
        )?;
        let metadata = session.metadata()?;
        let result = OpenSegmentedDocumentResult {
            session_id,
            revision,
            persisted_revision,
            document_kind,
            encoding: metadata.encoding,
            line_ending: metadata.line_ending,
            byte_length: first_snapshot.len(),
            filesystem_readonly: metadata.filesystem_readonly,
            readonly: metadata.readonly(),
            first_window,
            recovery_conflict_path: recovery_conflict_path
                .map(|path| path.to_string_lossy().into_owned()),
        };
        Ok(SessionCandidate {
            session,
            result,
            index_ready,
            persist_cache: recovered_records.is_empty(),
            validation_ready: initial_metadata.validation_complete,
            baseline_ready,
            committed: false,
        })
    }

    pub(crate) fn reload_session(
        &self,
        session_id: &str,
        event_sink: Option<EventSink>,
    ) -> TextDocumentResult<OpenSegmentedDocumentResult> {
        let old_session = self.session(session_id)?;
        let path = old_session
            .path
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))?
            .clone();
        // 候选的文件、首窗、编码探测和索引缓存全部先验证；失败时旧会话完全不动。
        let candidate = self.build_candidate(path.clone(), next_session_id(), event_sink, false)?;
        if old_session
            .closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TextDocumentError::new(
                "session-closing",
                "分段文档会话正在关闭",
            ));
        }
        self.cancel_tasks_for_session(session_id);
        old_session.index_generation.fetch_add(1, Ordering::AcqRel);
        old_session
            .validation_generation
            .fetch_add(1, Ordering::AcqRel);
        let swap_result = (|| -> TextDocumentResult<()> {
            let _save_guard = old_session
                .save_guard
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "保存锁已损坏"))?;
            let _lifecycle = old_session
                .lifecycle_guard
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
            let history_assets = old_session
                .state
                .read()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?
                .history
                .all_assets();
            let mut journal = old_session
                .journal
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
            let current_path = old_session
                .path
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))?
                .clone();
            if current_path != path {
                return Err(TextDocumentError::new(
                    "reload-path-changed",
                    "构建 reload 候选期间文档路径已变化，请重试",
                ));
            }
            let mut registry = self
                .inner
                .registry
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?;
            if !registry
                .by_id
                .get(session_id)
                .is_some_and(|registered| Arc::ptr_eq(registered, &old_session))
                || registry.by_path.get(&path).map(String::as_str) != Some(session_id)
            {
                return Err(TextDocumentError::new(
                    "session-not-found",
                    "旧分段文档会话已被替换",
                ));
            }
            registry.by_id.remove(session_id);
            registry.by_id.insert(
                candidate.session.session_id.clone(),
                candidate.session.clone(),
            );
            registry
                .by_path
                .insert(path, candidate.session.session_id.clone());
            drop(registry);

            // registry 交换后 reload 已提交；旧资产清理失败只能告警，不能返回失败并假装旧会话仍可用。
            #[cfg(test)]
            let force_cleanup_failure =
                self.inner.reload_cleanup_fail.swap(false, Ordering::AcqRel);
            #[cfg(not(test))]
            let force_cleanup_failure = false;
            if force_cleanup_failure {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    "测试注入：reload 已提交，跳过旧会话资产清理",
                );
                return Ok(());
            }
            if let Err(error) = journal.remove() {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    &format!("reload 后清理旧恢复日志失败：{error}"),
                );
            }
            for asset in history_assets {
                if let Err(error) = remove_file_if_exists(&asset) {
                    crate::app_logger::warn(
                        "SegmentedDocument",
                        &format!("reload 后清理旧历史资产失败：{error}"),
                    );
                }
            }
            if let Err(error) = old_session.added.remove() {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    &format!("reload 后清理旧 Added 文件失败：{error}"),
                );
            }
            Ok(())
        })();
        if let Err(error) = swap_result {
            old_session.closing.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(candidate.activate())
    }

    #[cfg(test)]
    pub(crate) fn read_window(
        &self,
        session_id: &str,
        revision: u64,
        start_byte: u64,
        target_bytes: usize,
    ) -> TextDocumentResult<SegmentedWindow> {
        let session = self.session(session_id)?;
        let (snapshot, line_index) = session.snapshot_with_index()?;
        if revision != snapshot.revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                snapshot.revision,
            ));
        }
        read_window_from_snapshot(
            &session,
            &snapshot,
            &line_index,
            start_byte,
            target_bytes,
            None,
        )
    }

    pub(crate) fn read_window_with_request(
        &self,
        session_id: &str,
        revision: u64,
        start_byte: u64,
        target_bytes: usize,
        request_id: u64,
    ) -> TextDocumentResult<SegmentedWindow> {
        let session = self.session(session_id)?;
        let (snapshot, line_index) = session.snapshot_with_index()?;
        if revision != snapshot.revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                snapshot.revision,
            ));
        }
        read_window_from_snapshot(
            &session,
            &snapshot,
            &line_index,
            start_byte,
            target_bytes,
            Some(request_id),
        )
    }

    pub(crate) fn apply_edits(
        &self,
        batch: SegmentedEditBatch,
    ) -> TextDocumentResult<ApplySegmentedEditsResult> {
        let session = self.session(&batch.session_id)?;
        let _lifecycle = session
            .lifecycle_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
        let metadata = session.ensure_writable()?;

        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        if state.revision != batch.base_revision {
            return Err(TextDocumentError::revision_conflict(
                batch.base_revision,
                state.revision,
            ));
        }
        if batch.edits.is_empty() {
            return Ok(ApplySegmentedEditsResult {
                revision: state.revision,
                persisted_revision: state.persisted_revision,
                dirty: state.revision != state.persisted_revision,
                invalidated_from_byte: 0,
                invalidated_to_byte: 0,
            });
        }

        let snapshot = DocumentSnapshot {
            revision: state.revision,
            tree: state.piece_tree.clone(),
            original: state.original.clone(),
            added: session.added.clone(),
        };
        let base_tree = state.piece_tree.clone();
        drop(state);
        let mut edits = batch.edits.clone();
        edits.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
        validate_edit_transaction_size(&edits)?;
        validate_edit_batch(&snapshot, &edits)?;

        let mut prepared = Vec::with_capacity(edits.len());
        let mut journal_edits = Vec::with_capacity(edits.len());
        let mut inverse_edits = Vec::with_capacity(edits.len());
        let mut index_edits = Vec::with_capacity(edits.len());
        let mut invalidated_from = u64::MAX;
        let mut invalidated_to = 0_u64;
        let mut history_weight = 0_u64;
        let mut coordinate_delta = 0_i128;
        for edit in edits {
            let inserted =
                encoding::normalize_inserted_text(&edit.inserted_text, metadata.line_ending);
            let deleted =
                snapshot.read_range(edit.from_byte, (edit.to_byte - edit.from_byte) as usize)?;
            let deleted_lines = encoding::count_line_breaks(&deleted);
            let inserted_lines = encoding::count_line_breaks(&inserted);
            let offset = if inserted.is_empty() {
                None
            } else {
                Some(session.added.append(&inserted)?)
            };
            invalidated_from = invalidated_from.min(edit.from_byte);
            invalidated_to =
                invalidated_to.max(edit.to_byte.max(edit.from_byte + inserted.len() as u64));
            history_weight = history_weight
                .saturating_add(edit.to_byte.saturating_sub(edit.from_byte))
                .saturating_add(inserted.len() as u64);
            let inserted_text = String::from_utf8(inserted.clone())
                .map_err(|_| TextDocumentError::new("invalid-utf8", "插入文本不是有效 UTF-8"))?;
            let deleted_text = String::from_utf8(deleted)
                .map_err(|_| TextDocumentError::new("invalid-utf8", "被替换范围不是有效 UTF-8"))?;
            let inverse_from = (edit.from_byte as i128 + coordinate_delta) as u64;
            inverse_edits.push(SegmentedEdit {
                from_byte: inverse_from,
                to_byte: inverse_from + inserted.len() as u64,
                inserted_text: deleted_text,
            });
            coordinate_delta +=
                inserted.len() as i128 - edit.to_byte.saturating_sub(edit.from_byte) as i128;
            journal_edits.push(SegmentedEdit {
                from_byte: edit.from_byte,
                to_byte: edit.to_byte,
                inserted_text,
            });
            index_edits.push(LineIndexEdit {
                from_byte: edit.from_byte,
                to_byte: edit.to_byte,
                inserted_bytes: inserted.len() as u64,
                inserted_lines,
                deleted_lines,
            });
            prepared.push((edit.from_byte, edit.to_byte, offset, inserted));
        }

        let mut next_tree = base_tree;
        for (from, to, offset, inserted) in prepared.into_iter().rev() {
            let piece = offset.map(|offset| {
                Piece::added(
                    offset,
                    inserted.len() as u64,
                    encoding::count_line_breaks(&inserted),
                )
            });
            next_tree = next_tree.replace_range(from, to, piece)?;
        }
        session.chunk_cache.clear()?;
        // journal 与 state 在同一提交区间内持锁，避免“命令返回错误但 revision 已生效”。
        let mut journal = session
            .journal
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
        let mut state = session
            .state
            .write()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
        if state.revision != batch.base_revision {
            return Err(TextDocumentError::revision_conflict(
                batch.base_revision,
                state.revision,
            ));
        }
        let next_revision = batch.base_revision + 1;
        state.history.push_transaction(
            HistoryOperation::Edits(inverse_edits),
            HistoryOperation::Edits(journal_edits.clone()),
            history_weight,
        );
        state.revision = next_revision;
        state.piece_tree = next_tree;
        update_line_index_after_edits(
            &mut state,
            LineIndexDelta {
                base_revision: batch.base_revision,
                revision: next_revision,
                edits: index_edits,
            },
        );
        let persisted_revision = state.persisted_revision;
        journal.record(JournalRecord {
            base_revision: batch.base_revision,
            revision: next_revision,
            edits: journal_edits,
            replacement_file: None,
        });
        cleanup_unreferenced_history_assets(&mut state, &journal);
        drop(state);
        drop(journal);
        schedule_journal_flush(session.clone());
        start_line_index(session.clone(), true, false);

        Ok(ApplySegmentedEditsResult {
            revision: next_revision,
            persisted_revision,
            dirty: next_revision != persisted_revision,
            invalidated_from_byte: invalidated_from,
            invalidated_to_byte: invalidated_to,
        })
    }

    pub(crate) fn undo_revision(
        &self,
        session_id: &str,
        base_revision: u64,
    ) -> TextDocumentResult<SegmentedHistoryResult> {
        self.move_history(session_id, base_revision, HistoryDirection::Undo)
    }

    pub(crate) fn redo_revision(
        &self,
        session_id: &str,
        base_revision: u64,
    ) -> TextDocumentResult<SegmentedHistoryResult> {
        self.move_history(session_id, base_revision, HistoryDirection::Redo)
    }

    fn move_history(
        &self,
        session_id: &str,
        base_revision: u64,
        direction: HistoryDirection,
    ) -> TextDocumentResult<SegmentedHistoryResult> {
        let session = self.session(session_id)?;
        let _lifecycle = session
            .lifecycle_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
        session.ensure_active()?;
        if matches!(direction, HistoryDirection::Redo) {
            // 后台发现非法编码后，只允许继续 undo 直至基线；redo 和新编辑仍受只读门禁约束。
            session.ensure_writable()?;
        }
        let (operation, snapshot, line_ending) = {
            let state = session
                .state
                .read()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
            if state.revision != base_revision {
                return Err(TextDocumentError::revision_conflict(
                    base_revision,
                    state.revision,
                ));
            }
            let entry = match direction {
                HistoryDirection::Undo => state.history.undo.back(),
                HistoryDirection::Redo => state.history.redo.back(),
            };
            let Some(entry) = entry else {
                return Ok(SegmentedHistoryResult {
                    changed: false,
                    revision: state.revision,
                    persisted_revision: state.persisted_revision,
                    byte_length: state.piece_tree.len(),
                    dirty: state.revision != state.persisted_revision,
                    can_undo: state.history.can_undo(),
                    can_redo: state.history.can_redo(),
                });
            };
            let operation = match direction {
                HistoryDirection::Undo => entry.undo.clone(),
                HistoryDirection::Redo => entry.redo.clone(),
            };
            (
                operation,
                DocumentSnapshot {
                    revision: state.revision,
                    tree: state.piece_tree.clone(),
                    original: state.original.clone(),
                    added: session.added.clone(),
                },
                session.metadata()?.line_ending,
            )
        };
        let prepared = prepare_history_operation(&session, &snapshot, &operation, line_ending)?;
        session.chunk_cache.clear()?;
        let mut journal = session
            .journal
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
        let mut state = session
            .state
            .write()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
        if state.revision != base_revision {
            return Err(TextDocumentError::revision_conflict(
                base_revision,
                state.revision,
            ));
        }
        let entry = match direction {
            HistoryDirection::Undo => state.history.undo.pop_back(),
            HistoryDirection::Redo => state.history.redo.pop_back(),
        }
        .ok_or_else(|| TextDocumentError::new("history-empty", "撤销/重做历史已变化"))?;
        match direction {
            HistoryDirection::Undo => state.history.redo.push_back(entry),
            HistoryDirection::Redo => state.history.undo.push_back(entry),
        }
        state.piece_tree = prepared.tree;
        state.original = prepared.original;
        state.revision += 1;
        if let Some(index_edits) = prepared.index_edits {
            update_line_index_after_edits(
                &mut state,
                LineIndexDelta {
                    base_revision,
                    revision: base_revision + 1,
                    edits: index_edits,
                },
            );
        } else {
            invalidate_line_index_for_replacement(&mut state);
        }
        journal.record(JournalRecord {
            base_revision,
            revision: state.revision,
            edits: prepared.journal_edits,
            replacement_file: prepared.replacement_file,
        });
        cleanup_unreferenced_history_assets(&mut state, &journal);
        let result = SegmentedHistoryResult {
            changed: true,
            revision: state.revision,
            persisted_revision: state.persisted_revision,
            byte_length: state.piece_tree.len(),
            dirty: state.revision != state.persisted_revision,
            can_undo: state.history.can_undo(),
            can_redo: state.history.can_redo(),
        };
        drop(state);
        drop(journal);
        schedule_journal_flush(session.clone());
        start_line_index(session.clone(), true, false);
        Ok(result)
    }

    pub(crate) fn flush_journal(
        &self,
        session_id: &str,
        revision: u64,
    ) -> TextDocumentResult<FlushSegmentedJournalResult> {
        let session = self.session(session_id)?;
        let _save_guard = session
            .save_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "保存锁已损坏"))?;
        flush_session_journal(&session, Some(revision))?;
        Ok(FlushSegmentedJournalResult { revision })
    }

    pub(crate) fn save_revision(
        &self,
        session_id: &str,
        revision: u64,
        target_path: Option<PathBuf>,
        overwrite_external: bool,
    ) -> TextDocumentResult<SaveSegmentedRevisionResult> {
        let session = self.session(session_id)?;
        let _save_guard = session
            .save_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "保存锁已损坏"))?;
        if session.closing.load(Ordering::Acquire) {
            return Err(TextDocumentError::new(
                "session-closing",
                "分段文档会话正在关闭",
            ));
        }
        session.save_in_progress.store(true, Ordering::Release);
        let _progress_guard = SaveProgressGuard(&session.save_in_progress);
        let snapshot = session.snapshot()?;
        if snapshot.revision != revision {
            return Err(TextDocumentError::revision_conflict(
                revision,
                snapshot.revision,
            ));
        }
        let save_metadata = session.metadata()?;
        if !save_metadata.baseline_ready {
            return Err(TextDocumentError::new(
                "baseline-materialization-pending",
                "不可变 baseline 仍在后台准备，完成前不能保存",
            ));
        }
        let persisted_revision = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?
            .persisted_revision;
        if !save_metadata.validation_complete && revision != persisted_revision {
            return Err(TextDocumentError::new(
                "encoding-validation-pending",
                "后台编码校验尚未完成；补丁已保留，校验完成后可保存",
            ));
        }
        if save_metadata.encoding == TextEncoding::Unsupported {
            if revision != persisted_revision {
                return Err(TextDocumentError::new(
                    "unsupported-dirty-save",
                    "后台发现非 UTF-8 内容；已产生的补丁会保留，但必须先转换编码再保存",
                ));
            }
        }
        #[cfg(test)]
        self.inner.save_test_pause.pause_if_enabled();

        let current_path = session
            .path
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))?
            .clone();
        let target = normalize_save_target(
            target_path
                .as_deref()
                .unwrap_or_else(|| current_path.as_path()),
        )?;
        if !target.is_dir() && document_kind_from_path(&target)? != session.document_kind {
            return Err(TextDocumentError::new(
                "save-kind-mismatch",
                "另存为目标必须保留当前 TXT/JSON 文档类型",
            ));
        }
        let saving_original_path = target == current_path;
        if saving_original_path && !overwrite_external {
            // recovery flush 会读取 immutable original；先稳定映射外部修改错误，避免底层
            // snapshot reader 的 IO 错误越过公开的 external-file-changed 契约。
            ensure_baseline_unchanged(&session, &target)?;
        }
        // frozen snapshot 已捕获并完成基线校验；随后刷新可包含更高 revision，保存仍只写 frozen tree。
        flush_session_journal(&session, None)?;

        let path_reservation = SavePathReservation::acquire(
            self.inner.clone(),
            session_id,
            current_path.clone(),
            target.clone(),
        )?;
        if saving_original_path && !overwrite_external {
            ensure_baseline_unchanged(&session, &target)?;
        }
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(TextDocumentError::new(
                "save-directory-missing",
                format!("保存目录不存在：{}", parent.display()),
            ));
        }
        let temp_path = unique_temp_path(parent, &target);
        let save_result = (|| -> TextDocumentResult<()> {
            let mut temp = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(|error| {
                    TextDocumentError::new(
                        "save-temp-create-failed",
                        format!("创建临时保存文件失败：{error}"),
                    )
                })?;
            if session.bom_len == 3 {
                temp.write_all(encoding::UTF8_BOM)?;
            }
            let mut reader = snapshot.reader();
            #[cfg(test)]
            let write_fault_after = {
                let limit = self
                    .inner
                    .save_write_fail_after_bytes
                    .swap(SAVE_WRITE_FAULT_DISABLED, Ordering::AcqRel);
                (limit != SAVE_WRITE_FAULT_DISABLED).then_some(limit)
            };
            #[cfg(not(test))]
            let write_fault_after = None;
            copy_snapshot_for_save(&mut reader, &mut temp, write_fault_after).map_err(|error| {
                TextDocumentError::new(
                    "save-write-failed",
                    format!("流式写入临时文件失败：{error}"),
                )
            })?;
            match std::fs::metadata(&target) {
                Ok(metadata) => temp
                    .set_permissions(metadata.permissions())
                    .map_err(|error| {
                        TextDocumentError::new(
                            "save-permissions-failed",
                            format!("保留目标文件权限失败：{error}"),
                        )
                    })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(TextDocumentError::new(
                        "save-metadata-failed",
                        format!("读取目标文件权限失败：{error}"),
                    ));
                }
            }
            temp.sync_all().map_err(|error| {
                TextDocumentError::new("save-sync-failed", format!("同步临时保存文件失败：{error}"))
            })?;
            drop(temp);
            if saving_original_path && !overwrite_external {
                ensure_baseline_unchanged(&session, &target)?;
            }
            replace_file(&temp_path, &target).map_err(|error| {
                TextDocumentError::new("save-replace-failed", format!("原子替换文件失败：{error}"))
            })?;
            sync_parent_dir(parent)?;
            Ok(())
        })();
        if let Err(error) = save_result {
            if let Err(cleanup_error) = remove_file_if_exists(&temp_path) {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    &format!("清理失败的保存临时文件失败：{cleanup_error}"),
                );
            }
            return Err(error);
        }

        let new_baseline = BaselineIdentity::read(&target)?;
        let target_readonly = std::fs::metadata(&target)
            .map_err(|error| {
                TextDocumentError::new(
                    "save-metadata-failed",
                    format!("读取已保存文件权限失败：{error}"),
                )
            })?
            .permissions()
            .readonly();
        let replacement_original = Arc::new(OriginalFile::open_immutable(
            &target,
            self.inner.root.join("sessions").join(format!(
                "{}-{revision}-{}.baseline",
                session.session_id,
                next_session_id()
            )),
            session.bom_len,
        )?);
        if BaselineIdentity::read(&target)? != new_baseline {
            return Err(TextDocumentError::new(
                "external-file-changed-after-save",
                "保存后建立不可变 baseline 期间目标文件又被外部修改",
            ));
        }
        let next_journal_path = self
            .inner
            .root
            .join("recovery")
            .join(format!("{}.journal", recovery_key(&target)));
        // apply_edits 也按 journal -> state 顺序提交；这里沿用同一锁序，保存旧 revision
        // 时只剪掉已落盘记录，并保留并发产生的更高 revision。
        let _lifecycle = session
            .lifecycle_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
        let mut journal = session
            .journal
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
        let mut state = session
            .state
            .write()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
        let mut rebased_tree = PieceTree::from_original(replacement_original.body_len());
        for record in journal
            .records()
            .iter()
            .filter(|record| record.revision > revision)
        {
            rebased_tree = replay_record(&rebased_tree, &session.added, record)?;
        }
        let protected_assets = state.history.protected_assets();
        journal.prune(
            revision,
            new_baseline.clone(),
            next_journal_path,
            &protected_assets,
            session.bom_len,
        )?;
        cleanup_unreferenced_history_assets(&mut state, &journal);
        path_reservation.commit()?;
        *session
            .path
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))? =
            target.clone();
        *session
            .baseline
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文件基线锁已损坏"))? =
            new_baseline.clone();
        *session
            .recovery_baseline
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复 baseline 锁已损坏"))? =
            replacement_original.clone();
        let readonly = {
            let mut metadata = session
                .metadata
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档元数据锁已损坏"))?;
            metadata.filesystem_readonly = target_readonly;
            metadata.readonly()
        };
        // 保存后让 Original 指向刚写出的目标快照；更高 revision 由保留 journal 重放，
        // 这样 Save As 后旧源文件再被修改也不会污染当前会话。
        state.original = replacement_original;
        state.piece_tree = rebased_tree;
        state.persisted_revision = revision;
        let current_revision = state.revision;
        let persisted_revision = state.persisted_revision;
        let dirty = current_revision != persisted_revision;
        let index_to_cache = (current_revision == revision && state.line_index.is_completed())
            .then(|| state.line_index.clone());
        drop(state);
        drop(journal);
        if let Some(index) = index_to_cache {
            let cache_path = self
                .inner
                .root
                .join("indexes")
                .join(format!("{}.json", recovery_key(&target)));
            if let Err(error) = index.save_cache(&cache_path, &baseline_cache_key(&new_baseline)) {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    &format!("保存目标行索引缓存失败：{error}"),
                );
            }
        }

        Ok(SaveSegmentedRevisionResult {
            session_id: session_id.to_string(),
            saved_revision: revision,
            current_revision,
            persisted_revision,
            dirty,
            filesystem_readonly: target_readonly,
            readonly,
            modified_at: (new_baseline.modified_nanos / 1_000_000_000) as i64,
        })
    }

    pub(crate) fn close_session(
        &self,
        session_id: &str,
        discard_changes: bool,
    ) -> TextDocumentResult<()> {
        let session = self.session(session_id)?;
        if session
            .closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TextDocumentError::new(
                "session-closing",
                "分段文档会话正在关闭",
            ));
        }
        self.cancel_tasks_for_session(session_id);
        session.index_generation.fetch_add(1, Ordering::AcqRel);
        session.validation_generation.fetch_add(1, Ordering::AcqRel);
        let close_result = (|| -> TextDocumentResult<()> {
            // 锁序与 save_revision 一致：先等待保存，再等待已进入提交点的编辑/写任务。
            let _save_guard = session
                .save_guard
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "保存锁已损坏"))?;
            let _lifecycle = session
                .lifecycle_guard
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
            let state = session
                .state
                .read()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
            let dirty = state.revision != state.persisted_revision;
            let revision = state.revision;
            let history_assets = state.history.all_assets();
            drop(state);
            if discard_changes || !dirty {
                session
                    .journal
                    .lock()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?
                    .remove()?;
                for asset in history_assets {
                    remove_file_if_exists(&asset)?;
                }
                session.added.remove()?;
            } else {
                flush_session_journal(&session, Some(revision))?;
                let journal = session
                    .journal
                    .lock()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
                let mut state = session
                    .state
                    .write()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
                if state.revision != revision {
                    return Err(TextDocumentError::revision_conflict(
                        revision,
                        state.revision,
                    ));
                }
                let orphaned = state
                    .history
                    .cleanup_candidates_after_history_drop(&journal.referenced_assets());
                for asset in &orphaned {
                    remove_file_if_exists(asset)?;
                }
                state.history.forget_assets(&orphaned);
                // journal 已内嵌普通补丁，任务替换另有独立文件引用；旧 session Added 不参与恢复。
                session.added.remove()?;
            }
            let mut registry = self
                .inner
                .registry
                .lock()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?;
            registry.by_id.remove(session_id);
            registry.by_path.retain(|_, owner| owner != session_id);
            Ok(())
        })();
        if close_result.is_err() {
            session.closing.store(false, Ordering::Release);
        }
        close_result
    }

    pub(crate) fn start_task(
        &self,
        request: StartSegmentedTaskRequest,
        event_sink: Option<EventSink>,
    ) -> TextDocumentResult<StartSegmentedTaskResult> {
        let session = self.session(&request.session_id)?;
        if matches!(
            &request.task,
            super::SegmentedTask::ReplaceAll { .. } | super::SegmentedTask::JsonFormat
        ) {
            session.ensure_writable()?;
        }
        super::task_runner::start(self.clone(), request, event_sink)
    }

    pub(crate) fn check_external_change(
        &self,
        session_id: &str,
    ) -> TextDocumentResult<CheckSegmentedExternalChangeResult> {
        let session = self.session(session_id)?;
        let path = session
            .path
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))?
            .clone();
        let baseline = session
            .baseline
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文件基线锁已损坏"))?
            .clone();
        let (change, modified_at, current_identity) = match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (SegmentedExternalChangeKind::Deleted, 0, None)
            }
            Err(error) => {
                return Err(TextDocumentError::new(
                    "file-identity-failed",
                    format!("检查外部文件变化失败：{error}"),
                ));
            }
            Ok(_) => match BaselineIdentity::read(&path) {
                Ok(identity) => {
                    let modified_at = (identity.modified_nanos / 1_000_000_000) as i64;
                    let change = if identity == baseline {
                        SegmentedExternalChangeKind::None
                    } else {
                        SegmentedExternalChangeKind::Modified
                    };
                    (change, modified_at, Some(identity))
                }
                Err(error) => return Err(error),
            },
        };
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok(CheckSegmentedExternalChangeResult {
            session_id: session_id.to_string(),
            revision: state.revision,
            change,
            change_token: external_change_token(change, current_identity.as_ref(), &baseline),
            modified_at,
            dirty: state.revision != state.persisted_revision,
            save_in_progress: session.save_in_progress.load(Ordering::Acquire),
        })
    }

    pub(crate) fn session_status(
        &self,
        session_id: &str,
    ) -> TextDocumentResult<SegmentedSessionStatus> {
        let session = self.session(session_id)?;
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        let metadata = session.metadata()?;
        let baseline_error = session
            .baseline_error
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "baseline 错误锁已损坏"))?
            .clone();
        Ok(SegmentedSessionStatus {
            session_id: session_id.to_string(),
            revision: state.revision,
            persisted_revision: state.persisted_revision,
            byte_length: state.piece_tree.len(),
            indexed_bytes: state.line_index.indexed_bytes(),
            total_bytes: state.line_index.total_bytes(),
            estimated_lines: state.line_index.estimated_lines(),
            completed: state.line_index.is_completed(),
            encoding: metadata.encoding,
            line_ending: metadata.line_ending,
            filesystem_readonly: metadata.filesystem_readonly,
            readonly: metadata.readonly(),
            baseline_error,
            can_undo: state.history.can_undo(),
            can_redo: state.history.can_redo(),
        })
    }

    pub(crate) fn cancel_task(
        &self,
        task_id: &str,
    ) -> TextDocumentResult<CancelSegmentedTaskResult> {
        let cancelled = self
            .inner
            .tasks
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "任务表锁已损坏"))?
            .get(task_id)
            .map(|control| {
                control.cancel.store(true, Ordering::Release);
                true
            })
            .unwrap_or(false);
        Ok(CancelSegmentedTaskResult {
            task_id: task_id.to_string(),
            cancelled,
        })
    }

    pub(super) fn register_task(
        &self,
        task_id: String,
        control: super::task_runner::TaskControl,
    ) -> TextDocumentResult<()> {
        self.inner
            .tasks
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "任务表锁已损坏"))?
            .insert(task_id, control);
        Ok(())
    }

    pub(super) fn finish_task(&self, task_id: &str) {
        match self.inner.tasks.lock() {
            Ok(mut tasks) => {
                tasks.remove(task_id);
            }
            Err(error) => crate::app_logger::error(
                "SegmentedDocument",
                &format!("清理后台任务状态失败：{error}"),
            ),
        }
    }

    /// 抽屉搜索只需要固定 revision 的正文，不等待或创建全文行索引。
    #[cfg(any(mobile, test))]
    pub(crate) fn mobile_search_snapshot_readers(
        &self,
        session_id: &str,
        revision: u64,
    ) -> Result<impl Fn() -> Box<dyn std::io::Read + Send>, String> {
        let session = self.session(session_id).map_err(|_| "session-closed")?;
        let snapshot = session.snapshot().map_err(|_| "snapshot-unavailable")?;
        if snapshot.revision != revision {
            return Err("revision-changed".into());
        }
        // Re-read the same immutable snapshot for encoding detection, without buffering its body.
        Ok(move || -> Box<dyn std::io::Read + Send> { Box::new(snapshot.reader()) })
    }

    pub(super) fn task_snapshot(
        &self,
        session_id: &str,
        base_revision: u64,
    ) -> TextDocumentResult<(SegmentedDocumentKind, LineEnding, bool, DocumentSnapshot)> {
        let session = self.session(session_id)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let (snapshot, line_index) = session.snapshot_with_index()?;
            if snapshot.revision != base_revision {
                return Err(TextDocumentError::revision_conflict(
                    base_revision,
                    snapshot.revision,
                ));
            }
            if line_index.is_completed()
                && line_index.revision() == snapshot.revision
                && line_index.line_ending_exact()
            {
                let metadata = session.metadata()?;
                return Ok((
                    session.document_kind,
                    metadata.line_ending,
                    metadata.readonly(),
                    snapshot,
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(TextDocumentError::new(
                    "index-pending",
                    "全文索引尚未完成，请在 completed 事件后重试",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub(super) fn task_output_path(&self, task_id: &str, extension: &str) -> PathBuf {
        self.inner
            .root
            .join("tasks")
            .join(format!("{task_id}.{extension}"))
    }

    pub(super) fn publish_copy_fallback(
        &self,
        temporary: &Path,
        final_path: &Path,
    ) -> TextDocumentResult<()> {
        // 发布与淘汰在同一管理器锁内完成，防止并发 copy 互相删除刚发布的结果。
        let _guard = self
            .inner
            .copy_fallback_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "复制兜底锁已损坏"))?;
        std::fs::rename(temporary, final_path)?;
        if let Err(error) = cleanup_copy_fallbacks(&self.inner.root.join("tasks"), Some(final_path))
        {
            // 新结果已经原子发布；清理失败只能保留旧文件，不能把可用路径误报成失败。
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理旧复制兜底失败，保留已发布的新文件：{error}"),
            );
        }
        Ok(())
    }

    pub(super) fn copy_fallback_forced(&self) -> bool {
        #[cfg(test)]
        {
            return self.inner.force_copy_fallback.load(Ordering::Acquire);
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    #[cfg(test)]
    pub(crate) fn force_copy_fallback(&self, enabled: bool) {
        self.inner
            .force_copy_fallback
            .store(enabled, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_save_write_after(&self, bytes: u64) {
        self.inner
            .save_write_fail_after_bytes
            .store(bytes, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_save_baseline_prepare(&self) {
        self.inner
            .fail_save_baseline_prepare
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_save_state_prepare(&self) {
        self.inner
            .fail_save_state_prepare
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_save_journal_prune(&self) {
        self.inner
            .fail_save_journal_prune
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn cache_usage(&self, session_id: &str) -> TextDocumentResult<(usize, usize)> {
        self.session(session_id)?.chunk_cache.usage()
    }

    #[cfg(test)]
    pub(crate) fn added_len(&self, session_id: &str) -> TextDocumentResult<u64> {
        self.session(session_id)?.added.len()
    }

    #[cfg(test)]
    pub(crate) fn index_worker_starts(&self, session_id: &str) -> TextDocumentResult<u64> {
        Ok(self
            .session(session_id)?
            .index_worker_starts
            .load(Ordering::Acquire))
    }

    #[cfg(test)]
    pub(crate) fn index_build_starts(&self, session_id: &str) -> TextDocumentResult<u64> {
        Ok(self
            .session(session_id)?
            .index_build_starts
            .load(Ordering::Acquire))
    }

    #[cfg(test)]
    pub(crate) fn initial_index_build_starts(&self, session_id: &str) -> TextDocumentResult<u64> {
        Ok(self
            .session(session_id)?
            .initial_index_build_starts
            .load(Ordering::Acquire))
    }

    #[cfg(test)]
    pub(crate) fn index_revision(&self, session_id: &str) -> TextDocumentResult<u64> {
        let session = self.session(session_id)?;
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok(state.line_index.revision())
    }

    #[cfg(test)]
    pub(crate) fn index_is_exact(&self, session_id: &str) -> TextDocumentResult<bool> {
        let session = self.session(session_id)?;
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok(state.line_index.revision() == state.revision
            && state.line_index.line_ending_exact()
            && !state.index_exact_rebuild_required)
    }

    #[cfg(test)]
    pub(crate) fn set_index_chunk_delay(
        &self,
        session_id: &str,
        millis: u64,
    ) -> TextDocumentResult<()> {
        self.session(session_id)?
            .index_chunk_delay_millis
            .store(millis, Ordering::Release);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_validation_chunk_delay(&self, millis: u64) {
        self.inner
            .validation_delay_millis
            .store(millis, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn force_baseline_copy_fallback(&self, enabled: bool, delay_millis: u64) {
        self.inner
            .force_baseline_copy_fallback
            .store(enabled, Ordering::Release);
        self.inner
            .baseline_copy_chunk_delay_millis
            .store(delay_millis, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn baseline_ready(&self, session_id: &str) -> TextDocumentResult<bool> {
        Ok(self.session(session_id)?.metadata()?.baseline_ready)
    }

    #[cfg(test)]
    pub(crate) fn index_identity(&self, session_id: &str) -> TextDocumentResult<usize> {
        let session = self.session(session_id)?;
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        Ok(Arc::as_ptr(&state.line_index) as usize)
    }

    #[cfg(test)]
    pub(crate) fn force_unsupported_for_test(&self, session_id: &str) -> TextDocumentResult<()> {
        let session = self.session(session_id)?;
        let mut metadata = session
            .metadata
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档元数据锁已损坏"))?;
        metadata.encoding = TextEncoding::Unsupported;
        metadata.validation_complete = true;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn enable_save_test_pause(&self) {
        self.inner.save_test_pause.enable();
    }

    #[cfg(test)]
    pub(crate) fn wait_until_save_test_paused(&self) -> bool {
        self.inner.save_test_pause.wait_until_entered()
    }

    #[cfg(test)]
    pub(crate) fn release_save_test_pause(&self) {
        self.inner.save_test_pause.release();
    }

    #[cfg(test)]
    pub(crate) fn enable_open_probe_test_pause(&self) {
        self.inner.open_probe_test_pause.enable();
    }

    #[cfg(test)]
    pub(crate) fn wait_until_open_probe_test_paused(&self) -> bool {
        self.inner.open_probe_test_pause.wait_until_entered()
    }

    #[cfg(test)]
    pub(crate) fn release_open_probe_test_pause(&self) {
        self.inner.open_probe_test_pause.release();
    }

    #[cfg(test)]
    pub(crate) fn fail_reload_cleanup_for_test(&self) {
        self.inner
            .reload_cleanup_fail
            .store(true, Ordering::Release);
    }

    pub(super) fn commit_task_output(
        &self,
        session_id: &str,
        base_revision: u64,
        output_path: &Path,
        line_breaks: u64,
        undo_path: &Path,
        undo_line_breaks: u64,
        cancel: &AtomicBool,
    ) -> TextDocumentResult<TaskCommitState> {
        let session = self.session(session_id)?;
        let output_original = Arc::new(OriginalFile::open(output_path, 0)?);
        let undo_original = OriginalFile::open(undo_path, 0)?;
        let length = output_original.body_len();
        let history_weight = undo_original.body_len().saturating_add(length);
        let undo_replacement = JournalReplacementFile {
            path: undo_path.to_string_lossy().into_owned(),
            line_breaks: undo_line_breaks,
        };
        let redo_replacement = JournalReplacementFile {
            path: output_path.to_string_lossy().into_owned(),
            line_breaks,
        };
        let _lifecycle = session
            .lifecycle_guard
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
        session.ensure_writable()?;
        ensure_task_not_cancelled(cancel)?;
        {
            let state = session
                .state
                .read()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
            if state.revision != base_revision {
                return Err(TextDocumentError::revision_conflict(
                    base_revision,
                    state.revision,
                ));
            }
        }

        // 任务线程已在扫描期间同时产出前后快照；提交只交换 Piece 元数据，不在锁内复制全文。
        session.chunk_cache.clear()?;
        let mut journal = session
            .journal
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
        let (next_revision, persisted_revision, dirty) = {
            let mut state = session
                .state
                .write()
                .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
            if state.revision != base_revision {
                return Err(TextDocumentError::revision_conflict(
                    base_revision,
                    state.revision,
                ));
            }
            ensure_task_not_cancelled(cancel)?;
            state.history.push_transaction(
                HistoryOperation::Replacement(undo_replacement),
                HistoryOperation::Replacement(redo_replacement.clone()),
                history_weight,
            );
            state.piece_tree = PieceTree::from_original(length);
            state.original = output_original;
            state.revision += 1;
            invalidate_line_index_for_replacement(&mut state);
            journal.record(JournalRecord {
                base_revision,
                revision: state.revision,
                edits: Vec::new(),
                replacement_file: Some(redo_replacement),
            });
            cleanup_unreferenced_history_assets(&mut state, &journal);
            (
                state.revision,
                state.persisted_revision,
                state.revision != state.persisted_revision,
            )
        };
        drop(journal);
        schedule_journal_flush(session.clone());
        start_line_index(session.clone(), true, false);
        Ok(TaskCommitState {
            revision: next_revision,
            byte_length: length,
            persisted_revision,
            dirty,
        })
    }

    fn cancel_tasks_for_session(&self, session_id: &str) {
        match self.inner.tasks.lock() {
            Ok(tasks) => {
                for control in tasks.values() {
                    if control.session_id == session_id {
                        control.cancel.store(true, Ordering::Release);
                    }
                }
            }
            Err(error) => crate::app_logger::error(
                "SegmentedDocument",
                &format!("取消会话后台任务失败：{error}"),
            ),
        }
    }

    pub(super) fn session(&self, session_id: &str) -> TextDocumentResult<Arc<DocumentSession>> {
        self.inner
            .registry
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "会话表锁已损坏"))?
            .by_id
            .get(session_id)
            .cloned()
            .ok_or_else(|| TextDocumentError::new("session-not-found", "分段文档会话不存在"))
    }
}

fn read_window_from_snapshot(
    session: &DocumentSession,
    snapshot: &DocumentSnapshot,
    line_index: &LineIndex,
    requested_start: u64,
    target_bytes: usize,
    request_id: Option<u64>,
) -> TextDocumentResult<SegmentedWindow> {
    if target_bytes == 0 || target_bytes > MAX_WINDOW_BYTES {
        return Err(TextDocumentError::new(
            "invalid-window-size",
            format!("窗口大小必须在 1..={MAX_WINDOW_BYTES} 字节之间"),
        ));
    }
    let metadata = session.metadata()?;
    let use_lossy_mapping =
        !metadata.validation_complete || metadata.encoding == TextEncoding::Unsupported;
    let mut start = requested_start.min(snapshot.len());
    if !use_lossy_mapping {
        for _ in 0..3 {
            if start == 0
                || !snapshot
                    .byte_at(start)?
                    .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
            {
                break;
            }
            start -= 1;
        }
    }
    if start > 0
        && snapshot.byte_at(start - 1)? == Some(b'\r')
        && snapshot.byte_at(start)? == Some(b'\n')
    {
        start -= 1;
    }

    let mut end = (start + target_bytes.max(1) as u64).min(snapshot.len());
    if !use_lossy_mapping {
        for _ in 0..3 {
            if end >= snapshot.len()
                || !snapshot
                    .byte_at(end)?
                    .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
            {
                break;
            }
            end += 1;
        }
    }
    if end < snapshot.len()
        && end > 0
        && snapshot.byte_at(end - 1)? == Some(b'\r')
        && snapshot.byte_at(end)? == Some(b'\n')
    {
        end += 1;
    }

    let bytes = match session.chunk_cache.get(snapshot.revision, start, end)? {
        Some(bytes) => bytes,
        None => session.chunk_cache.insert(
            snapshot.revision,
            start,
            end,
            snapshot.read_range(start, (end - start) as usize)?,
        )?,
    };
    if bytes.len() != (end - start) as usize {
        return Err(TextDocumentError::new(
            "unexpected-eof",
            "读取窗口时正文提前结束",
        ));
    }
    let (text, utf16_byte_offsets) = if use_lossy_mapping {
        let (text, offsets) = encoding::decode_lossy_with_utf16_offsets(&bytes);
        (text, Some(offsets))
    } else {
        (
            std::str::from_utf8(&bytes)
                .map(str::to_string)
                .map_err(|error| {
                    TextDocumentError::new(
                        "invalid-utf8-window",
                        format!("窗口包含无效 UTF-8：{error}"),
                    )
                })?,
            None,
        )
    };
    let (start_line, active_line_index) =
        line_number_for_offset(session, snapshot, line_index, start)?;
    let leading_partial_line = !is_line_start(snapshot, start)?;
    let trailing_partial_line =
        end < snapshot.len() && end > start && !is_line_start(snapshot, end)?;
    let long_line =
        window_intersects_long_line(snapshot, start, end, &bytes, LONG_LINE_THRESHOLD_BYTES)?;
    let json_lexical_state = if session.document_kind == SegmentedDocumentKind::Json {
        active_line_index
            .json_checkpoint(start)
            .map(|(json_checkpoint, mut state)| {
                let mut cursor = json_checkpoint;
                while cursor < start {
                    let length = (start - cursor).min(256 * 1024) as usize;
                    let bytes = snapshot.read_range(cursor, length)?;
                    if bytes.is_empty() {
                        break;
                    }
                    advance_json_state(&mut state, &bytes);
                    cursor += bytes.len() as u64;
                }
                Ok::<_, TextDocumentError>(state)
            })
            .transpose()?
    } else {
        None
    };

    Ok(SegmentedWindow {
        request_id,
        revision: snapshot.revision,
        start_byte: start,
        end_byte: end,
        start_line,
        text,
        leading_partial_line,
        trailing_partial_line,
        long_line,
        index_progress: active_line_index.progress(),
        utf16_byte_offsets,
        json_lexical_state,
    })
}

fn line_number_for_offset(
    session: &DocumentSession,
    snapshot: &DocumentSnapshot,
    fallback_index: &LineIndex,
    target: u64,
) -> TextDocumentResult<(u64, Arc<LineIndex>)> {
    let _scan = session
        .line_scan_guard
        .lock()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "窗口行号补扫锁已损坏"))?;
    let active = session
        .state
        .read()
        .ok()
        .filter(|state| state.revision == snapshot.revision)
        .map(|state| state.line_index.clone())
        .unwrap_or_else(|| Arc::new(fallback_index.clone()));
    let (mut cursor, mut lines, mut previous_cr) = active.nearest_line_checkpoint(target);
    let mut checkpoints = Vec::new();
    let mut last_checkpoint = cursor;
    while cursor < target {
        let length = (target - cursor).min(256 * 1024) as usize;
        let bytes = snapshot.read_range(cursor, length)?;
        if bytes.is_empty() {
            break;
        }
        lines += encoding::advance_line_breaks(&bytes, &mut previous_cr);
        cursor += bytes.len() as u64;
        if cursor.saturating_sub(last_checkpoint) >= 1024 * 1024 || cursor == target {
            checkpoints.push((cursor, lines, previous_cr));
            last_checkpoint = cursor;
        }
    }
    // target 不会位于 CRLF 中间；若前缀尾部 CR 后不是 LF，它已构成完整 lone-CR 换行。
    if previous_cr && snapshot.byte_at(target)? != Some(b'\n') {
        lines += 1;
        if let Some(last) = checkpoints.last_mut().filter(|entry| entry.0 == target) {
            *last = (target, lines, false);
        } else {
            checkpoints.push((target, lines, false));
        }
    }
    drop(active);
    let published = {
        let mut state = session
            .state
            .write()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
        if state.revision == snapshot.revision
            && !state.line_index.is_completed()
            && !checkpoints.is_empty()
        {
            Arc::make_mut(&mut state.line_index).apply_line_checkpoints(&checkpoints);
        }
        if state.revision == snapshot.revision {
            state.line_index.clone()
        } else {
            Arc::new(fallback_index.clone())
        }
    };
    Ok((lines, published))
}

fn is_line_start(snapshot: &DocumentSnapshot, offset: u64) -> TextDocumentResult<bool> {
    if offset == 0 {
        return Ok(true);
    }
    Ok(match snapshot.byte_at(offset - 1)? {
        Some(b'\n') => true,
        Some(b'\r') => offset == snapshot.len() || snapshot.byte_at(offset)? != Some(b'\n'),
        _ => false,
    })
}

fn window_intersects_long_line(
    snapshot: &DocumentSnapshot,
    start: u64,
    end: u64,
    bytes: &[u8],
    threshold: usize,
) -> TextDocumentResult<bool> {
    let newlines: Vec<_> = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| matches!(*byte, b'\n' | b'\r').then_some(index))
        .collect();
    if newlines.is_empty() {
        if bytes.len() > threshold {
            return Ok(true);
        }
        let before = same_line_bytes_before(snapshot, start, threshold + 1)?;
        let after = same_line_bytes_after(snapshot, end, threshold + 1)?;
        return Ok(before.saturating_add(bytes.len()).saturating_add(after) > threshold);
    }
    let first = newlines[0];
    if first > threshold
        || same_line_bytes_before(snapshot, start, threshold + 1)?.saturating_add(first) > threshold
    {
        return Ok(true);
    }
    for pair in newlines.windows(2) {
        if pair[1].saturating_sub(pair[0] + 1) > threshold {
            return Ok(true);
        }
    }
    let trailing = bytes.len().saturating_sub(newlines[newlines.len() - 1] + 1);
    Ok(trailing > threshold
        || trailing.saturating_add(same_line_bytes_after(snapshot, end, threshold + 1)?)
            > threshold)
}

fn same_line_bytes_before(
    snapshot: &DocumentSnapshot,
    start: u64,
    limit: usize,
) -> TextDocumentResult<usize> {
    let mut cursor = start;
    let mut total = 0_usize;
    while cursor > 0 && total <= limit {
        let length = (cursor as usize).min(64 * 1024).min(limit + 1 - total);
        let chunk_start = cursor - length as u64;
        let chunk = snapshot.read_range(chunk_start, length)?;
        if let Some(position) = chunk
            .iter()
            .rposition(|byte| matches!(*byte, b'\n' | b'\r'))
        {
            return Ok(total + chunk.len().saturating_sub(position + 1));
        }
        total += chunk.len();
        cursor = chunk_start;
    }
    Ok(total)
}

fn same_line_bytes_after(
    snapshot: &DocumentSnapshot,
    end: u64,
    limit: usize,
) -> TextDocumentResult<usize> {
    let mut cursor = end;
    let mut total = 0_usize;
    while cursor < snapshot.len() && total <= limit {
        let length = ((snapshot.len() - cursor) as usize)
            .min(64 * 1024)
            .min(limit + 1 - total);
        let chunk = snapshot.read_range(cursor, length)?;
        if let Some(position) = chunk.iter().position(|byte| matches!(*byte, b'\n' | b'\r')) {
            return Ok(total + position);
        }
        total += chunk.len();
        cursor += chunk.len() as u64;
    }
    Ok(total)
}

fn update_line_index_after_edits(state: &mut SessionState, delta: LineIndexDelta) {
    state.index_exact_rebuild_required = true;
    if state.initial_index_pending {
        if !state.initial_index_requires_exact {
            let expected_base = state
                .pending_index_deltas
                .last()
                .map(|pending| pending.revision)
                .unwrap_or(delta.base_revision);
            if expected_base == delta.base_revision {
                state.pending_index_deltas.push(delta);
            } else {
                state.initial_index_requires_exact = true;
                state.pending_index_deltas.clear();
            }
        }
        state.line_index = Arc::new(LineIndex::pending(state.piece_tree.len(), state.revision));
        return;
    }

    match state.line_index.apply_delta(&delta) {
        Ok(index) => state.line_index = Arc::new(index),
        Err(error) => {
            // delta 不变量失败时显式降级为待重建，绝不继续暴露伪精确检查点。
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("合并行索引增量失败，等待精确重建：{error}"),
            );
            state.line_index = Arc::new(LineIndex::pending(state.piece_tree.len(), state.revision));
        }
    }
}

fn invalidate_line_index_for_replacement(state: &mut SessionState) {
    state.initial_index_requires_exact |= state.initial_index_pending;
    state.pending_index_deltas.clear();
    state.index_exact_rebuild_required = true;
    state.line_index = Arc::new(LineIndex::pending(state.piece_tree.len(), state.revision));
}

fn start_baseline_materialization(
    session: Arc<DocumentSession>,
    index_ready: bool,
    validation_ready: bool,
    persist_cache: bool,
) {
    std::thread::spawn(move || {
        let materialize =
            || -> TextDocumentResult<()> {
                if session.closing.load(Ordering::Acquire) {
                    return Ok(());
                }
                let source = session
                    .path
                    .lock()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "文档路径锁已损坏"))?
                    .clone();
                let expected = session
                    .baseline
                    .lock()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "文件基线锁已损坏"))?
                    .clone();
                #[cfg(test)]
                let delay = session.baseline_copy_chunk_delay_millis;
                #[cfg(not(test))]
                let delay = 0;
                let immutable = Arc::new(OriginalFile::copy_to_immutable(
                    &source,
                    session.initial_baseline_asset_path.clone(),
                    session.bom_len,
                    delay,
                )?);
                // 复制前后的完整身份必须一致；失败资产随 Arc Drop 清理，绝不发布混合快照。
                if BaselineIdentity::read(&source)? != expected {
                    return Err(TextDocumentError::new(
                        "file-changed-during-baseline-copy",
                        "后台建立不可变 baseline 时源文件发生变化",
                    ));
                }
                let _lifecycle = session
                    .lifecycle_guard
                    .lock()
                    .map_err(|_| TextDocumentError::new("lock-poisoned", "会话生命周期锁已损坏"))?;
                if session.closing.load(Ordering::Acquire) {
                    return Ok(());
                }
                let index = {
                    let mut state = session.state.write().map_err(|_| {
                        TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏")
                    })?;
                    state.original = immutable.clone();
                    if state.initial_index_pending {
                        *session.initial_index_snapshot.lock().map_err(|_| {
                            TextDocumentError::new("lock-poisoned", "初始索引快照锁已损坏")
                        })? = Some(DocumentSnapshot {
                            revision: state.revision,
                            tree: state.piece_tree.clone(),
                            original: immutable.clone(),
                            added: session.added.clone(),
                        });
                    }
                    state.line_index.clone()
                };
                *session.recovery_baseline.lock().map_err(|_| {
                    TextDocumentError::new("lock-poisoned", "恢复 baseline 锁已损坏")
                })? = immutable;
                let metadata = {
                    let mut metadata = session.metadata.lock().map_err(|_| {
                        TextDocumentError::new("lock-poisoned", "文档元数据锁已损坏")
                    })?;
                    metadata.baseline_ready = true;
                    *metadata
                };
                drop(_lifecycle);

                if !validation_ready {
                    start_encoding_validation(session.clone());
                }
                if !index_ready {
                    start_line_index(session.clone(), false, persist_cache);
                } else if validation_ready {
                    // 缓存索引无需再扫描，也要通知前端 baseline 门禁已解除。
                    emit_completed_index(&session, session.event_sink.as_ref(), &index, metadata);
                }
                Ok(())
            };
        if let Err(error) = materialize() {
            let message = error.to_string();
            if let Ok(mut baseline_error) = session.baseline_error.lock() {
                *baseline_error = Some(message.clone());
            }
            if let (Some(sink), Ok((snapshot, index))) =
                (session.event_sink.as_ref(), session.snapshot_with_index())
            {
                sink(super::SegmentedEvent::Index(super::IndexProgressEvent {
                    session_id: session.session_id.clone(),
                    task_id: format!("baseline-{}", session.session_id),
                    request_id: format!("baseline-{}", session.session_id),
                    revision: snapshot.revision,
                    indexed_bytes: index.indexed_bytes(),
                    total_bytes: snapshot.len(),
                    estimated_lines: index.estimated_lines(),
                    completed: false,
                    encoding: None,
                    line_ending: None,
                    readonly: Some(true),
                    baseline_error: Some(message.clone()),
                }));
            }
            crate::app_logger::error(
                "SegmentedDocument",
                &format!("后台建立不可变 baseline 失败：{message}"),
            );
        }
    });
}

fn start_encoding_validation(session: Arc<DocumentSession>) {
    let generation = session.validation_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let snapshot = match session.initial_index_snapshot.lock() {
        Ok(snapshot) => snapshot.clone(),
        Err(error) => {
            crate::app_logger::error(
                "SegmentedDocument",
                &format!("读取编码校验快照失败：{error}"),
            );
            return;
        }
    };
    let Some(snapshot) = snapshot else {
        crate::app_logger::error("SegmentedDocument", "编码校验缺少不可变初始快照");
        return;
    };
    std::thread::spawn(move || {
        let should_cancel = || {
            session.closing.load(Ordering::Acquire)
                || session.validation_generation.load(Ordering::Acquire) != generation
        };
        #[cfg(test)]
        let validation_delay_millis = session.validation_chunk_delay_millis;
        #[cfg(not(test))]
        let validation_delay_millis = 0;
        let result = validate_snapshot_utf8(&snapshot, &should_cancel, validation_delay_millis);
        let valid = match result {
            Ok(Some(valid)) => valid,
            Ok(None) => return,
            Err(error) => {
                crate::app_logger::error(
                    "SegmentedDocument",
                    &format!("后台 UTF-8 校验失败：{error}"),
                );
                return;
            }
        };
        if should_cancel() {
            return;
        }
        let state = match session.state.read() {
            Ok(state) if state.revision == snapshot.revision => state,
            Ok(_) => return,
            Err(error) => {
                crate::app_logger::error(
                    "SegmentedDocument",
                    &format!("提交 UTF-8 校验结果失败：{error}"),
                );
                return;
            }
        };
        let revision = state.revision;
        let index = state.line_index.clone();
        let metadata = match session.metadata.lock() {
            Ok(mut metadata) => {
                if metadata.validation_complete || should_cancel() {
                    return;
                }
                metadata.validation_complete = true;
                metadata.encoding = if valid {
                    if session.bom_len == encoding::UTF8_BOM.len() as u64 {
                        TextEncoding::Utf8Bom
                    } else {
                        TextEncoding::Utf8
                    }
                } else {
                    TextEncoding::Unsupported
                };
                *metadata
            }
            Err(error) => {
                crate::app_logger::error(
                    "SegmentedDocument",
                    &format!("更新 UTF-8 校验结果失败：{error}"),
                );
                return;
            }
        };
        drop(state);
        if let Some(sink) = &session.event_sink {
            sink(super::SegmentedEvent::Index(super::IndexProgressEvent {
                session_id: session.session_id.clone(),
                task_id: format!("validation-{}-{revision}", session.session_id),
                request_id: format!("validation-{}-{revision}", session.session_id),
                revision,
                indexed_bytes: session.index_reported_bytes.load(Ordering::Acquire),
                total_bytes: index.total_bytes(),
                estimated_lines: session.index_reported_lines.load(Ordering::Acquire),
                completed: index.is_completed(),
                encoding: Some(metadata.encoding),
                line_ending: None,
                readonly: Some(metadata.readonly()),
                baseline_error: None,
            }));
        }
    });
}

fn validate_snapshot_utf8(
    snapshot: &DocumentSnapshot,
    should_cancel: &dyn Fn() -> bool,
    delay_millis: u64,
) -> TextDocumentResult<Option<bool>> {
    let mut offset = 0_u64;
    let mut tail = Vec::with_capacity(4);
    while offset < snapshot.len() {
        if should_cancel() {
            return Ok(None);
        }
        if delay_millis > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_millis));
        }
        let bytes = snapshot.read_range(offset, VALIDATION_CHUNK_BYTES)?;
        if bytes.is_empty() {
            break;
        }
        let mut combined = Vec::with_capacity(tail.len() + bytes.len());
        combined.extend_from_slice(&tail);
        combined.extend_from_slice(&bytes);
        tail.clear();
        if let Err(error) = std::str::from_utf8(&combined) {
            if error.error_len().is_some() {
                return Ok(Some(false));
            }
            tail.extend_from_slice(&combined[error.valid_up_to()..]);
        }
        offset += bytes.len() as u64;
    }
    Ok(Some(tail.is_empty()))
}

fn start_line_index(session: Arc<DocumentSession>, debounce: bool, persist_cache: bool) {
    let generation = session.index_generation.fetch_add(1, Ordering::AcqRel) + 1;
    if persist_cache {
        session
            .index_cache_generation
            .store(generation, Ordering::Release);
    }
    let sink = session.event_sink.clone();
    if let Some(sink) = &sink {
        if let Ok((snapshot, index)) = session.snapshot_with_index() {
            if !index.is_completed() {
                sink(super::SegmentedEvent::Index(super::IndexProgressEvent {
                    session_id: session.session_id.clone(),
                    task_id: format!("index-{}-{}", session.session_id, snapshot.revision),
                    request_id: format!("index-{}-{}", session.session_id, snapshot.revision),
                    revision: snapshot.revision,
                    indexed_bytes: 0,
                    total_bytes: snapshot.len(),
                    estimated_lines: 0,
                    completed: false,
                    encoding: None,
                    line_ending: None,
                    readonly: None,
                    baseline_error: None,
                }));
            }
        }
    }
    if session
        .index_worker_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    #[cfg(test)]
    session.index_worker_starts.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || {
        let mut debounce_next = debounce;
        loop {
            let loop_generation = session.index_generation.load(Ordering::Acquire);
            let initial_pending = session
                .state
                .read()
                .map(|state| state.initial_index_pending)
                .unwrap_or(false);
            if initial_pending {
                match build_and_commit_initial_index(&session, sink.as_ref(), persist_cache) {
                    Ok(needs_exact) => {
                        if needs_exact {
                            debounce_next = true;
                            continue;
                        }
                    }
                    Err(error) => {
                        session.index_worker_running.store(false, Ordering::Release);
                        crate::app_logger::error(
                            "SegmentedDocument",
                            &format!("构建原始分段行索引失败：{error}"),
                        );
                        return;
                    }
                }
            } else {
                if debounce_next {
                    // 输入热路径只推进 generation；必须连续静默后才对最新快照做一次全文扫描。
                    std::thread::sleep(std::time::Duration::from_millis(INDEX_EDIT_IDLE_MILLIS));
                    if session.index_generation.load(Ordering::Acquire) != loop_generation {
                        continue;
                    }
                }
                debounce_next = true;
                let persist_cache =
                    session.index_cache_generation.load(Ordering::Acquire) == loop_generation;
                match build_and_commit_exact_index(
                    &session,
                    sink.as_ref(),
                    loop_generation,
                    persist_cache,
                ) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        if session.index_generation.load(Ordering::Acquire) != loop_generation {
                            continue;
                        }
                        session.index_worker_running.store(false, Ordering::Release);
                        crate::app_logger::error(
                            "SegmentedDocument",
                            &format!("构建分段行索引失败：{error}"),
                        );
                        return;
                    }
                }
            }

            session.index_worker_running.store(false, Ordering::Release);
            if session.index_generation.load(Ordering::Acquire) == loop_generation {
                return;
            }
            // store(false) 与新编辑调度之间存在窗口；CAS 保证至多一个 worker 接续。
            if session
                .index_worker_running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return;
            }
        }
    });
}

fn build_and_commit_initial_index(
    session: &Arc<DocumentSession>,
    sink: Option<&EventSink>,
    persist_cache: bool,
) -> TextDocumentResult<bool> {
    if session.closing.load(Ordering::Acquire) {
        return Ok(false);
    }
    let snapshot = session
        .initial_index_snapshot
        .lock()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "初始索引快照锁已损坏"))?
        .clone()
        .ok_or_else(|| TextDocumentError::new("initial-index-missing", "初始索引快照不存在"))?;
    #[cfg(test)]
    {
        session.index_build_starts.fetch_add(1, Ordering::Relaxed);
        session
            .initial_index_build_starts
            .fetch_add(1, Ordering::Relaxed);
    }
    // 原始扫描只受会话关闭影响；普通编辑通过 revision delta 合并，绝不取消这次不可变扫描。
    let should_cancel = || {
        #[cfg(test)]
        {
            let delay = session.index_chunk_delay_millis.load(Ordering::Relaxed);
            if delay > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        session.closing.load(Ordering::Acquire)
    };
    let tracked_sink = tracked_index_sink(session, sink);
    let publish_progress =
        |indexed_bytes: u64, estimated_lines: u64, checkpoint: LineIndexBuildCheckpoint| {
            let Ok(mut state) = session.state.write() else {
                return;
            };
            if !state.initial_index_pending || state.revision != snapshot.revision {
                return;
            }
            // 前台补扫检查点与后台 JSON 检查点共存；只增量更新本次稀疏节点，避免 O(n²) clone。
            Arc::make_mut(&mut state.line_index).apply_build_progress(
                indexed_bytes,
                estimated_lines,
                checkpoint,
            );
        };
    let Some(mut index) = LineIndex::build(
        &session.session_id,
        &snapshot,
        tracked_sink.as_ref(),
        Some(&publish_progress),
        &should_cancel,
    )?
    else {
        return Ok(false);
    };
    let (committed_index, needs_exact, can_persist) = {
        let mut state = session
            .state
            .write()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
        if !state.initial_index_pending {
            return Ok(state.index_exact_rebuild_required);
        }
        let latest_revision = state.revision;
        let had_edits = latest_revision != snapshot.revision;
        let mut merge_failed = state.initial_index_requires_exact;
        if !merge_failed {
            for delta in &state.pending_index_deltas {
                match index.apply_delta(delta) {
                    Ok(merged) => index = merged,
                    Err(error) => {
                        merge_failed = true;
                        crate::app_logger::warn(
                            "SegmentedDocument",
                            &format!("原始索引合并编辑 delta 失败：{error}"),
                        );
                        break;
                    }
                }
            }
            merge_failed |= index.revision() != latest_revision;
        }
        state.initial_index_pending = false;
        state.initial_index_requires_exact = false;
        state.pending_index_deltas.clear();
        state.index_exact_rebuild_required = had_edits || merge_failed;
        let committed_index = if merge_failed {
            state.line_index =
                Arc::new(LineIndex::pending(state.piece_tree.len(), latest_revision));
            None
        } else {
            let index = Arc::new(index);
            state.line_index = index.clone();
            Some(index)
        };
        let can_persist = persist_cache
            && !had_edits
            && !merge_failed
            && state.revision == state.persisted_revision;
        (
            committed_index,
            state.index_exact_rebuild_required,
            can_persist,
        )
    };
    if let Ok(mut initial_snapshot) = session.initial_index_snapshot.lock() {
        *initial_snapshot = None;
    }
    if let Some(index) = committed_index {
        let metadata = update_index_metadata(session, &index)?;
        if can_persist {
            persist_line_index(session, &index);
        }
        emit_completed_index(session, sink, &index, metadata);
    }
    Ok(needs_exact)
}

fn build_and_commit_exact_index(
    session: &Arc<DocumentSession>,
    sink: Option<&EventSink>,
    generation: u64,
    persist_cache: bool,
) -> TextDocumentResult<bool> {
    if session.closing.load(Ordering::Acquire) {
        return Ok(true);
    }
    let snapshot = session.snapshot()?;
    let should_cancel = || {
        #[cfg(test)]
        {
            let delay = session.index_chunk_delay_millis.load(Ordering::Relaxed);
            if delay > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        session.index_generation.load(Ordering::Acquire) != generation
    };
    #[cfg(test)]
    session.index_build_starts.fetch_add(1, Ordering::Relaxed);
    // delta 合并后的索引仍可服务读取；精确重建不再发送从 0 开始的进度以免 UI 回退。
    let Some(index) = LineIndex::build(&session.session_id, &snapshot, None, None, &should_cancel)?
    else {
        return Ok(false);
    };
    let index = Arc::new(index);
    let mut state = session
        .state
        .write()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话写锁已损坏"))?;
    if state.revision != snapshot.revision || should_cancel() {
        return Ok(false);
    }
    state.line_index = index.clone();
    state.initial_index_pending = false;
    state.pending_index_deltas.clear();
    state.index_exact_rebuild_required = false;
    let can_persist = persist_cache && state.revision == state.persisted_revision;
    drop(state);
    let metadata = update_index_metadata(session, &index)?;
    if can_persist {
        persist_line_index(session, &index);
    }
    emit_completed_index(session, sink, &index, metadata);
    Ok(true)
}

fn update_index_metadata(
    session: &DocumentSession,
    index: &LineIndex,
) -> TextDocumentResult<SessionMetadata> {
    let mut metadata = session
        .metadata
        .lock()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "文档元数据锁已损坏"))?;
    if index.line_ending_exact() {
        metadata.line_ending = index.line_ending();
    }
    Ok(*metadata)
}

fn persist_line_index(session: &DocumentSession, index: &LineIndex) {
    if let Err(error) = index.save_cache(&session.index_cache_path, &session.index_cache_key) {
        crate::app_logger::warn("SegmentedDocument", &format!("写入行索引缓存失败：{error}"));
    }
}

fn emit_completed_index(
    session: &DocumentSession,
    sink: Option<&EventSink>,
    index: &LineIndex,
    metadata: SessionMetadata,
) {
    session
        .index_reported_bytes
        .store(index.indexed_bytes(), Ordering::Release);
    session
        .index_reported_lines
        .store(index.estimated_lines(), Ordering::Release);
    if let Some(sink) = sink {
        sink(super::SegmentedEvent::Index(super::IndexProgressEvent {
            session_id: session.session_id.clone(),
            task_id: format!("index-{}-{}", session.session_id, index.revision()),
            request_id: format!("index-{}-{}", session.session_id, index.revision()),
            revision: index.revision(),
            indexed_bytes: index.indexed_bytes(),
            total_bytes: index.total_bytes(),
            estimated_lines: index.estimated_lines(),
            completed: true,
            encoding: Some(metadata.encoding),
            line_ending: index.line_ending_exact().then_some(metadata.line_ending),
            readonly: Some(metadata.readonly()),
            baseline_error: None,
        }));
    }
}

fn tracked_index_sink(
    session: &Arc<DocumentSession>,
    sink: Option<&EventSink>,
) -> Option<EventSink> {
    let sink = sink?.clone();
    let session = Arc::downgrade(session);
    Some(Arc::new(move |event| {
        if let (Some(session), super::SegmentedEvent::Index(progress)) = (session.upgrade(), &event)
        {
            session
                .index_reported_bytes
                .store(progress.indexed_bytes, Ordering::Release);
            session
                .index_reported_lines
                .store(progress.estimated_lines, Ordering::Release);
        }
        sink(event);
    }))
}

fn document_kind_from_path(path: &Path) -> TextDocumentResult<SegmentedDocumentKind> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("txt") => Ok(SegmentedDocumentKind::Text),
        Some("json") => Ok(SegmentedDocumentKind::Json),
        _ => Err(TextDocumentError::new(
            "unsupported-document-kind",
            "分段编辑器只允许打开 .txt 和 .json 文件",
        )),
    }
}

fn absolute_normalized_path(path: &Path) -> TextDocumentResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR.to_string()),
            std::path::Component::Normal(value) => normalized.push(value),
        }
    }
    Ok(normalized)
}

fn next_session_id() -> String {
    let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("seg-{nanos:x}-{sequence:x}")
}

fn prepare_history_operation(
    session: &DocumentSession,
    snapshot: &DocumentSnapshot,
    operation: &HistoryOperation,
    line_ending: LineEnding,
) -> TextDocumentResult<PreparedHistoryOperation> {
    match operation {
        HistoryOperation::Replacement(replacement) => {
            let original = Arc::new(OriginalFile::open(Path::new(&replacement.path), 0)?);
            Ok(PreparedHistoryOperation {
                tree: PieceTree::from_original(original.body_len()),
                original,
                journal_edits: Vec::new(),
                replacement_file: Some(replacement.clone()),
                index_edits: None,
            })
        }
        HistoryOperation::Edits(edits) => {
            let mut edits = edits.clone();
            edits.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
            validate_edit_batch(snapshot, &edits)?;
            let mut prepared = Vec::with_capacity(edits.len());
            let mut journal_edits = Vec::with_capacity(edits.len());
            let mut index_edits = Vec::with_capacity(edits.len());
            for edit in edits {
                let inserted = encoding::normalize_inserted_text(&edit.inserted_text, line_ending);
                let deleted = snapshot.read_range(
                    edit.from_byte,
                    edit.to_byte.saturating_sub(edit.from_byte) as usize,
                )?;
                let offset = if inserted.is_empty() {
                    None
                } else {
                    Some(session.added.append(&inserted)?)
                };
                journal_edits.push(SegmentedEdit {
                    from_byte: edit.from_byte,
                    to_byte: edit.to_byte,
                    inserted_text: String::from_utf8(inserted.clone()).map_err(|_| {
                        TextDocumentError::new("invalid-utf8", "历史补丁不是有效 UTF-8")
                    })?,
                });
                index_edits.push(LineIndexEdit {
                    from_byte: edit.from_byte,
                    to_byte: edit.to_byte,
                    inserted_bytes: inserted.len() as u64,
                    inserted_lines: encoding::count_line_breaks(&inserted),
                    deleted_lines: encoding::count_line_breaks(&deleted),
                });
                prepared.push((edit.from_byte, edit.to_byte, offset, inserted));
            }
            let mut tree = snapshot.tree.clone();
            for (from, to, offset, inserted) in prepared.into_iter().rev() {
                let piece = offset.map(|offset| {
                    Piece::added(
                        offset,
                        inserted.len() as u64,
                        encoding::count_line_breaks(&inserted),
                    )
                });
                tree = tree.replace_range(from, to, piece)?;
            }
            Ok(PreparedHistoryOperation {
                tree,
                original: snapshot.original.clone(),
                journal_edits,
                replacement_file: None,
                index_edits: Some(index_edits),
            })
        }
    }
}

fn validate_edit_transaction_size(edits: &[SegmentedEdit]) -> TextDocumentResult<()> {
    let bytes = edits.iter().try_fold(0_u64, |total, edit| {
        total
            .checked_add(edit.to_byte.saturating_sub(edit.from_byte))
            .and_then(|total| total.checked_add(edit.inserted_text.len() as u64))
    });
    if bytes.is_none_or(|bytes| bytes > MAX_EDIT_TRANSACTION_BYTES) {
        return Err(TextDocumentError::new(
            "edit-transaction-too-large",
            format!(
                "单次增量编辑不得超过 {MAX_EDIT_TRANSACTION_BYTES} 字节；大范围操作请使用后台任务"
            ),
        ));
    }
    Ok(())
}

fn validate_edit_batch(
    snapshot: &DocumentSnapshot,
    edits: &[SegmentedEdit],
) -> TextDocumentResult<()> {
    for edit in edits {
        if edit.from_byte > edit.to_byte || edit.to_byte > snapshot.len() {
            return Err(TextDocumentError::new(
                "invalid-edit-range",
                format!(
                    "编辑范围越界：{}..{}, len={}",
                    edit.from_byte,
                    edit.to_byte,
                    snapshot.len()
                ),
            ));
        }
        validate_text_boundary(snapshot, edit.from_byte)?;
        validate_text_boundary(snapshot, edit.to_byte)?;
    }
    for pair in edits.windows(2) {
        let same_insert_position = pair[0].from_byte == pair[0].to_byte
            && pair[1].from_byte == pair[1].to_byte
            && pair[0].from_byte == pair[1].from_byte;
        if pair[0].to_byte > pair[1].from_byte || same_insert_position {
            return Err(TextDocumentError::new(
                "overlapping-edits",
                "同一批编辑范围重叠或顺序不确定",
            ));
        }
    }
    Ok(())
}

fn validate_text_boundary(snapshot: &DocumentSnapshot, offset: u64) -> TextDocumentResult<()> {
    if offset == 0 || offset == snapshot.len() {
        return Ok(());
    }
    if snapshot
        .byte_at(offset)?
        .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
    {
        return Err(TextDocumentError::new(
            "utf8-boundary-split",
            format!("字节位置 {offset} 位于 UTF-8 字符内部"),
        ));
    }
    if snapshot.byte_at(offset - 1)? == Some(b'\r') && snapshot.byte_at(offset)? == Some(b'\n') {
        return Err(TextDocumentError::new(
            "crlf-boundary-split",
            format!("字节位置 {offset} 位于 CRLF 内部"),
        ));
    }
    Ok(())
}

fn schedule_journal_flush(session: Arc<DocumentSession>) {
    if session
        .journal_flush_scheduled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if session.closing.load(Ordering::Acquire) {
            session
                .journal_flush_scheduled
                .store(false, Ordering::Release);
            return;
        }
        match session.save_guard.lock() {
            Ok(_save_guard) => {
                if let Err(error) = flush_session_journal(&session, None) {
                    crate::app_logger::error(
                        "SegmentedDocument",
                        &format!("刷新恢复日志失败：{error}"),
                    );
                }
            }
            Err(error) => {
                crate::app_logger::error(
                    "SegmentedDocument",
                    &format!("获取恢复快照保存锁失败：{error}"),
                );
                session
                    .journal_flush_scheduled
                    .store(false, Ordering::Release);
            }
        }
    });
}

/// 在 save_guard 下把预先建立的不可变 baseline 交给 journal，再原子追加补丁记录。
/// 首次 durable flush 只同步 Added 与小型 journal，不再在 1 秒恢复窗口内复制全文。
fn flush_session_journal(
    session: &DocumentSession,
    expected_revision: Option<u64>,
) -> TextDocumentResult<u64> {
    let (revision, persisted_revision) = {
        let state = session
            .state
            .read()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "文档会话读锁已损坏"))?;
        (state.revision, state.persisted_revision)
    };
    if let Some(expected) = expected_revision {
        if expected != revision {
            return Err(TextDocumentError::revision_conflict(expected, revision));
        }
    }
    let recovery_baseline_target = {
        let journal = session
            .journal
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
        if !journal.has_records() {
            // 在 journal 锁内复位，保证随后 record 的编辑一定能重新调度。
            session
                .journal_flush_scheduled
                .store(false, Ordering::Release);
            return Ok(revision);
        }
        journal
            .needs_recovery_snapshot()
            .then(|| journal.recovery_snapshot_path())
    };

    if let Err(error) = session.added.sync() {
        reset_journal_flush_schedule(session);
        return Err(error);
    }
    let prepared_snapshot = if let Some(target) = recovery_baseline_target {
        let baseline = session
            .recovery_baseline
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复 baseline 锁已损坏"))?
            .clone();
        let source = match baseline.immutable_asset_path() {
            Ok(source) => source,
            Err(error) => {
                reset_journal_flush_schedule(session);
                return Err(error);
            }
        };
        if let Err(error) = publish_recovery_baseline(source, &target) {
            reset_journal_flush_schedule(session);
            return Err(error);
        }
        Some(RecoverySnapshot {
            path: target.to_string_lossy().into_owned(),
            revision: persisted_revision,
            bom_len: session.bom_len,
        })
    } else {
        None
    };

    let mut journal = session
        .journal
        .lock()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "恢复日志锁已损坏"))?;
    if let Some(snapshot) = prepared_snapshot {
        journal.install_recovery_snapshot(snapshot);
    }
    let flush_result = journal.flush();
    session
        .journal_flush_scheduled
        .store(false, Ordering::Release);
    flush_result?;
    Ok(revision)
}

fn publish_recovery_baseline(source: &Path, target: &Path) -> TextDocumentResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_file_if_exists(target)?;
    // source 与 recovery 都在同一 state root；硬链接只增加目录项，不扫描正文。
    std::fs::hard_link(source, target).map_err(|error| {
        TextDocumentError::new(
            "recovery-baseline-publish-failed",
            format!("发布不可变恢复 baseline 失败：{error}"),
        )
    })?;
    if let Some(parent) = target.parent() {
        sync_parent_dir(parent)?;
    }
    Ok(())
}

fn reset_journal_flush_schedule(session: &DocumentSession) {
    if let Ok(_journal) = session.journal.lock() {
        session
            .journal_flush_scheduled
            .store(false, Ordering::Release);
    } else {
        session
            .journal_flush_scheduled
            .store(false, Ordering::Release);
    }
}

fn ensure_baseline_unchanged(session: &DocumentSession, path: &Path) -> TextDocumentResult<()> {
    let expected = session
        .baseline
        .lock()
        .map_err(|_| TextDocumentError::new("lock-poisoned", "文件基线锁已损坏"))?
        .clone();
    let actual = BaselineIdentity::read(path)?;
    if actual != expected {
        return Err(TextDocumentError::new(
            "external-file-changed",
            "保存前检测到外部文件已变化",
        ));
    }
    Ok(())
}

fn ensure_open_identity_unchanged(
    path: &Path,
    probe_file: &File,
    expected: &BaselineIdentity,
) -> TextDocumentResult<()> {
    let handle_identity = BaselineIdentity::read_file(path, probe_file)?;
    let path_identity = BaselineIdentity::read(path)?;
    if handle_identity != *expected || path_identity != *expected {
        return Err(TextDocumentError::new(
            "file-changed-during-open",
            "读取首窗或建立不可变 baseline 期间源文件发生变化，请重试",
        ));
    }
    Ok(())
}

fn ensure_task_not_cancelled(cancel: &AtomicBool) -> TextDocumentResult<()> {
    if cancel.load(Ordering::Acquire) {
        Err(TextDocumentError::new("task-cancelled", "后台任务已取消"))
    } else {
        Ok(())
    }
}

fn normalize_save_target(path: &Path) -> TextDocumentResult<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let file_name = path.file_name().ok_or_else(|| {
                TextDocumentError::new("save-target-invalid", "保存目标缺少文件名")
            })?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let parent = parent.canonicalize().map_err(|error| {
                TextDocumentError::new(
                    "save-directory-missing",
                    format!("解析保存目录失败：{error}"),
                )
            })?;
            Ok(parent.join(file_name))
        }
        Err(error) => Err(TextDocumentError::new(
            "save-target-invalid",
            format!("解析保存目标失败：{error}"),
        )),
    }
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn cleanup_unreferenced_history_assets(state: &mut SessionState, journal: &EditJournal) {
    let candidates = state
        .history
        .cleanup_candidates(&journal.referenced_assets());
    let mut removed = Vec::new();
    for asset in candidates {
        match remove_file_if_exists(&asset) {
            Ok(()) => removed.push(asset),
            Err(error) => crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理已淘汰历史资产失败：{error}"),
            ),
        }
    }
    state.history.forget_assets(&removed);
}

fn replay_record(
    tree: &PieceTree,
    added: &AddedStore,
    record: &JournalRecord,
) -> TextDocumentResult<PieceTree> {
    if let Some(replacement) = &record.replacement_file {
        let mut file = File::open(&replacement.path).map_err(|error| {
            TextDocumentError::new(
                "journal-replacement-missing",
                format!("恢复任务结果失败：{error}"),
            )
        })?;
        let (offset, length) = added.append_reader(&mut file)?;
        return Ok(tree.replace_all_with_added(offset, length, replacement.line_breaks));
    }
    let mut next = tree.clone();
    let mut edits = record.edits.clone();
    edits.sort_by_key(|edit| (edit.from_byte, edit.to_byte));
    for edit in edits.into_iter().rev() {
        let bytes = edit.inserted_text.as_bytes();
        let piece = if bytes.is_empty() {
            None
        } else {
            let offset = added.append(bytes)?;
            Some(Piece::added(
                offset,
                bytes.len() as u64,
                encoding::count_line_breaks(bytes),
            ))
        };
        next = next.replace_range(edit.from_byte, edit.to_byte, piece)?;
    }
    Ok(next)
}

fn materialize_recovery_document(
    source_path: &Path,
    state_root: &Path,
    conflict: &RecoveryConflict,
) -> TextDocumentResult<PathBuf> {
    let recovery = conflict.snapshot.as_ref().ok_or_else(|| {
        TextDocumentError::new(
            "recovery-snapshot-missing",
            format!(
                "旧恢复日志缺少可物化快照，已保留在 {}",
                conflict.journal_path.display()
            ),
        )
    })?;
    let original = Arc::new(OriginalFile::open(
        Path::new(&recovery.path),
        recovery.bom_len,
    )?);
    let added = Arc::new(AddedStore::create(
        state_root
            .join("sessions")
            .join(format!("{}.recovery.added", next_session_id())),
    )?);
    let mut tree = PieceTree::from_original(original.body_len());
    let mut revision = recovery.revision;
    for record in conflict
        .records
        .iter()
        .filter(|record| record.revision > recovery.revision)
    {
        if record.base_revision != revision {
            return Err(TextDocumentError::new(
                "recovery-snapshot-revision-gap",
                "恢复快照后的 journal revision 不连续",
            ));
        }
        tree = replay_record(&tree, &added, record)?;
        revision = record.revision;
    }
    let recovered_snapshot = DocumentSnapshot {
        revision,
        tree,
        original,
        added,
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("txt");
    let preferred = source_path.parent().unwrap_or_else(|| Path::new("."));
    match write_recovered_document_to_directory(
        preferred,
        stem,
        extension,
        timestamp,
        &recovered_snapshot,
        recovery.bom_len,
        false,
    ) {
        Ok(path) => return Ok(path),
        Err(error) => crate::app_logger::warn(
            "SegmentedDocument",
            &format!("源目录无法写入恢复文档，改用应用目录：{error}"),
        ),
    }
    let fallback = state_root.join("recovered");
    write_recovered_document_to_directory(
        &fallback,
        stem,
        extension,
        timestamp,
        &recovered_snapshot,
        recovery.bom_len,
        false,
    )
}

fn write_recovered_document_to_directory(
    directory: &Path,
    stem: &str,
    extension: &str,
    timestamp: u128,
    snapshot: &DocumentSnapshot,
    bom_len: u64,
    force_hard_link_failure: bool,
) -> TextDocumentResult<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let temp_path = directory.join(format!(
        ".{stem}.recovered-{timestamp}-{}.tmp",
        std::process::id()
    ));
    remove_file_if_exists(&temp_path)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    let write_result = (|| -> TextDocumentResult<()> {
        if bom_len == encoding::UTF8_BOM.len() as u64 {
            output.write_all(encoding::UTF8_BOM)?;
        }
        let mut reader = snapshot.reader();
        std::io::copy(&mut reader, &mut output)?;
        output.sync_all()?;
        Ok(())
    })();
    drop(output);
    if let Err(error) = write_result {
        let _ = remove_file_if_exists(&temp_path);
        return Err(error);
    }
    for attempt in 0_u32..10_000 {
        let recovered_path = directory.join(format!(
            "{stem}.recovered-{timestamp}-{attempt}.{extension}"
        ));
        let hard_link_result = if force_hard_link_failure {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "test hard-link failure",
            ))
        } else {
            std::fs::hard_link(&temp_path, &recovered_path)
        };
        match hard_link_result {
            Ok(()) => {
                remove_file_if_exists(&temp_path)?;
                sync_parent_dir(directory)?;
                return Ok(recovered_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                let mut target = match OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&recovered_path)
                {
                    Ok(target) => target,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        let _ = remove_file_if_exists(&temp_path);
                        return Err(error.into());
                    }
                };
                let copy_result = (|| -> std::io::Result<()> {
                    let mut source = File::open(&temp_path)?;
                    std::io::copy(&mut source, &mut target)?;
                    target.sync_all()
                })();
                if let Err(error) = copy_result {
                    let _ = remove_file_if_exists(&recovered_path);
                    let _ = remove_file_if_exists(&temp_path);
                    return Err(error.into());
                }
                remove_file_if_exists(&temp_path)?;
                sync_parent_dir(directory)?;
                return Ok(recovered_path);
            }
        }
    }
    let _ = remove_file_if_exists(&temp_path);
    Err(TextDocumentError::new(
        "recovery-path-exhausted",
        "无法为恢复文档分配无覆盖文件名",
    ))
}

fn recovery_key(path: &Path) -> String {
    let normalized = path
        .canonicalize()
        .or_else(|_| {
            // 文件删除后仍以可解析的父目录恢复同一个绝对键，避免 macOS
            // `/var` 与 `/private/var` 这类符号链接前缀改变 recovery key。
            path.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .and_then(|parent| path.file_name().map(|name| parent.join(name)))
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "path has no canonicalizable parent",
                ))
        })
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let digest = Sha256::digest(normalized.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn baseline_cache_key(identity: &BaselineIdentity) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        identity.path,
        identity.physical_len,
        identity.modified_nanos,
        identity.file_id.as_deref().unwrap_or(""),
        identity.change_stamp.as_deref().unwrap_or("")
    )
}

fn external_change_token(
    change: SegmentedExternalChangeKind,
    current: Option<&BaselineIdentity>,
    baseline: &BaselineIdentity,
) -> String {
    let kind = match change {
        SegmentedExternalChangeKind::None => "none",
        SegmentedExternalChangeKind::Modified => "modified",
        SegmentedExternalChangeKind::Deleted => "deleted",
    };
    let identity = current.unwrap_or(baseline);
    let payload = format!(
        "{kind}\0{}\0{}\0{}\0{}\0{}",
        identity.path,
        identity.physical_len,
        identity.modified_nanos,
        identity.file_id.as_deref().unwrap_or(""),
        identity.change_stamp.as_deref().unwrap_or("")
    );
    Sha256::digest(payload.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn cleanup_copy_fallbacks(directory: &Path, keep: Option<&Path>) -> TextDocumentResult<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if keep.is_some_and(|keep| keep == path)
            || path.extension().and_then(|extension| extension.to_str()) != Some("txt")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn cleanup_session_transients(directory: &Path) -> TextDocumentResult<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            // durable recovery baseline 已硬链接到 recovery；sessions 目录只含崩溃遗留临时资产。
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn copy_snapshot_for_save<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    fail_after_bytes: Option<u64>,
) -> std::io::Result<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut written = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(written);
        }
        let allowed = fail_after_bytes
            .map(|limit| limit.saturating_sub(written).min(count as u64) as usize)
            .unwrap_or(count);
        if allowed > 0 {
            writer.write_all(&buffer[..allowed])?;
            written = written.saturating_add(allowed as u64);
        }
        if allowed < count {
            // 测试在临时文件写阶段精确注入 ENOSPC；目标文件尚未替换，因此真实失败语义不被绕过。
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No space left on device (test injection)",
            ));
        }
    }
}

fn unique_temp_path(parent: &Path, target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("segmented-document");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce))
}

#[cfg(target_os = "windows")]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let mut from: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    from.push(0);
    let mut to: Vec<u16> = target_path.as_os_str().encode_wide().collect();
    to.push(0);
    // SAFETY: 两个 UTF-16 缓冲区都显式补了 NUL，且在调用期间保持存活；flags 只要求同步替换。
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    std::fs::rename(temp_path, target_path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_dir(parent: &Path) -> TextDocumentResult<()> {
    File::open(parent)?.sync_all()?;
    Ok(())
}

struct SaveProgressGuard<'a>(&'a AtomicBool);

impl Drop for SaveProgressGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_parent_dir(_parent: &Path) -> TextDocumentResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_latest_history_transaction_remains_undoable() {
        let mut history = HistoryState::default();

        history.push_transaction(
            HistoryOperation::Edits(Vec::new()),
            HistoryOperation::Edits(Vec::new()),
            HISTORY_CAPACITY_BYTES + 1,
        );

        assert!(history.can_undo());
        assert_eq!(history.undo.len(), 1);
    }

    #[test]
    fn recovered_document_falls_back_when_hard_link_is_unavailable() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nomo-recovery-copy-{nonce}"));
        std::fs::create_dir_all(&root).expect("root");
        let source = root.join("source.txt");
        std::fs::write(&source, "base local").expect("source");
        let original = Arc::new(OriginalFile::open(&source, 0).expect("original"));
        let added = Arc::new(AddedStore::create(root.join("added")).expect("added"));
        let snapshot = DocumentSnapshot {
            revision: 1,
            tree: PieceTree::from_original(original.body_len()),
            original,
            added,
        };

        let recovered = write_recovered_document_to_directory(
            &root,
            "sample",
            "txt",
            nonce / 1_000_000,
            &snapshot,
            0,
            true,
        )
        .expect("copy fallback");

        assert_eq!(
            std::fs::read_to_string(recovered).expect("recovered"),
            "base local"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
