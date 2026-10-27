use super::{piece_tree::PieceSource, TextDocumentError, TextDocumentResult};
#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const COPY_BUFFER_BYTES: usize = 64 * 1024;
pub(super) const DEFAULT_CACHE_CAPACITY_BYTES: usize = 8 * 1024 * 1024;
#[cfg(test)]
static MAX_READ_RANGE_REQUEST: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static TOTAL_READ_RANGE_BYTES: AtomicUsize = AtomicUsize::new(0);

type CacheKey = (u64, u64, u64);

struct CacheInner {
    entries: HashMap<CacheKey, Arc<Vec<u8>>>,
    order: VecDeque<CacheKey>,
    used_bytes: usize,
}

pub(super) struct ChunkCache {
    capacity_bytes: usize,
    inner: Mutex<CacheInner>,
}

impl ChunkCache {
    pub(super) fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            inner: Mutex::new(CacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                used_bytes: 0,
            }),
        }
    }

    pub(super) fn get(
        &self,
        revision: u64,
        start: u64,
        end: u64,
    ) -> TextDocumentResult<Option<Arc<Vec<u8>>>> {
        let key = (revision, start, end);
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "分块缓存锁已损坏"))?;
        let Some(value) = inner.entries.get(&key).cloned() else {
            return Ok(None);
        };
        inner.order.retain(|entry| *entry != key);
        inner.order.push_back(key);
        Ok(Some(value))
    }

    pub(super) fn insert(
        &self,
        revision: u64,
        start: u64,
        end: u64,
        bytes: Vec<u8>,
    ) -> TextDocumentResult<Arc<Vec<u8>>> {
        let value = Arc::new(bytes);
        if value.len() > self.capacity_bytes {
            return Ok(value);
        }
        let key = (revision, start, end);
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "分块缓存锁已损坏"))?;
        if let Some(previous) = inner.entries.remove(&key) {
            inner.used_bytes = inner.used_bytes.saturating_sub(previous.len());
            inner.order.retain(|entry| *entry != key);
        }
        while inner.used_bytes + value.len() > self.capacity_bytes {
            let Some(oldest) = inner.order.pop_front() else {
                break;
            };
            if let Some(removed) = inner.entries.remove(&oldest) {
                inner.used_bytes = inner.used_bytes.saturating_sub(removed.len());
            }
        }
        inner.used_bytes += value.len();
        inner.entries.insert(key, value.clone());
        inner.order.push_back(key);
        Ok(value)
    }

    pub(super) fn clear(&self) -> TextDocumentResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "分块缓存锁已损坏"))?;
        inner.entries.clear();
        inner.order.clear();
        inner.used_bytes = 0;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn usage(&self) -> TextDocumentResult<(usize, usize)> {
        self.inner
            .lock()
            .map(|inner| (inner.used_bytes, self.capacity_bytes))
            .map_err(|_| TextDocumentError::new("lock-poisoned", "分块缓存锁已损坏"))
    }
}

#[derive(Debug)]
pub(super) struct OriginalFile {
    file: File,
    body_offset: u64,
    body_len: u64,
    physical_len: u64,
    modified: std::time::SystemTime,
    // 会话 baseline 是应用私有资产；journal 通过 recovery 目录中的硬链接独立持有它。
    owned_asset: Option<PathBuf>,
}

impl OriginalFile {
    pub(super) fn open(path: &Path, body_offset: u64) -> TextDocumentResult<Self> {
        let file = File::open(path).map_err(|error| {
            TextDocumentError::new("open-failed", format!("打开分段文档失败：{error}"))
        })?;
        Self::from_file(file, body_offset)
    }

    /// 接管已完成身份绑定的句柄，避免 probe 校验后再次按路径打开产生 TOCTOU。
    pub(super) fn from_file(file: File, body_offset: u64) -> TextDocumentResult<Self> {
        let metadata = file.metadata()?;
        let physical_len = metadata.len();
        let modified = metadata.modified()?;
        Ok(Self {
            file,
            body_offset,
            body_len: physical_len.saturating_sub(body_offset),
            physical_len,
            modified,
            owned_asset: None,
        })
    }

