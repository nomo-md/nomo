//! 本地图片的异步解码与有界缓存（GPUI 全局状态）。
//!
//! 解码前检查输入预算；后台任务和排队数量有界，按最长边和像素数缩小后上传；
//! 缓存按估算字节淘汰最久未用的条目。解码完成后递增 `generation` 并刷新窗口，
//! 语义视图把 generation 纳入含图片块的布局缓存键，从而在尺寸就绪后重排。

use gpui::{App, Global, RenderImage};
use image::{Frame, ImageDecoder, imageops::FilterType};
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 解码宽度上限：2× 视网膜屏下约 1200 逻辑像素，足够正文宽度使用。
const MAX_DECODE_WIDTH: u32 = 2400;
/// 位图缓存总量上限（BGRA 字节）。
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_INPUT_PIXELS: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BITMAP_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_DECODERS: usize = 2;
const MAX_QUEUED: usize = 64;
const MAX_ENTRIES: usize = 512;
const MAX_METADATA_BYTES: usize = 512 * 1024;
// 路径元数据淘汰后仍记住近期失败，避免同一屏大量坏图片随每次完成重新排队。
const MAX_FAILURES: usize = 2048;

#[derive(Clone, Copy)]
enum Failure { Budget, Read, Decode }
impl Failure {
    fn message(self) -> &'static str {
        match self { Self::Budget => "图片超过解码资源预算", Self::Read => "图片文件无法读取", Self::Decode => "图片格式不支持或内容损坏" }
    }
}
type Decoded = (Arc<RenderImage>, u32, u32, usize);

enum Entry {
    Queued,
    Loading,
    /// `used` 用 Cell 记录访问时钟，使绘制阶段只需共享借用即可更新 LRU 顺序。
    Ready { image: Arc<RenderImage>, width: u32, height: u32, bytes: usize, used: Cell<u64>, version: u64 },
    Failed { reason: Failure, used: Cell<u64>, version: u64 },
}

#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<PathBuf, Entry>,
    bytes: usize,
    clock: Cell<u64>,
    metadata_bytes: usize,
    queued: VecDeque<PathBuf>,
    running: usize,
    failures: HashMap<u64, (Failure, u64)>,
    failure_order: VecDeque<(u64, u64)>,
    pub generation: u64,
}

impl Global for ImageStore {}

