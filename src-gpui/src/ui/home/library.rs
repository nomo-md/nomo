//! 主页只持久化目录入口、最近路径和可再生缩略图；不保存正文或编辑会话。
use gpui::{App, AppContext, Context, Entity, Global, Task};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::VecDeque, path::{Path, PathBuf}, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

pub(super) const RECENT_LIMIT: usize = 200;
const FOLDER_LIMIT: usize = 64;
const PATH_LIMIT: usize = 8192;
const DISK_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_PNG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Entry {
    pub path: PathBuf,
    pub opened: u64,
    /// 仅用于进程内保存失效通知，不改变“最近打开”的排序时间。
    pub revision: u64,
}

#[derive(Clone, Default)]
pub(crate) struct Snapshot {
    pub folders: Vec<Entry>,
    pub recent: Vec<Entry>,
}

pub(crate) enum Change {
    Refresh,
    Opened(PathBuf),
    Saved(PathBuf),
    AddFolder(PathBuf),
    Remove { path: PathBuf, folder: bool },
    ClearRecent,
    Rename(Vec<(PathBuf, PathBuf)>),
}

pub(crate) struct Library {
    pub snapshot: Arc<Snapshot>,
    pub error: Option<String>,
    pub loaded: bool,
    pub generation: u64,
    pending: VecDeque<Change>,
    worker: Option<Task<()>>,
}

#[derive(Clone)]
struct SharedLibrary(Entity<Library>);
impl Global for SharedLibrary {}

pub(crate) fn shared(cx: &mut App) -> Entity<Library> {
    if let Some(shared) = cx.try_global::<SharedLibrary>() { return shared.0.clone(); }
    let library = cx.new(|cx| {
        let mut library = Library { snapshot: Arc::default(), error: None, loaded: false, generation: 0, pending: VecDeque::new(), worker: None };
        library.change(Change::Refresh, cx);
        library
    });
    cx.set_global(SharedLibrary(library.clone()));
    library
}

impl Library {
    pub(crate) fn is_busy(&self) -> bool { self.worker.is_some() || !self.pending.is_empty() }

    pub(crate) fn change(&mut self, change: Change, cx: &mut Context<Self>) {
        // 刷新是可合并的读请求；文件事件按发生顺序提交，避免晚到的读取覆盖新记录。
        if matches!(change, Change::Refresh) && self.pending.iter().any(|item| matches!(item, Change::Refresh)) { return; }
        if self.pending.len() >= 256 {
            self.error = Some("主页记录队列已满，请稍后重新打开文件或添加目录".into());
            cx.notify();
            return;
        }
        self.pending.push_back(change);
        if self.worker.is_some() { return; }
        self.worker = Some(cx.spawn(async move |this, cx| {
            loop {
                let change = this.update(cx, |library, _| library.pending.pop_front()).ok().flatten();
                let Some(change) = change else { break; };
                let saved = match &change { Change::Saved(path) => Some(path.clone()), _ => None };
                let result = cx.background_executor().spawn(async move { apply(change) }).await;
                if this.update(cx, |library, cx| {
                    library.loaded = true;
                    match result {
                        Ok(mut snapshot) => {
                            for entry in &mut snapshot.recent {
                                entry.revision = if saved.as_ref() == Some(&entry.path) { library.generation.wrapping_add(1) }
                                    else { library.snapshot.recent.iter().find(|old| old.path == entry.path).map_or(0, |old| old.revision) };
                            }
                            library.snapshot = Arc::new(snapshot); library.error = None;
                        }
                        Err(error) => {
                            crate::app_logger::warn("Home", &format!("主页记录保存或读取失败：{error}"));
                            library.error = Some(format!("主页记录保存或读取失败：{error}"));
                        },
                    }
                    library.generation = library.generation.wrapping_add(1);
                    cx.notify();
                }).is_err() { return; }
            }
            let _ = this.update(cx, |library, cx| { library.worker = None; cx.notify(); });
        }));
    }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as u64 }