    /// 以文件级 COW clone 建立真正独立于源路径写入的 baseline；不支持 clone 时才流式复制。
    pub(super) fn open_immutable(
        source: &Path,
        asset_path: PathBuf,
        body_offset: u64,
    ) -> TextDocumentResult<Self> {
        if let Some(original) =
            Self::try_open_cow_immutable(source, asset_path.clone(), body_offset)?
        {
            return Ok(original);
        }
        copy_durable_immutable_asset(source, &asset_path, 0)?;
        let mut original = Self::open(&asset_path, body_offset)?;
        original.owned_asset = Some(asset_path);
        Ok(original)
    }

    /// 打开热路径只尝试文件系统 COW；失败返回 None，由会话在首窗发布后后台流式物化。
    pub(super) fn try_open_cow_immutable(
        source: &Path,
        asset_path: PathBuf,
        body_offset: u64,
    ) -> TextDocumentResult<Option<Self>> {
        if let Some(parent) = asset_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        remove_file_if_exists(&asset_path)?;
        if clone_file_cow(source, &asset_path).is_err() {
            remove_file_if_exists(&asset_path)?;
            return Ok(None);
        }
        File::open(&asset_path)?.sync_all()?;
        sync_parent_directory(&asset_path)?;
        let mut original = Self::open(&asset_path, body_offset)?;
        original.owned_asset = Some(asset_path);
        Ok(Some(original))
    }

    pub(super) fn copy_to_immutable(
        source: &Path,
        asset_path: PathBuf,
        body_offset: u64,
        chunk_delay_millis: u64,
    ) -> TextDocumentResult<Self> {
        copy_durable_immutable_asset(source, &asset_path, chunk_delay_millis)?;
        let mut original = Self::open(&asset_path, body_offset)?;
        original.owned_asset = Some(asset_path);
        Ok(original)
    }

    /// 小文件已完整包含在固定上限 probe 中，直接发布该字节快照，避免跨平台后台竞态。
    pub(super) fn create_immutable_from_bytes(
        asset_path: PathBuf,
        bytes: &[u8],
        body_offset: u64,
    ) -> TextDocumentResult<Self> {
        if let Some(parent) = asset_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        remove_file_if_exists(&asset_path)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&asset_path)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        sync_parent_directory(&asset_path)?;
        let mut original = Self::open(&asset_path, body_offset)?;
        original.owned_asset = Some(asset_path);
        Ok(original)
    }

    pub(super) fn body_len(&self) -> u64 {
        self.body_len
    }

    pub(super) fn immutable_asset_path(&self) -> TextDocumentResult<&Path> {
        self.owned_asset.as_deref().ok_or_else(|| {
            TextDocumentError::new(
                "immutable-baseline-missing",
                "当前 Original 不是可持久化的会话 baseline",
            )
        })
    }

    fn read_range(&self, offset: u64, output: &mut [u8]) -> std::io::Result<usize> {
        self.ensure_unchanged()?;
        let read = read_at(&self.file, output, self.body_offset + offset)?;
        self.ensure_unchanged()?;
        Ok(read)
    }

    fn ensure_unchanged(&self) -> std::io::Result<()> {
        let metadata = self.file.metadata()?;
        if metadata.len() != self.physical_len || metadata.modified()? != self.modified {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "original file changed while reading immutable session snapshot",
            ));
        }
        Ok(())
    }
}

impl Drop for OriginalFile {
    fn drop(&mut self) {
        if let Some(path) = &self.owned_asset {
            if let Err(error) = remove_file_if_exists(path) {
                crate::app_logger::warn(
                    "SegmentedDocument",
                    &format!("清理会话 baseline 资产失败（{}）：{error}", path.display()),
                );
            }
        }
    }
}

