use super::*;
use crate::text_document::{
    chunk_reader::DocumentSnapshot, markdown_session::search::PreparedReplacement,
    TextDocumentError, TextDocumentResult,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);
const CACHE_BYTES: usize = 2 * 1024 * 1024;
type Work = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct QueueState {
    jobs: VecDeque<(String, Work)>,
    workers: usize,
}
#[derive(Default)]
struct Queue {
    state: Mutex<QueueState>,
    wake: Condvar,
}
fn queue() -> &'static Arc<Queue> {
    static QUEUE: OnceLock<Arc<Queue>> = OnceLock::new();
    QUEUE.get_or_init(Default::default)
}

// 按需启动至多两个工作线程，空闲后退出；分页插到队首。取消不等待正文锁。
fn schedule(id: String, priority: bool, work: Work) {
    let queue = queue().clone();
    let mut state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
    if priority {
        state.jobs.push_front((id, work));
    } else {
        state.jobs.push_back((id, work));
    }
    while state.workers < 2 {
        state.workers += 1;
        let queue = queue.clone();
        std::thread::spawn(move || loop {
            let work = {
                let mut state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
                while state.jobs.is_empty() {
                    let (next, timeout) = queue
                        .wake
                        .wait_timeout(state, Duration::from_secs(30))
                        .unwrap_or_else(|e| e.into_inner());
                    state = next;
                    if timeout.timed_out() && state.jobs.is_empty() {
                        state.workers -= 1;
                        return;
                    }
                }
                state.jobs.pop_front().unwrap().1
            };
            work();
        });
    }
    queue.wake.notify_all();
}

pub(in crate::text_document) struct SearchIndex {
    pub count: u64,
    pub checkpoints: Vec<Checkpoint>,
}
pub(in crate::text_document) struct TaskData {
    pub progress: TaskProgress,
    pub index: Option<SearchIndex>,
    pub prepared: Option<PreparedReplacement>,
    pub completed: Option<ReplaceResult>,
    pub completed_operation: Option<String>,
    touched: Instant,
}
pub(in crate::text_document) struct SearchTask {
    pub owner: String,
    pub query: SearchQuery,
    pub identity: TaskIdentity,
    pub snapshot: DocumentSnapshot,
    pub cancel: AtomicBool,
    pub matcher: Matcher,
    pub data: Mutex<TaskData>,
    sink: Option<ProgressSink>,
    active: Mutex<usize>,
    idle: Condvar,
    retired: Weak<Mutex<HashMap<String, Arc<SearchTask>>>>,
    // 会话引用使关闭后的后台取消仍能安全读取快照，最后一个任务释放后才清理目录。
    _lifetime: Box<dyn Send + Sync>,
}
struct WorkLease(Arc<SearchTask>);
impl Drop for WorkLease {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().unwrap_or_else(|e| e.into_inner());
        *active -= 1;
        self.0.idle.notify_all();
        if *active == 0 {
            drop(active);
            if let Some(retired) = self.0.retired.upgrade() {
                retired
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&self.0.identity.task_id);
            }
        }
    }
}
impl SearchTask {
    fn lease(self: &Arc<Self>) -> WorkLease {
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        WorkLease(self.clone())
    }
    pub fn emit(&self, state: &str, scanned: u64, count: u64, error: Option<TextDocumentError>) {
        let progress = {
            let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
            data.progress = TaskProgress {
                identity: self.identity.clone(),
                state: state.into(),
                scanned_bytes: scanned,
                total_bytes: self.snapshot.len(),
                match_count: count,
                error,
            };
            data.progress.clone()
        };
        if let Some(sink) = &self.sink {
            sink(progress);
        }
    }
    pub fn finish_error(&self, error: TextDocumentError) {
        let state = if error.code == "task-cancelled" {
            "cancelled"
        } else if error.code == "revision-conflict" {
            "conflict"
        } else {
            "failed"
        };
        self.emit(state, 0, 0, Some(error));
    }
    pub fn progress_reporter(&self) -> impl FnMut(u64, u64) + '_ {
        let mut emitted = Instant::now() - Duration::from_secs(1);
        move |bytes, count| {
            if emitted.elapsed() >= Duration::from_millis(100) {
                self.emit("running", bytes, count, None);
                emitted = Instant::now();
            }
        }
    }
}