impl ImageStore {
    fn metadata_cost(path: &Path) -> usize { (path.as_os_str().len() + 256).saturating_mul(4) }
    fn path_key(path: &Path) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hash);
        hash.finish()
    }
    fn remember_failure(&mut self, path: &Path, failure: Failure) {
        let key = Self::path_key(path);
        if let Some((stored,version)) = self.failures.get_mut(&key) {
            if *version == self.generation { *stored = failure; return; }
        }
        while self.failure_order.len() >= MAX_FAILURES {
            if let Some((key,version)) = self.failure_order.pop_front() {
                if self.failures.get(&key).is_some_and(|(_,current)| *current == version) { self.failures.remove(&key); }
            }
        }
        self.failures.insert(key,(failure,self.generation));
        self.failure_order.push_back((key,self.generation));
    }

    pub fn version(&self, path: &Path) -> (u8, u64) {
        if Self::metadata_cost(path) > MAX_METADATA_BYTES { return (2,0); }
        match self.entries.get(path) {
            Some(Entry::Ready { version, .. }) => (1, *version),
            Some(Entry::Failed { version, .. }) => (2, *version),
            Some(_) => (0, 0),
            None => self.failures.get(&Self::path_key(path)).map_or((3,self.generation), |(_,version)| (2,*version)),
        }
    }
    pub fn failure(&self, path: &Path) -> Option<&'static str> {
        if Self::metadata_cost(path) > MAX_METADATA_BYTES { return Some(Failure::Budget.message()); }
        match self.entries.get(path) { Some(Entry::Failed { reason, .. }) => Some(reason.message()), Some(_) => None,
            None => self.failures.get(&Self::path_key(path)).map(|(reason,_)| reason.message()) }
    }
    /// 已解码图片的原始像素尺寸（缩放前）；未就绪返回 None。
    pub fn size(&self, path: &Path) -> Option<(u32, u32)> {
        match self.entries.get(path) {
            Some(Entry::Ready { width, height, .. }) => Some((*width, *height)),
            _ => None,
        }
    }

    pub fn image(&self, path: &Path) -> Option<Arc<RenderImage>> {
        self.clock.set(self.clock.get() + 1);
        match self.entries.get(path) {
            Some(Entry::Ready { image, used, .. }) => {
                used.set(self.clock.get());
                Some(image.clone())
            }
            _ => None,
        }
    }

    pub fn is_pending_or_known(&self, path: &Path) -> bool {
        self.entries.contains_key(path) || Self::metadata_cost(path) > MAX_METADATA_BYTES || self.failures.contains_key(&Self::path_key(path))
    }

    fn remove(&mut self, path: &Path) -> Option<Entry> {
        let entry = self.entries.remove(path)?;
        self.metadata_bytes = self.metadata_bytes.saturating_sub(Self::metadata_cost(path));
        if let Entry::Ready { bytes, .. } = &entry { self.bytes -= bytes; }
        Some(entry)
    }
    fn evict_oldest(&mut self, retired: &mut Vec<Arc<RenderImage>>) -> bool {
        let oldest = self.entries.iter().filter_map(|(path, entry)| match entry {
            Entry::Ready { used, .. } | Entry::Failed { used, .. } => Some((path.clone(), used.get())),
            _ => None,
        }).min_by_key(|(_, used)| *used);
        let Some((path, _)) = oldest else { return false; };
        if let Some(Entry::Ready { image, .. }) = self.remove(&path) { retired.push(image); }
        true
    }
    fn enqueue(&mut self, path: PathBuf, retired: &mut Vec<Arc<RenderImage>>) {
        if self.is_pending_or_known(&path) || self.queued.len() >= MAX_QUEUED { return; }
        let cost = Self::metadata_cost(&path);
        if cost > MAX_METADATA_BYTES { return; }
        while self.entries.len() >= MAX_ENTRIES || self.metadata_bytes + cost > MAX_METADATA_BYTES {
            if !self.evict_oldest(retired) { return; }
        }
        self.metadata_bytes += cost;
        self.entries.insert(path.clone(), Entry::Queued);
        self.queued.push_back(path);
    }

    /// 返回已淘汰位图，由调用方同时释放各窗口的 GPU atlas 条目。
    fn insert(&mut self, path: PathBuf, decoded: Option<Decoded>) -> Vec<Arc<RenderImage>> {
        let mut retired = Vec::new();
        if let Some(Entry::Ready { image, .. }) = self.remove(&path) {
            retired.push(image);
        }
        self.clock.set(self.clock.get() + 1);
        self.metadata_bytes += Self::metadata_cost(&path);
        self.generation = self.generation.wrapping_add(1);
        match decoded {
            Some((image, width, height, bytes)) => {
                self.failures.remove(&Self::path_key(&path));
                self.bytes += bytes;
                self.entries.insert(path, Entry::Ready { image, width, height, bytes, used: Cell::new(self.clock.get()), version: self.generation });
                while self.bytes > MAX_CACHE_BYTES {
                    if !self.evict_oldest(&mut retired) { break; }
                }
            }
            None => {
                self.remember_failure(&path,Failure::Decode);
                self.entries.insert(path, Entry::Failed { reason: Failure::Decode, used: Cell::new(self.clock.get()), version: self.generation });
            }
        }
        while self.entries.len() > MAX_ENTRIES || self.metadata_bytes > MAX_METADATA_BYTES {
            if !self.evict_oldest(&mut retired) { break; }
        }
        retired
    }
}

/// 请求解码一批图片；已在缓存或正在解码的路径会被跳过。
pub fn request(paths: Vec<PathBuf>, cx: &mut App) {
    let mut retired = Vec::new();
    for path in paths {
        cx.global_mut::<ImageStore>().enqueue(path, &mut retired);
    }
    for image in retired { cx.drop_image(image, None); }
    start_pending(cx);
}