pub(super) fn path_key(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("文件路径不能转换为 UTF-8")?;
    if path.len() > PATH_LIMIT { return Err("文件路径过长，未加入主页记录".into()); }
    Ok(path.to_owned())
}

fn connect() -> Result<Connection, String> {
    let root = crate::settings::config_path().and_then(|p| p.parent().map(PathBuf::from)).ok_or("无法确定主页数据目录")?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join("document-library.sqlite3");
    let new = !path.exists();
    let connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection.busy_timeout(Duration::from_secs(2)).map_err(|e| e.to_string())?;
    // 结构索引已经设置进程级 SQLite 堆上限；主页连接只使用很小的页缓存。
    connection.execute_batch("PRAGMA cache_size=-128; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;").map_err(|e| e.to_string())?;
    if new { connection.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;").map_err(|e| e.to_string())?; }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (path TEXT NOT NULL, folder INTEGER NOT NULL, opened INTEGER NOT NULL, PRIMARY KEY(path, folder));
         CREATE TABLE IF NOT EXISTS thumbnails (path TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, png BLOB NOT NULL, touched INTEGER NOT NULL);"
    ).map_err(|e| e.to_string())?;
    Ok(connection)
}

fn read_entries(connection: &Connection, folder: bool) -> Result<Vec<Entry>, String> {
    let limit = if folder { FOLDER_LIMIT } else { RECENT_LIMIT };
    let mut query = connection.prepare("SELECT path, opened FROM entries WHERE folder=?1 ORDER BY opened DESC, path LIMIT ?2").map_err(|e| e.to_string())?;
    let entries = query.query_map(params![folder, limit as i64], |row| {
        Ok(Entry { path: PathBuf::from(row.get::<_, String>(0)?), opened: row.get::<_, i64>(1)?.max(0) as u64, revision: 0 })
    }).map_err(|e| e.to_string())?;
    entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

fn apply(change: Change) -> Result<Snapshot, String> {
    let mut connection = connect()?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        match change {
            Change::Refresh => {},
            Change::Opened(path) => {
                let key = path_key(&path)?;
                transaction.execute("INSERT INTO entries(path,folder,opened) VALUES (?1,0,?2) ON CONFLICT(path,folder) DO UPDATE SET opened=excluded.opened", params![key, now() as i64]).map_err(|e| e.to_string())?;
                transaction.execute("DELETE FROM entries WHERE folder=0 AND path NOT IN (SELECT path FROM entries WHERE folder=0 ORDER BY opened DESC,path LIMIT ?1)", params![RECENT_LIMIT as i64]).map_err(|e| e.to_string())?;
            },
            Change::Saved(path) => {
                let key = path_key(&path)?;
                transaction.execute("INSERT OR IGNORE INTO entries(path,folder,opened) VALUES (?1,0,?2)", params![key, now() as i64]).map_err(|e| e.to_string())?;
                transaction.execute("DELETE FROM thumbnails WHERE path=?1", params![key]).map_err(|e| e.to_string())?;
                transaction.execute("DELETE FROM entries WHERE folder=0 AND path NOT IN (SELECT path FROM entries WHERE folder=0 ORDER BY opened DESC,path LIMIT ?1)", params![RECENT_LIMIT as i64]).map_err(|e| e.to_string())?;
            },
            Change::AddFolder(path) => {
                let key = path_key(&path)?;
                let count: i64 = transaction.query_row("SELECT count(*) FROM entries WHERE folder=1 AND path<>?1", params![key], |row| row.get(0)).map_err(|e| e.to_string())?;
                if count >= FOLDER_LIMIT as i64 { return Err(format!("最多保留 {FOLDER_LIMIT} 个文件夹入口，请先移除不用的入口")); }
                transaction.execute("INSERT INTO entries(path,folder,opened) VALUES (?1,1,?2) ON CONFLICT(path,folder) DO UPDATE SET opened=excluded.opened", params![key, now() as i64]).map_err(|e| e.to_string())?;
            },
            Change::Remove { path, folder } => {
                transaction.execute("DELETE FROM entries WHERE path=?1 AND folder=?2", params![path_key(&path)?, folder]).map_err(|e| e.to_string())?;
            },
            Change::ClearRecent => { transaction.execute("DELETE FROM entries WHERE folder=0", []).map_err(|e| e.to_string())?; },
            Change::Rename(paths) => {
                for folder in [false, true] {
                    let entries = read_entries(&transaction, folder)?;
                    for entry in entries {
                        if let Some(next) = paths.iter().find_map(|(old, new)| entry.path.strip_prefix(old).ok().map(|suffix| new.join(suffix))) {
                            let old = path_key(&entry.path)?;
                            let next = path_key(&next)?;
                            transaction.execute("DELETE FROM entries WHERE path=?1 AND folder=?2", params![old, folder]).map_err(|e| e.to_string())?;
                            transaction.execute("INSERT INTO entries(path,folder,opened) VALUES (?1,?2,?3) ON CONFLICT(path,folder) DO UPDATE SET opened=MAX(opened,excluded.opened)", params![next, folder, entry.opened as i64]).map_err(|e| e.to_string())?;
                        }
                    }
                }
            },
        }
        transaction.execute("DELETE FROM thumbnails WHERE path NOT IN (SELECT path FROM entries WHERE folder=0)", []).map_err(|e| e.to_string())?;
        Ok(())
    })();
    result?;
    transaction.commit().map_err(|e| e.to_string())?;
    let snapshot = Snapshot { folders: read_entries(&connection, true)?, recent: read_entries(&connection, false)? };
    connection.execute_batch("PRAGMA incremental_vacuum(64);").map_err(|e| e.to_string())?;
    Ok(snapshot)
}