#[derive(Clone, Default)]
pub(in crate::text_document) struct SearchService {
    tasks: Arc<Mutex<HashMap<String, Arc<SearchTask>>>>,
    retired: Arc<Mutex<HashMap<String, Arc<SearchTask>>>>,
}
impl SearchService {
    pub fn create(
        &self,
        owner: String,
        query: SearchQuery,
        snapshot: DocumentSnapshot,
        lifetime: Box<dyn Send + Sync>,
        sink: Option<ProgressSink>,
    ) -> TextDocumentResult<Arc<SearchTask>> {
        let matcher = Matcher::new(&query)?;
        let identity = TaskIdentity {
            task_id: format!(
                "markdown-search-{}",
                NEXT_TASK.fetch_add(1, Ordering::Relaxed)
            ),
            request_id: query.request_id.clone(),
            document_id: query.document_id.clone(),
            base_revision: query.base_revision,
        };
        let total_bytes = snapshot.len();
        let task = Arc::new(SearchTask {
            owner: owner.clone(),
            query,
            snapshot,
            matcher,
            cancel: AtomicBool::new(false),
            data: Mutex::new(TaskData {
                progress: TaskProgress {
                    identity: identity.clone(),
                    state: "queued".into(),
                    scanned_bytes: 0,
                    total_bytes,
                    match_count: 0,
                    error: None,
                },
                index: None,
                prepared: None,
                completed: None,
                completed_operation: None,
                touched: Instant::now(),
            }),
            identity,
            sink,
            _lifetime: lifetime,
            active: Mutex::new(0),
            idle: Condvar::new(),
            retired: Arc::downgrade(&self.retired),
        });
        self.cancel_owner(&owner);
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.identity.task_id.clone(), task.clone());
        Ok(task)
    }
    pub fn get(&self, owner: &str, id: &str) -> TextDocumentResult<Arc<SearchTask>> {
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let task = tasks
            .get(id)
            .filter(|task| task.owner == owner)
            .ok_or_else(|| {
                TextDocumentError::new("search-expired", "搜索结果已经失效，请重新搜索")
            })?
            .clone();
        drop(tasks);
        task.data.lock().unwrap_or_else(|e| e.into_inner()).touched = Instant::now();
        Ok(task)
    }
    pub fn start(
        &self,
        task: Arc<SearchTask>,
        run: impl FnOnce(Arc<SearchTask>) -> TextDocumentResult<()> + Send + 'static,
    ) {
        let service = self.clone();
        let lease = task.lease();
        schedule(
            task.identity.task_id.clone(),
            false,
            Box::new(move || {
                let _lease = lease;
                if let Err(error) = check_cancel(&task.cancel).and_then(|_| run(task.clone())) {
                    task.finish_error(error);
                }
                service.trim();
            }),
        );
    }
    pub fn start_search(&self, task: Arc<SearchTask>) {
        self.start(task, |task| {
            let scanned = task.matcher.scan(
                &task.snapshot,
                Checkpoint::default(),
                &task.cancel,
                true,
                |_, _| Ok(true),
                task.progress_reporter(),
            )?;
            check_cancel(&task.cancel)?;
            task.data.lock().unwrap_or_else(|e| e.into_inner()).index = Some(SearchIndex {
                count: scanned.count,
                checkpoints: scanned.checkpoints,
            });
            task.emit("ready", task.snapshot.len(), scanned.count, None);
            Ok(())
        });
    }
    pub fn page(
        &self,
        owner: &str,
        id: &str,
        from: u64,
        limit: usize,
    ) -> TextDocumentResult<SearchPage> {
        let task = self.get(owner, id)?;
        check_cancel(&task.cancel)?;
        let (count, checkpoint) = {
            let data = task.data.lock().unwrap_or_else(|e| e.into_inner());
            let index = data
                .index
                .as_ref()
                .ok_or_else(|| TextDocumentError::new("search-not-ready", "搜索尚未完成"))?;
            let checkpoint = index
                .checkpoints
                .iter()
                .rev()
                .find(|c| c.count <= from)
                .copied()
                .unwrap_or_default();
            (index.count, checkpoint)
        };
        let limit = limit.clamp(1, PAGE_SIZE);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let identity = task.identity.clone();
        let lease = task.lease();
        schedule(
            id.into(),
            true,
            Box::new(move || {
                let _lease = lease;
                let result = (|| {
                    let mut matches = Vec::with_capacity(limit);
                    if from < count {
                        task.matcher.scan(
                            &task.snapshot,
                            checkpoint,
                            &task.cancel,
                            false,
                            |m, _| {
                                if m.index >= from {
                                    matches.push(m);
                                }
                                Ok(matches.len() < limit)
                            },
                            |_, _| {},
                        )?;
                    }
                    check_cancel(&task.cancel)?;
                    Ok(SearchPage {
                        identity,
                        total_count: count,
                        from_index: from,
                        matches,
                    })
                })();
                let _ = send.send(result);
            }),
        );
        receive
            .recv()
            .map_err(|_| TextDocumentError::new("task-cancelled", "搜索分页已取消"))?
    }
    pub fn cancel(&self, owner: &str, id: &str) {
        let task = {
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            if tasks.get(id).is_some_and(|task| task.owner == owner) {
                tasks.remove(id)
            } else {
                None
            }
        };
        if let Some(task) = task {
            task.cancel.store(true, Ordering::Release);
            // invalidation 可能持有 Session 锁，不能反向等待正在提交的 Task 锁。
            if let Ok(mut data) = task.data.try_lock() {
                if data.completed.is_none() {
                    data.progress.state = "cancelled".into();
                    if let Some(sink) = &task.sink {
                        sink(data.progress.clone());
                    }
                }
            }
            queue()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .jobs
                .retain(|(key, _)| key != id);
            let active = task.active.lock().unwrap_or_else(|e| e.into_inner());
            if *active > 0 {
                self.retired
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.into(), task.clone());
            }
        }
    }
    pub fn invalidate(&self, document: &str, except: Option<&str>) {
        let ids: Vec<_> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|t| {
                t.query.document_id == document && except != Some(t.identity.task_id.as_str())
            })
            .map(|t| (t.owner.clone(), t.identity.task_id.clone()))
            .collect();
        for (owner, id) in ids {
            self.cancel(&owner, &id);
        }
    }
    pub fn release(&self, owner: &str, id: &str) {
        let task = self.get(owner, id).ok().or_else(|| {
            self.retired
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .filter(|task| task.owner == owner)
                .cloned()
        });
        self.cancel(owner, id);
        if let Some(task) = task {
            let mut active = task.active.lock().unwrap_or_else(|e| e.into_inner());
            while *active > 0 {
                active = task.idle.wait(active).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
    pub fn cancel_owner(&self, owner: &str) {
        let ids: Vec<_> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|t| t.owner == owner)
            .map(|t| t.identity.task_id.clone())
            .collect();
        for id in ids {
            self.cancel(owner, &id);
        }
    }
    fn trim(&self) {
        let mut candidates = vec![];
        let mut used = 0;
        {
            let tasks: Vec<_> = self
                .tasks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .cloned()
                .collect();
            for task in tasks {
                let data = task.data.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(index) = &data.index {
                    let size = index.checkpoints.capacity() * std::mem::size_of::<Checkpoint>();
                    used += size;
                    candidates.push((
                        data.touched,
                        size,
                        task.owner.clone(),
                        task.identity.task_id.clone(),
                    ));
                }
            }
        }
        candidates.sort_by_key(|entry| entry.0);
        let mut remaining = candidates.len();
        for (_, size, owner, id) in candidates {
            if used <= CACHE_BYTES && remaining <= 16 {
                break;
            }
            self.cancel(&owner, &id);
            used -= size;
            remaining -= 1;
        }
    }
}