fn start_pending(cx: &mut App) {
    loop {
        let store = cx.global_mut::<ImageStore>();
        if store.running >= MAX_DECODERS { break; }
        let Some(path) = store.queued.pop_front() else { break; };
        store.entries.insert(path.clone(), Entry::Loading);
        store.running += 1;
        let decode_path = path.clone();
        let task = cx.background_executor().spawn(async move { decode(&decode_path) });
        cx.spawn(async move |cx| {
            let decoded = task.await;
            cx.update(|cx| {
                let store = cx.global_mut::<ImageStore>();
                store.running -= 1;
                let failure = decoded.as_ref().err().copied();
                let retired = store.insert(path.clone(), decoded.ok());
                if let (Some(reason), Some(Entry::Failed { reason: stored, .. })) = (failure, store.entries.get_mut(&path)) { *stored = reason; }
                if let Some(reason) = failure { store.remember_failure(&path,reason); }
                // 此处在 App 任务中，所有窗口都在 App 内，可完整移除纹理。
                for image in retired { cx.drop_image(image, None); }
                start_pending(cx);
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

fn decode(path: &Path) -> Result<Decoded, Failure> {
    let file = std::fs::File::open(path).map_err(|_| Failure::Read)?;
    if file.metadata().map_err(|_| Failure::Read)?.len() > MAX_INPUT_BYTES { return Err(Failure::Budget); }
    let mut encoded = Vec::new();
    file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut encoded).map_err(|_| Failure::Read)?;
    if encoded.len() as u64 > MAX_INPUT_BYTES { return Err(Failure::Budget); }
    let mut reader = image::ImageReader::new(Cursor::new(encoded)).with_guessed_format().map_err(|_| Failure::Decode)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    reader.limits(limits);
    let error = |error| if matches!(error, image::ImageError::Limits(_)) { Failure::Budget } else { Failure::Decode };
    let decoder = reader.into_decoder().map_err(error)?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_INPUT_PIXELS || decoder.total_bytes() > MAX_DECODE_BYTES { return Err(Failure::Budget); }
    let image = image::DynamicImage::from_decoder(decoder).map_err(error)?;
    let scale = (MAX_DECODE_WIDTH as f64 / width.max(height) as f64).min((MAX_BITMAP_PIXELS as f64 / pixels as f64).sqrt()).min(1.0);
    let image = if scale < 1.0 { image.resize_exact((width as f64 * scale).floor().max(1.0) as u32, (height as f64 * scale).floor().max(1.0) as u32, FilterType::Triangle) } else { image };
    let mut buffer = image.into_rgba8();
    // GPUI 的图片纹理为 BGRA。
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bytes = buffer.len();
    let render = RenderImage::new(vec![Frame::new(buffer)]);
    Ok((Arc::new(render), width, height, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![Frame::new(image::RgbaImage::new(1, 1))]))
    }

    #[test]
    fn cache_eviction_returns_only_lru_texture_and_allows_reload() {
        let mut store = ImageStore::default();
        let images = [bitmap(), bitmap(), bitmap()];
        let bytes = MAX_CACHE_BYTES / 2;
        assert!(store.insert("a".into(), Some((images[0].clone(), 1, 1, bytes))).is_empty());
        assert!(store.insert("b".into(), Some((images[1].clone(), 1, 1, bytes))).is_empty());
        store.image(Path::new("a"));
        let retired = store.insert("c".into(), Some((images[2].clone(), 1, 1, bytes)));
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &images[1]));
        assert!(!store.is_pending_or_known(Path::new("b")));
        assert!(store.image(Path::new("a")).is_some());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
        let retired = store.insert("b".into(), Some((images[1].clone(), 1, 1, bytes)));
        assert_eq!(retired.len(), 1);
        assert!(store.image(Path::new("b")).is_some());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
    }

    #[test]
    fn cache_eviction_replacement_and_failure_release_old_texture_once() {
        let mut store = ImageStore::default();
        let old = bitmap();
        let new = bitmap();
        store.insert("a".into(), Some((old.clone(), 1, 1, 4)));
        let retired = store.insert("a".into(), Some((new.clone(), 1, 1, 4)));
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &old));
        assert_eq!(store.bytes, 4);
        let retired = store.insert("a".into(), None);
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &new));
        assert_eq!(store.bytes, 0);
        assert!(store.insert("a".into(), None).is_empty());
    }
}
