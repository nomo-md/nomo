use super::{
    invalidation::Invalidation,
    parser::{Batch, Block},
    scanner::Scanner,
    source_anchors::SourceAnchors,
    store::{ChangeScan, Identity, Query, Response, SlugScan, Store},
};
use crate::text_document::{chunk_reader::DocumentSnapshot, TextDocumentError, TextDocumentResult};
use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Condvar, Mutex,
    },
};

static NEXT: AtomicU64 = AtomicU64::new(1);
fn lock<T>(value: &Mutex<T>) -> TextDocumentResult<std::sync::MutexGuard<'_, T>> {
    value
        .lock()
        .map_err(|_| TextDocumentError::new("structure-lock-poisoned", "结构索引锁已损坏"))
}
struct Scan {
    scanner: Scanner,
    read: u64,
    scanned_bytes: u64,
    reuse: Option<Reuse>,
    calibration: Option<u64>,
    changes: Option<ChangeScan>,
    slugs: Option<SlugScan>,
}
struct Reuse {
    previous: Arc<Job>,
    invalidation: Invalidation,
    checkpoint: Block,
    anchors: Option<SourceAnchors>,
}
struct Tail {
    previous: Identity,
    from: u64,
    to: u64,
    bytes: i64,
    lines: i64,
}
enum Work {
    Scan(Batch, Option<Tail>),
    Calibrate(u64),
    Changes,
}
struct Job {
    owner: String,
    identity: Identity,
    snapshot: DocumentSnapshot,
    cancel: Arc<AtomicBool>,
    active: AtomicBool,
    priority: AtomicU8,
    scan: Mutex<Scan>,
    // 读取租约保证关闭期间快照依赖的会话临时文件仍然有效。
    _lease: Box<dyn Any + Send + Sync>,
}
#[derive(Default)]
struct State {
    jobs: HashMap<u64, Arc<Job>>,
    queue: VecDeque<Arc<Job>>,
    running: bool,
    published: HashMap<String, u64>,
    building: HashMap<String, u64>,
    invalidations: HashMap<String, Option<Invalidation>>,
    // 每个活动会话仅保留一个修订号；关闭/移交时释放，旧快照不能倒退索引。
    revisions: HashMap<String, u64>,
}
struct Inner {
    store: Mutex<Store>,
    state: Mutex<State>,
    // 独立于调度/数据库锁，编辑和关闭可中断正在执行的 SQL。
    executing: Mutex<Option<Arc<Job>>>,
    foreground: Mutex<usize>,
    foreground_done: Condvar,
}
struct Foreground<'a>(&'a Inner);
impl Drop for Foreground<'_> {
    fn drop(&mut self) {
        if let Ok(mut count) = self.0.foreground.lock() {
            *count -= 1;
            self.0.foreground_done.notify_one();
        }
    }
}
#[derive(Clone)]
pub struct StructureService {
    inner: Arc<Inner>,
}
impl StructureService {
    pub fn toc_baseline(
        &self,
        owner: &str,
        identity: &Identity,
        data: Option<&str>,
    ) -> TextDocumentResult<Option<String>> {
        let state = lock(&self.inner.state)?;
        if !state
            .jobs
            .get(&identity.index_generation)
            .is_some_and(|job| {
                job.owner == owner
                    && job.identity == *identity
                    && !job.cancel.load(Ordering::Acquire)
            })
        {
            return Err(TextDocumentError::new(
                "structure-generation-expired",
                "目录索引版本已失效",
            ));
        }
        lock(&self.inner.store)?.toc_baseline(&identity.document_id, data)
    }
    pub fn close_outline(&self, document_id: &str, view_id: &str) -> TextDocumentResult<()> {
        lock(&self.inner.store)?.close_outline(document_id, view_id)
    }
    pub fn new(root: &std::path::Path) -> TextDocumentResult<Self> {
        let name = format!(
            "structure-{}-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path: PathBuf = root.join(name);
        Ok(Self {
            inner: Arc::new(Inner {
                store: Mutex::new(Store::lazy(path)),
                state: Mutex::new(State::default()),
                executing: Mutex::new(None),
                foreground: Mutex::new(0),
                foreground_done: Condvar::new(),
            }),
        })
    }
    pub fn request(
        &self,
        owner: &str,
        document_id: &str,
        snapshot: DocumentSnapshot,
        lease: Box<dyn Any + Send + Sync>,
    ) -> TextDocumentResult<Identity> {
        *lock(&self.inner.foreground)? += 1;
        let _foreground = Foreground(&self.inner);
        self.cancel_executing(document_id, Some(snapshot.revision));
        let mut state = lock(&self.inner.state)?;
        if let Some(&current) = state.revisions.get(document_id) {
            if snapshot.revision < current {
                return Err(TextDocumentError::revision_conflict(snapshot.revision, current));
            }
        }
        state.revisions.insert(document_id.into(), snapshot.revision);
        for generation in [
            state.building.get(document_id),
            state.published.get(document_id),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(job) = state.jobs.get(generation) {
                if job.owner == owner
                    && job.identity.revision == snapshot.revision
                    && !job.cancel.load(Ordering::Acquire)
                    && lock(&self.inner.store)?
                        .query(&job.identity, Query::Progress)?
                        .state
                        != "failed"
                {
                    return Ok(job.identity.clone());
                }
            }
        }
        if let Some(old) = state.building.remove(document_id) {
            if let Some(job) = state.jobs.remove(&old) {
                job.cancel.store(true, Ordering::Release);
            }
            state
                .queue
                .retain(|job| job.identity.index_generation != old);
            lock(&self.inner.store)?.remove(old)?;
        }
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let identity = Identity::new(document_id.into(), snapshot.revision, generation);
        lock(&self.inner.store)?.create(&identity, snapshot.len())?;
        let previous = state
            .published
            .get(document_id)
            .and_then(|generation| state.jobs.get(generation))
            .filter(|job| job.owner == owner)
            .cloned();
        let invalidation = state.invalidations.get(document_id).copied().flatten();
        let reuse = if let (Some(previous), Some(invalidation)) = (previous, invalidation) {
            let store = lock(&self.inner.store)?;
            if let Some(checkpoint) =
                store.block_at(&previous.identity, invalidation.from.saturating_sub(1))?
            {
                store.reuse_range(&identity, &previous.identity, 0, checkpoint.from_byte, 0, 0)?;
                store.publish(&identity, &[], checkpoint.from_byte, false)?;
                Some(Reuse {
                    anchors: (invalidation.to > checkpoint.to_byte)
                        .then(|| SourceAnchors::new(&previous.snapshot.tree)).flatten(),
                    previous,
                    invalidation,
                    checkpoint,
                })
            } else {
                None
            }
        } else {
            None
        };
        let from = reuse
            .as_ref()
            .map(|reuse| reuse.checkpoint.from_byte)
            .unwrap_or(0);
        let line = reuse
            .as_ref()
            .map(|reuse| reuse.checkpoint.from_line)
            .unwrap_or(1);
        let job = Arc::new(Job {
            owner: owner.into(),
            identity: identity.clone(),
            scan: Mutex::new(Scan {
                scanner: Scanner::new(snapshot.revision, from, line),
                read: from,
                scanned_bytes: 0,
                reuse,
                calibration: None,
                changes: None,
                slugs: None,
            }),
            snapshot,
            cancel: Arc::new(AtomicBool::new(false)),
            active: AtomicBool::new(true),
            priority: AtomicU8::new(1),
            _lease: lease,
        });
        state.jobs.insert(generation, job.clone());
        state.building.insert(document_id.into(), generation);
        state.queue.push_back(job);
        start_worker(&self.inner, &mut state)?;
        Ok(identity)
    }
    pub fn demand(
        &self,
        owner: &str,
        identity: &Identity,
        active: bool,
        priority: u8,
    ) -> TextDocumentResult<()> {
        *lock(&self.inner.foreground)? += 1;
        let _foreground = Foreground(&self.inner);
        let mut state = lock(&self.inner.state)?;
        let job = state
            .jobs
            .get(&identity.index_generation)
            .filter(|job| job.owner == owner && job.identity == *identity)
            .cloned()
            .ok_or_else(|| {
                TextDocumentError::new("structure-generation-expired", "结构需求版本已失效")
            })?;
        job.active.store(active, Ordering::Release);
        job.priority.store(priority.min(2), Ordering::Release);
        state.queue.retain(|queued| queued.identity != *identity);
        if active && state.building.get(&identity.document_id) == Some(&identity.index_generation) {
            state.queue.push_back(job);
            start_worker(&self.inner, &mut state)?;
        }
        Ok(())
    }
    pub fn query(
        &self,
        owner: &str,
        identity: &Identity,
        query: Query,
    ) -> TextDocumentResult<Response> {
        *lock(&self.inner.foreground)? += 1;
        let _foreground = Foreground(&self.inner);
        let state = lock(&self.inner.state)?;
        let job = state
            .jobs
            .get(&identity.index_generation)
            .filter(|job| job.owner == owner && job.identity == *identity)
            .ok_or_else(|| {
                TextDocumentError::new(
                    "structure-generation-expired",
                    "结构生成版本已失效或归属已改变",
                )
            })?;
        if job.cancel.load(Ordering::Acquire) {
            return Err(TextDocumentError::new(
                "structure-cancelled",
                "结构生成已取消",
            ));
        }
        lock(&self.inner.store)?.query(identity, query)
    }
    /// 编辑只取消构建；最后已发布版本仍可供界面保留。关闭/移交同时释放所有版本。
    pub fn invalidate(&self, document_id: &str, release: bool) {
        self.cancel_executing(document_id, None);
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        self.invalidate_locked(&mut state, document_id, release);
    }
    fn invalidate_locked(&self, state: &mut State, document_id: &str, release: bool) {
        let mut generations = vec![];
        if release {
            state.revisions.remove(document_id);
            state.invalidations.remove(document_id);
            if let Ok(store) = self.inner.store.lock() {
                if let Err(error) = store.release_views(document_id) {
                    crate::app_logger::warn("StructureIndex", &error.message);
                }
            }
        } else {
            state.invalidations.insert(document_id.into(), None);
        }
        if let Some(generation) = state.building.remove(document_id) {
            generations.push(generation);
        }
        if release {
            if let Some(generation) = state.published.remove(document_id) {
                generations.push(generation);
            }
        }
        for generation in generations {
            if let Some(job) = state.jobs.remove(&generation) {
                job.cancel.store(true, Ordering::Release);
            }
            state
                .queue
                .retain(|job| job.identity.index_generation != generation);
            if let Ok(store) = self.inner.store.lock() {
                if let Err(error) = store.remove(generation) {
                    crate::app_logger::warn("StructureIndex", &error.message);
                }
            }
        }
    }
    fn cancel_executing(&self, document: &str, keep_revision: Option<u64>) {
        if let Ok(executing) = self.inner.executing.lock() {
            if let Some(job) = executing.as_ref().filter(|job| job.identity.document_id == document
                && keep_revision.is_none_or(|revision| job.identity.revision < revision)) {
                job.cancel.store(true, Ordering::Release);
            }
        }
    }
    pub fn edited(&self, document_id: &str, revision: u64, edits: &[(u64, u64, u64)]) {
        self.cancel_executing(document_id, Some(revision));
        if let Ok(mut state) = self.inner.state.lock() {
            if state.revisions.get(document_id).is_some_and(|current| *current > revision) { return; }
            let previous = state.invalidations.get(document_id).copied();
            // 修订门禁、取消与变更区间在同一锁内发布，避免旧请求消费半更新状态。
            state.revisions.insert(document_id.into(), revision);
            self.invalidate_locked(&mut state, document_id, false);
            let merged = if edits.is_empty() || previous == Some(None) {
                None
            } else {
                Invalidation::merge(previous.flatten(), edits)
            };
            state.invalidations.insert(document_id.into(), merged);
        }
    }
    #[cfg(test)]
    pub fn scanned_bytes(&self, identity: &Identity) -> u64 {
        let state = self.inner.state.lock().unwrap();
        let bytes = state.jobs[&identity.index_generation]
            .scan
            .lock()
            .unwrap()
            .scanned_bytes;
        bytes
    }
}
fn start_worker(inner: &Arc<Inner>, state: &mut State) -> TextDocumentResult<()> {
    if state.running || state.queue.is_empty() {
        return Ok(());
    }
    state.running = true;
    let weak = Arc::downgrade(inner);
    if let Err(error) = std::thread::Builder::new()
        .name("nomo-structure".into())
        .spawn(move || worker(weak))
    {
        state.running = false;
        for job in state.queue.drain(..) {
            lock(&inner.store)?.fail(job.identity.index_generation, &error.to_string())?;
        }
        return Err(TextDocumentError::new(
            "structure-worker-start",
            error.to_string(),
        ));
    }
    Ok(())
}
fn step(job: &Job, inner: &Inner) -> TextDocumentResult<Work> {
    let mut scan = lock(&job.scan)?;
    if scan.changes.is_some() { return Ok(Work::Changes); }
    if let Some(cursor) = scan.calibration {
        return Ok(Work::Calibrate(cursor));
    }
    let mut bytes = job.snapshot.read_range(
        scan.read,
        16384.min((job.snapshot.len() - scan.read) as usize),
    )?;
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => {
            return Err(TextDocumentError::new(
                "structure-utf8",
                "Markdown 快照包含无效 UTF-8",
            ))
        }
    };
    bytes.truncate(valid);
    if valid == 0 && scan.read < job.snapshot.len() {
        return Err(TextDocumentError::new("structure-read", "结构扫描未推进"));
    }
    scan.read += valid as u64;
    scan.scanned_bytes += valid as u64;
    let eof = scan.read == job.snapshot.len();
    let mut batch = scan
        .scanner
        .push(std::str::from_utf8(&bytes).unwrap(), eof)
        .map_err(|message| TextDocumentError::new("structure-parse", message))?;
    let mut tail = None;
    if let Some(reuse) = &scan.reuse {
        let changed = reuse.invalidation;
        let mut through = batch.blocks.len();
        for (index, block) in batch.blocks.iter_mut().enumerate() {
            if block.mode == "semantic" && block.from_byte < block.to_byte {
                if let Some(anchors) = &reuse.anchors {
                    if let (Some(from), Some(last)) = (
                        anchors.original_position(&job.snapshot.tree, block.from_byte),
                        anchors.original_position(&job.snapshot.tree, block.to_byte - 1),
                    ) {
                        if let Some(old) = lock(&inner.store)?.block_at(&reuse.previous.identity, from)? {
                            // 首尾来源均保留，且没有拆分/合并，才能沿用编辑块身份。
                            // 多标题容器内部的身份不作顺序猜测。
                            if old.mode == "semantic" && old.kind == block.kind
                                && old.from_byte == from && old.to_byte == last + 1
                                && old.headings.len() == block.headings.len() && block.headings.len() <= 1
                                && block.headings.iter().zip(&old.headings).all(|(new, old)|
                                    anchors.original_position(&job.snapshot.tree, new.byte) == Some(old.byte)) {
                                block.id = old.id;
                            }
                        }
                    }
                }
            }
            // 安全检查点可以早于实际编辑块（目录更新常回扫前一个标题）。
            // 这些前缀块仍需重解析以检查边界，但不能因此丢失身份和折叠状态。
            if block.mode == "semantic" && block.to_byte <= changed.from {
                if let Some(old) = lock(&inner.store)?.block_at(&reuse.previous.identity, block.from_byte)? {
                    if old.mode == "semantic" && old.kind == block.kind
                        && old.from_byte == block.from_byte && old.to_byte == block.to_byte
                        && old.source_hash == block.source_hash
                        && reuse.previous.snapshot.read_range(old.from_byte, (old.to_byte - old.from_byte) as usize)?
                            == job.snapshot.read_range(block.from_byte, (block.to_byte - block.from_byte) as usize)?
                    {
                        block.id = old.id;
                        block.content_version = old.content_version;
                    }
                }
                continue;
            }
            let old = if block.from_byte <= changed.from {
                lock(&inner.store)?.block_at(&reuse.previous.identity, block.from_byte)?
            } else { None };
            if block.mode == "semantic"
                && old.as_ref().is_some_and(|old| old.mode == "semantic"
                    && block.from_byte == old.from_byte
                    && block.to_byte == old.to_byte.saturating_add_signed(changed.delta)
                    && block.kind == old.kind
                    && changed.from >= old.from_byte
                    && changed.to <= old.to_byte)
            {
                block.id = old.unwrap().id;
            }
            if block.mode != "semantic"
                || block.from_byte < changed.to.saturating_add_signed(changed.delta)
            {
                continue;
            }
            let old_byte = block.from_byte.saturating_add_signed(-changed.delta);
            let Some(old) = lock(&inner.store)?.block_at(&reuse.previous.identity, old_byte)?
            else {
                continue;
            };
            if old.mode != "semantic"
                || old.kind != block.kind
                || old.source_hash != block.source_hash
                || old.from_byte != old_byte
                || old.to_byte.saturating_add_signed(changed.delta) != block.to_byte
            {
                continue;
            }
            let before = reuse
                .previous
                .snapshot
                .read_range(old.from_byte, (old.to_byte - old.from_byte) as usize)?;
            let after = job
                .snapshot
                .read_range(block.from_byte, (block.to_byte - block.from_byte) as usize)?;
            if before != after {
                continue;
            }
            block.id = old.id;
            block.content_version = old.content_version;
            tail = Some(Tail {
                previous: reuse.previous.identity.clone(),
                from: old.to_byte,
                to: reuse.previous.snapshot.len(),
                bytes: changed.delta,
                lines: block.to_line as i64 - old.to_line as i64,
            });
            through = index + 1;
            break;
        }
        batch.blocks.truncate(through);
    }
    if tail.is_some() {
        batch.consumed_bytes = job.snapshot.len();
        batch.complete = true;
    }
    Ok(Work::Scan(batch, tail))
}
fn worker(weak: std::sync::Weak<Inner>) {
    while let Some(inner) = weak.upgrade() {
        // std::Mutex 不保证公平；连续后台批次不能抢在已等待的正文查询前重新占锁。
        let Ok(mut foreground) = inner.foreground.lock() else { return; };
        while *foreground > 0 {
            foreground = match inner.foreground_done.wait(foreground) { Ok(value) => value, Err(_) => return };
        }
        drop(foreground);
        let job = {
            let Ok(mut state) = inner.state.lock() else {
                return;
            };
            let priority = state
                .queue
                .iter()
                .map(|job| job.priority.load(Ordering::Acquire))
                .max();
            let selected = priority.and_then(|priority| {
                state
                    .queue
                    .iter()
                    .position(|job| job.priority.load(Ordering::Acquire) == priority)
            });
            let Some(job) = selected.and_then(|index| state.queue.remove(index)) else {
                state.running = false;
                return;
            };
            job
        };
        if job.cancel.load(Ordering::Acquire) || !job.active.load(Ordering::Acquire) {
            continue;
        }
        let Ok(mut executing) = inner.executing.lock() else { return; };
        *executing = Some(job.clone());
        drop(executing);
        struct Execution<'a>(&'a Mutex<Option<Arc<Job>>>);
        impl Drop for Execution<'_> {
            fn drop(&mut self) { if let Ok(mut job) = self.0.lock() { *job = None; } }
        }
        let _execution = Execution(&inner.executing);
        let batch = step(&job, &inner);
        // state -> store 是统一锁顺序；取消与发布不能交错留下无主派生页。
        let Ok(mut state) = inner.state.lock() else {
            return;
        };
        if job.cancel.load(Ordering::Acquire) {
            continue;
        }
        let Ok(store) = inner.store.lock() else {
            state.running = false;
            return;
        };
        let result = (|| {
            let _cancellation = store.cancellable(job.cancel.clone())?;
            batch.and_then(|work| match work {
            Work::Scan(batch, tail) => {
                store.publish(&job.identity, &batch.blocks, batch.consumed_bytes, false)?;
                if batch.complete {
                    let mut scan=lock(&job.scan)?;
                    if let Some(reuse)=&scan.reuse {
                        let old_end=tail.as_ref().map_or(reuse.previous.snapshot.len(),|tail|tail.from);
                        let new_end=tail.as_ref().map_or(job.snapshot.len(),|tail|tail.from.saturating_add_signed(tail.bytes));
                        scan.slugs=Some(SlugScan::new(reuse.previous.identity.clone(),reuse.checkpoint.from_byte,old_end,new_end));
                    }
                }
                if let Some(tail) = tail {
                    store.reuse_range(
                        &job.identity,
                        &tail.previous,
                        tail.from,
                        tail.to,
                        tail.bytes,
                        tail.lines,
                    )?;
                }
                if batch.complete {
                    lock(&job.scan)?.calibration = Some(0);
                }
                Ok(false)
            }
            Work::Calibrate(cursor) => {
                let mut scan=lock(&job.scan)?;
                let next = if let Some(slugs)=&mut scan.slugs {
                    if store.calibrate_incremental(&job.identity,slugs)? {None} else {Some(cursor)}
                } else {store.calibrate(&job.identity, cursor)?};
                scan.calibration = next;
                if next.is_none() {
                    scan.changes = Some(ChangeScan::default());
                }
                Ok(false)
            }
            Work::Changes => {
                let previous = state.published.get(&job.identity.document_id).and_then(|id| state.jobs.get(id));
                let complete = store.record_changes_step(&job.identity, previous.map(|job| &job.identity),
                    lock(&job.scan)?.changes.as_mut().unwrap())?;
                if complete && !job.cancel.load(Ordering::Acquire) {
                    store.publish(&job.identity, &[], job.snapshot.len(), true)?;
                    lock(&job.scan)?.reuse = None;
                    lock(&job.scan)?.changes = None;
                }
                Ok(complete)
            }
        })})();
        if job.cancel.load(Ordering::Acquire) { continue; }
        match result {
            Ok(true) => {
                state.queue.retain(|queued| queued.identity != job.identity);
                state.building.remove(&job.identity.document_id);
                state.invalidations.remove(&job.identity.document_id);
                if let Some(old) = state.published.insert(
                    job.identity.document_id.clone(),
                    job.identity.index_generation,
                ) {
                    state.jobs.remove(&old);
                    if let Err(error) = store.remove(old) {
                        crate::app_logger::warn("StructureIndex", &error.message);
                    }
                }
            }
            Ok(false) => {
                if job.active.load(Ordering::Acquire)
                    && !state
                        .queue
                        .iter()
                        .any(|queued| queued.identity == job.identity)
                {
                    state.queue.push_back(job);
                }
            }
            Err(error) => {
                state.queue.retain(|queued| queued.identity != job.identity);
                if let Err(persist) = store.fail(job.identity.index_generation, &error.message) {
                    crate::app_logger::warn("StructureIndex", &persist.message);
                }
            }
        }
    }
}