fn copy_durable_immutable_asset(
    source: &Path,
    target: &Path,
    chunk_delay_millis: u64,
) -> TextDocumentResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_file_if_exists(target)?;
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        #[cfg(test)]
        if chunk_delay_millis > 0 {
            std::thread::sleep(std::time::Duration::from_millis(chunk_delay_millis));
        }
        #[cfg(not(test))]
        let _ = chunk_delay_millis;
    }
    output.sync_all()?;
    sync_parent_directory(target)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn clone_file_cow(source: &Path, target: &Path) -> std::io::Result<()> {
    unsafe extern "C" {
        fn clonefile(
            source: *const std::ffi::c_char,
            target: *const std::ffi::c_char,
            flags: u32,
        ) -> std::ffi::c_int;
    }
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source path contains NUL")
    })?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target path contains NUL")
    })?;
    // SAFETY: 两个 CString 在调用期间有效且 NUL 结尾；flags=0 采用 clonefile 的标准语义。
    if unsafe { clonefile(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn clone_file_cow(_source: &Path, _target: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file-level clone is not available on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
struct AddedInner {
    file: Option<File>,
    len: u64,
}

#[derive(Debug)]
pub(super) struct AddedStore {
    path: PathBuf,
    inner: Mutex<AddedInner>,
    persistent: bool,
}

impl AddedStore {
    pub(super) fn create(path: PathBuf) -> TextDocumentResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            persistent: false,
            inner: Mutex::new(AddedInner {
                file: Some(file),
                len: 0,
            }),
        })
    }

    /// Markdown 恢复日志持有 Added 区间；进程退出不能删除已确认输入。
    pub(super) fn open_persistent(path: PathBuf) -> TextDocumentResult<Self> {
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path)?;
        let len = file.metadata()?.len();
        Ok(Self { path, persistent: true, inner: Mutex::new(AddedInner { file: Some(file), len }) })
    }

    pub(super) fn append(&self, bytes: &[u8]) -> TextDocumentResult<u64> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))?;
        let offset = inner.len;
        let file = inner
            .file
            .as_mut()
            .ok_or_else(|| TextDocumentError::new("added-store-closed", "Added 文件已经关闭"))?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)?;
        inner.len += bytes.len() as u64;
        Ok(offset)
    }

    pub(super) fn append_reader(&self, reader: &mut dyn Read) -> TextDocumentResult<(u64, u64)> {
        self.append_reader_cancellable(reader, &|| false)
    }

    pub(super) fn append_reader_cancellable(
        &self,
        reader: &mut dyn Read,
        should_cancel: &dyn Fn() -> bool,
    ) -> TextDocumentResult<(u64, u64)> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))?;
        let offset = inner.len;
        let file = inner
            .file
            .as_mut()
            .ok_or_else(|| TextDocumentError::new("added-store-closed", "Added 文件已经关闭"))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
        let mut written = 0_u64;
        loop {
            if should_cancel() {
                return Err(TextDocumentError::new("task-cancelled", "后台任务已取消"));
            }
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])?;
            written += count as u64;
        }
        inner.len += written;
        Ok((offset, written))
    }

    pub(super) fn sync(&self) -> TextDocumentResult<()> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))?;
        inner
            .file
            .as_ref()
            .ok_or_else(|| TextDocumentError::new("added-store-closed", "Added 文件已经关闭"))?
            .sync_all()?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> TextDocumentResult<u64> {
        self.inner
            .lock()
            .map(|inner| inner.len)
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))
    }

    pub(super) fn remove(&self) -> TextDocumentResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))?;
        inner.file.take();
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                // 删除失败时恢复句柄，让 close 返回错误后会话仍可继续使用或重试。
                inner.file = Some(OpenOptions::new().read(true).write(true).open(&self.path)?);
                Err(error.into())
            }
        }
    }

    fn read_range(&self, offset: u64, output: &mut [u8]) -> TextDocumentResult<usize> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| TextDocumentError::new("lock-poisoned", "Added 文件锁已损坏"))?;
        let file = inner
            .file
            .as_ref()
            .ok_or_else(|| TextDocumentError::new("added-store-closed", "Added 文件已经关闭"))?;
        Ok(read_at(file, output, offset)?)
    }
}