pub(super) fn cached_thumbnail(path: &Path, fingerprint: &str) -> Result<Option<Vec<u8>>, String> {
    let connection = connect()?;
    let key = path_key(path)?;
    let png = connection.query_row("SELECT png FROM thumbnails WHERE path=?1 AND fingerprint=?2 AND length(png)<=?3", params![key, fingerprint, MAX_PNG_BYTES as i64], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    if png.is_some() { connection.execute("UPDATE thumbnails SET touched=?2 WHERE path=?1", params![key, now() as i64]).map_err(|e| e.to_string())?; }
    Ok(png)
}

pub(super) fn store_thumbnail(path: &Path, fingerprint: &str, png: &[u8]) -> Result<(), String> {
    if png.len() > MAX_PNG_BYTES { return Err("缩略图超过单项缓存预算".into()); }
    let mut connection = connect()?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let key = path_key(path)?;
    // 清除记录后，晚到的预览任务不得把对应内容重新写回缓存。
    transaction.execute("INSERT INTO thumbnails(path,fingerprint,png,touched) SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM entries WHERE path=?1 AND folder=0) ON CONFLICT(path) DO UPDATE SET fingerprint=excluded.fingerprint,png=excluded.png,touched=excluded.touched", params![key, fingerprint, png, now() as i64]).map_err(|e| e.to_string())?;
    let sizes = {
        let mut query = transaction.prepare("SELECT path,length(png) FROM thumbnails ORDER BY touched DESC,path").map_err(|e| e.to_string())?;
        query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };
    let mut total = 0usize;
    for (index, (path, bytes)) in sizes.into_iter().enumerate() {
        total = total.saturating_add(bytes.max(0) as usize);
        if total > DISK_CACHE_BYTES || index >= RECENT_LIMIT {
            transaction.execute("DELETE FROM thumbnails WHERE path=?1", params![path]).map_err(|e| e.to_string())?;
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    connection.execute_batch("PRAGMA incremental_vacuum(64);").map_err(|e| e.to_string())?;
    Ok(())
}
