use super::super::{chunk_reader::DocumentSnapshot, TextDocumentError, TextDocumentResult};
use super::{
    check, count,
    scan::{self, Block},
    source::Source,
    ByteRange, DocumentStats, StatsMetrics, StatsQuery, StatsResult, DISK_BUDGET, MEMORY_BUDGET,
};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::oneshot;

static NEXT_WORK: AtomicU64 = AtomicU64::new(1);
fn cancelled() -> TextDocumentError {
    TextDocumentError::new("stats-cancelled", "统计请求已取消")
}
pub(super) struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::app_logger::warn("WritingStats", &format!("清理统计索引失败：{error}"));
            }
        }
    }
}
struct Index {
    directory: Arc<Directory>,
    document: String,
    revision: u64,
    snapshot: DocumentSnapshot,
    full: DocumentStats,
    bytes: u64,
    body_length: u64,
}
struct Job {
    owner: String,
    query: StatsQuery,
    snapshot: DocumentSnapshot,
    root: PathBuf,
    // Keeps the session directory alive without holding its mutex while scanning.
    _lifetime: Box<dyn Send>,
    cancel: Arc<AtomicBool>,
    reply: oneshot::Sender<TextDocumentResult<StatsResult>>,
}
struct Active {
    owner: String,
    document: String,
    request: u64,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    pending: VecDeque<Job>,
    active: Option<Active>,
    running: bool,
    cache: VecDeque<Arc<Index>>,
}
#[derive(Clone, Default)]
pub(crate) struct WritingStatsService {
    state: Arc<Mutex<State>>,
}
impl WritingStatsService {
    pub fn clean_stale(root: &Path) -> TextDocumentResult<()> {
        if !root.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("writing-stats");
            if path.is_dir() && !path.is_symlink() {
                fs::remove_dir_all(path)?;
            }
        }
        Ok(())
    }
    pub async fn request(
        &self,
        owner: String,
        query: StatsQuery,
        snapshot: DocumentSnapshot,
        root: PathBuf,
        lifetime: Box<dyn Send>,
    ) -> TextDocumentResult<StatsResult> {
        let (reply, receive) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let start = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| super::invalid("统计调度锁已损坏"))?;
            if let Some(active) = &state.active {
                if active.owner == owner {
                    active.cancel.store(true, Ordering::Relaxed);
                }
            }
            let mut i = 0;
            while i < state.pending.len() {
                if state.pending[i].owner == owner {
                    let old = state.pending.remove(i).unwrap();
                    let _ = old.reply.send(Err(cancelled()));
                } else {
                    i += 1;
                }
            }
            state.pending.push_back(Job {
                owner,
                query,
                snapshot,
                root,
                _lifetime: lifetime,
                cancel,
                reply,
            });
            // Identity-only queue is bounded as well as the scan workspace.
            if state.pending.len() > 64 {
                let old = state.pending.pop_front().unwrap();
                let _ = old.reply.send(Err(TextDocumentError::new(
                    "stats-busy",
                    "统计队列已满，请稍后重试",
                )));
            }
            let start = !state.running;
            state.running = true;
            start
        };
        if start {
            let service = self.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("markdown-stats".into())
                .stack_size(512 * 1024)
                .spawn(move || service.worker())
            {
                let mut state = self.state.lock().unwrap();
                state.running = false;
                for job in state.pending.drain(..) {
                    let _ = job.reply.send(Err(super::invalid(error.to_string())));
                }
            }
        }
        receive
            .await
            .map_err(|_| super::invalid("统计任务没有返回结果"))?
    }
    pub fn cancel(&self, owner: &str, request: Option<u64>) {
        let mut state = self.state.lock().unwrap();
        if let Some(active) = &state.active {
            if active.owner == owner && request.is_none_or(|r| active.request <= r) {
                active.cancel.store(true, Ordering::Relaxed);
            }
        }
        let mut i = 0;
        while i < state.pending.len() {
            if state.pending[i].owner == owner
                && request.is_none_or(|r| state.pending[i].query.request_id <= r)
            {
                let job = state.pending.remove(i).unwrap();
                let _ = job.reply.send(Err(cancelled()));
            } else {
                i += 1;
            }
        }
    }
    pub fn invalidate(&self, document: &str, close: bool) {
        let mut state = self.state.lock().unwrap();
        if let Some(active) = &state.active {
            if active.document == document {
                active.cancel.store(true, Ordering::Relaxed);
            }
        }
        let mut i = 0;
        while i < state.pending.len() {
            if state.pending[i].query.document_id == document {
                let job = state.pending.remove(i).unwrap();
                let _ = job.reply.send(Err(cancelled()));
            } else {
                i += 1;
            }
        }
        if close {
            state.cache.retain(|index| index.document != document);
        }
    }
    fn worker(&self) {
        loop {
            let job = {
                let mut state = self.state.lock().unwrap();
                let Some(job) = state.pending.pop_front() else {
                    state.active = None;
                    state.running = false;
                    return;
                };
                state.active = Some(Active {
                    owner: job.owner.clone(),
                    document: job.query.document_id.clone(),
                    request: job.query.request_id,
                    cancel: job.cancel.clone(),
                });
                job
            };
            let result = self.run(&job);
            let _ = job.reply.send(result);
            self.state.lock().unwrap().active = None;
        }
    }
    fn run(&self, job: &Job) -> TextDocumentResult<StatsResult> {
        check(&job.cancel)?;
        let started = Instant::now();
        let cached = {
            let mut state = self.state.lock().unwrap();
            let found = state.cache.iter().position(|index| {
                index.document == job.query.document_id && index.revision == job.query.revision
            });
            found.map(|i| {
                let index = state.cache.remove(i).unwrap();
                state.cache.push_back(index.clone());
                index
            })
        };
        let cache_hit = cached.is_some();
        let previous = {
            self.state
                .lock()
                .unwrap()
                .cache
                .iter()
                .find(|index| index.document == job.query.document_id)
                .cloned()
        };
        let mut reused_blocks = 0;
        let mut source = Source::snapshot(job.snapshot.clone(), job.cancel.clone());
        let index = if let Some(index) = cached {
            index
        } else {
            let parent = job.root.join("writing-stats");
            fs::create_dir_all(&parent)?;
            let path = parent.join(format!(
                "{}-{}",
                job.query.revision,
                NEXT_WORK.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            let directory = Arc::new(Directory(path));
            let (body_length, headings, reused) = scan::build(
                &mut source,
                &directory.0,
                previous
                    .as_ref()
                    .map(|index| (index.directory.0.as_path(), &index.snapshot)),
            )?;
            reused_blocks = reused;
            let (words, visible_chars) =
                scan::summarize(&directory.0, body_length, reused, job.cancel.clone())?;
            let (chars, lines) = source.raw_counts(0, source.len)?;
            let full = DocumentStats {
                chars,
                words,
                visible_chars,
                lines,
                headings,
                reading_minutes: 0,
            }
            .finish();
            let mut bytes = 0;
            for file in fs::read_dir(&directory.0)? {
                bytes += file?.metadata()?.len();
            }
            let index = Arc::new(Index {
                directory,
                document: job.query.document_id.clone(),
                revision: job.query.revision,
                snapshot: job.snapshot.clone(),
                full,
                bytes,
                body_length,
            });
            check(&job.cancel)?;
            let mut state = self.state.lock().unwrap();
            check(&job.cancel)?;
            state.cache.retain(|old| old.document != index.document);
            if bytes <= DISK_BUDGET {
                while state.cache.len() >= 128
                    || state.cache.iter().map(|i| i.bytes).sum::<u64>() + bytes > DISK_BUDGET
                {
                    state.cache.pop_front();
                }
                state.cache.push_back(index.clone());
            }
            index
        };
        let selection = job
            .query
            .selection
            .map(|s| s.normalized(source.len))
            .filter(|s| s.from != s.to);
        let selected = if let Some(range) = selection {
            if range.from == 0 && range.to == source.len {
                Some(index.full.clone())
            } else {
                // Binary search the on-disk block checkpoints; only intersecting
                // paragraphs are read, preserving context around partial words.
                let (from, to) = query_blocks(&index.directory.0, range, index.body_length)?;
                let (_, _, words, visible_chars) = count::count(
                    &index.directory.0,
                    from,
                    to,
                    Some(range),
                    job.cancel.clone(),
                )?;
                let (chars, lines) = source.raw_counts(range.from, range.to)?;
                Some(
                    DocumentStats {
                        chars,
                        words,
                        visible_chars,
                        lines,
                        headings: 0,
                        reading_minutes: 0,
                    }
                    .finish(),
                )
            }
        } else {
            None
        };
        check(&job.cancel)?;
        Ok(StatsResult {
            identity: job.query.clone(),
            full: index.full.clone(),
            selected,
            metrics: StatsMetrics {
                elapsed_ms: started.elapsed().as_millis(),
                snapshot_bytes_read: source.read_bytes,
                // Fixed readers/writers, 20 syntax frames, bounded labels/tags and
                // Unicode tables. This is a conservative container bound, not RSS.
                workspace_bound_bytes: MEMORY_BUDGET,
                index_bytes: index.bytes,
                cache_hit,
                reused_blocks,
            },
        })
    }
}
fn query_blocks(root: &Path, range: ByteRange, body_length: u64) -> TextDocumentResult<(u64, u64)> {
    let mut file = File::open(root.join("blocks"))?;
    let count = file.metadata()?.len() / Block::SIZE;
    let mut find = |offset: u64| -> TextDocumentResult<u64> {
        let (mut lo, mut hi) = (0, count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            file.seek(SeekFrom::Start(mid * Block::SIZE))?;
            let block = Block::read(&mut file)?;
            if block.to <= offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    };
    let first = find(range.from)?;
    let last = find(range.to.saturating_sub(1))?;
    if first >= count {
        return Ok((body_length, body_length));
    }
    file.seek(SeekFrom::Start(first * Block::SIZE))?;
    let from = Block::read(&mut file)?.body;
    file.seek(SeekFrom::Start(last.min(count - 1) * Block::SIZE))?;
    let to = Block::read(&mut file)?.body_end;
    Ok((from, to))
}