impl Drop for AddedStore {
    fn drop(&mut self) {
        if self.persistent { return; }
        if let Err(error) = self.remove() {
            crate::app_logger::warn(
                "SegmentedDocument",
                &format!("清理 Added 临时文件失败：{error}"),
            );
        }
    }
}

#[derive(Clone)]
pub(super) struct DocumentSnapshot {
    pub(super) revision: u64,
    pub(super) tree: PieceTree,
    pub(super) original: Arc<OriginalFile>,
    pub(super) added: Arc<AddedStore>,
}

impl DocumentSnapshot {
    pub(super) fn len(&self) -> u64 {
        self.tree.len()
    }

    pub(super) fn read_range(&self, start: u64, length: usize) -> TextDocumentResult<Vec<u8>> {
        #[cfg(test)]
        {
            MAX_READ_RANGE_REQUEST.fetch_max(length, Ordering::Relaxed);
            TOTAL_READ_RANGE_BYTES.fetch_add(length, Ordering::Relaxed);
        }
        if start > self.len() {
            return Err(TextDocumentError::new(
                "invalid-read-range",
                format!("读取位置越界：{start} > {}", self.len()),
            ));
        }
        let wanted = std::cmp::min(length as u64, self.len() - start) as usize;
        let mut output = vec![0_u8; wanted];
        let mut output_offset = 0_usize;
        let wanted_end = start + wanted as u64;
        for (piece, piece_relative, segment_len) in self.tree.segments(start, wanted_end)? {
            let count = segment_len as usize;
            let target = &mut output[output_offset..output_offset + count];
            let read = match piece.source {
                PieceSource::Original { offset, .. } => {
                    self.original.read_range(offset + piece_relative, target)?
                }
                PieceSource::Added { offset, .. } => {
                    self.added.read_range(offset + piece_relative, target)?
                }
            };
            if read != count {
                return Err(TextDocumentError::new(
                    "unexpected-eof",
                    "分段正文在 Piece 区间内提前结束",
                ));
            }
            output_offset += count;
        }
        Ok(output)
    }

    pub(super) fn byte_at(&self, offset: u64) -> TextDocumentResult<Option<u8>> {
        if offset >= self.len() {
            return Ok(None);
        }
        Ok(self.read_range(offset, 1)?.first().copied())
    }

    pub(super) fn reader(&self) -> SnapshotReader {
        SnapshotReader {
            snapshot: self.clone(),
            position: 0,
        }
    }
}

#[cfg(test)]
pub(super) fn reset_read_range_metrics() {
    MAX_READ_RANGE_REQUEST.store(0, Ordering::Release);
    TOTAL_READ_RANGE_BYTES.store(0, Ordering::Release);
}

#[cfg(test)]
pub(super) fn max_read_range_request() -> usize {
    MAX_READ_RANGE_REQUEST.load(Ordering::Acquire)
}

#[cfg(test)]
pub(super) fn total_read_range_bytes() -> usize {
    TOTAL_READ_RANGE_BYTES.load(Ordering::Acquire)
}

pub(super) struct SnapshotReader {
    snapshot: DocumentSnapshot,
    position: u64,
}

impl Read for SnapshotReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let bytes = self
            .snapshot
            .read_range(self.position, output.len())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        output[..bytes.len()].copy_from_slice(&bytes);
        self.position += bytes.len() as u64;
        Ok(bytes.len())
    }
}

#[cfg(unix)]
fn read_at(file: &File, output: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, output, offset)
}

#[cfg(windows)]
fn read_at(file: &File, output: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, output, offset)
}

#[cfg(not(any(unix, windows)))]
fn read_at(file: &File, output: &mut [u8], offset: u64) -> std::io::Result<usize> {
    let mut cloned = file.try_clone()?;
    cloned.seek(SeekFrom::Start(offset))?;
    cloned.read(output)
}

// 避免 piece_tree 的内部类型泄露到父模块接口。
use super::piece_tree::PieceTree;
