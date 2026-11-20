//! 本地图片的异步解码与有界缓存（GPUI 全局状态）。
//!
//! 解码前检查输入预算；后台任务和排队数量有界，按最长边和像素数缩小后上传；
//! 缓存按估算字节淘汰最久未用的条目。解码完成后递增 `generation` 并刷新窗口，
//! 语义视图把 generation 纳入含图片块的布局缓存键，从而在尺寸就绪后重排。

use gpui::{App, Global, RenderImage};
use image::{Frame, ImageDecoder, imageops::FilterType};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

mod animation;
mod gif;
mod source;
pub use source::{ImageKey, ImageRequest, ImageSource};
pub(crate) use source::{image_label, is_data_source, resolve_image};

/// 解码宽度上限：2× 视网膜屏下约 1200 逻辑像素，足够正文宽度使用。
const MAX_DECODE_WIDTH: u32 = 2400;
/// 位图缓存总量上限（BGRA 字节）。
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_INPUT_PIXELS: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BITMAP_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_ANIMATION_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;
const MAX_DECODERS: usize = 2;
const MAX_QUEUED: usize = 64;
const MAX_ENTRIES: usize = 512;
const MAX_METADATA_BYTES: usize = 512 * 1024;
// 路径元数据淘汰后仍记住近期失败，避免同一屏大量坏图片随每次完成重新排队。
const MAX_FAILURES: usize = 2048;

#[derive(Clone, Copy, Debug)]
enum Failure { Budget, Read, Decode }
impl Failure {
    fn message(self) -> &'static str {
        match self { Self::Budget => "图片超过解码资源预算", Self::Read => "图片文件无法读取", Self::Decode => "图片格式不支持或内容损坏" }
    }
}
type Decoded = (Arc<RenderImage>, u32, u32, usize);
struct Loaded {
    bitmap: Decoded,
    stream: Option<gif::GifStream>,
    notice: Option<Failure>,
}
impl From<Decoded> for Loaded {
    fn from(bitmap: Decoded) -> Self { Self { bitmap, stream: None, notice: None } }
}

enum Entry {
    Queued,
    Loading(u64),
    /// `used` 用 Cell 记录访问时钟，使绘制阶段只需共享借用即可更新 LRU 顺序。
    Ready { image: Arc<RenderImage>, width: u32, height: u32, bytes: usize, used: Cell<u64>, version: u64, notice: Option<Failure> },
    Failed { reason: Failure, used: Cell<u64>, version: u64 },
}

#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<ImageKey, Entry>,
    bytes: usize,
    clock: Cell<u64>,
    metadata_bytes: usize,
    queued: VecDeque<ImageRequest>,
    // Only queued or running requests own encoded data; ready entries retain the digest and bitmap.
    pending_data_bytes: usize,
    running: usize,
    next_request: u64,
    animations: HashMap<ImageKey, animation::Playback>,
    animation_bytes: Arc<AtomicUsize>,
    animation_queue: RefCell<VecDeque<(ImageKey, u64)>>,
    animation_scheduled: Cell<bool>,
    retired_animations: Vec<Arc<RenderImage>>,
    failures: HashMap<u64, (Failure, u64)>,
    failure_order: VecDeque<(u64, u64)>,
    pub generation: u64,
}

impl Global for ImageStore {}

impl ImageStore {
    #[cfg(feature = "automation")]
    pub(crate) fn automation_state(&self) -> serde_json::Value {
        serde_json::json!({"entries":self.entries.len(),"decodedBytes":self.bytes,"metadataBytes":self.metadata_bytes,
            "animationReservedBytes":self.animation_bytes.load(Ordering::Relaxed),"animations":self.animations.len(),
            "pendingDataBytes":self.pending_data_bytes,
            "pending":self.running+self.queued.len(),"failed":self.entries.values().filter(|e|matches!(e,Entry::Failed{..})).count(),
            "scope":"process CPU cache; decodedBytes is static/poster bitmaps; animationReservedBytes includes bounded working sets and compressed sources; excludes GPU"})
    }
    fn cache_bytes(&self) -> usize { self.bytes + self.animation_bytes.load(Ordering::Relaxed) }
    pub(crate) fn pending_data_capacity(&self) -> usize {
        source::MAX_PENDING_DATA_BYTES.saturating_sub(self.pending_data_bytes)
    }
    pub(crate) fn can_request(&self, key: &ImageKey, encoded_len: usize) -> bool {
        !self.is_pending_or_known(key)
            && self.queued.len() + self.animation_queue.borrow().len() < MAX_QUEUED
            && encoded_len <= self.pending_data_capacity()
    }
    fn metadata_cost(path: &ImageKey) -> usize { (path.estimated_bytes() + 256).saturating_mul(4) }
    fn path_key(path: &ImageKey) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hash);
        hash.finish()
    }
    fn remember_failure(&mut self, path: &ImageKey, failure: Failure) {
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

    pub fn version(&self, path: &ImageKey) -> (u8, u64) {
        if Self::metadata_cost(path) > MAX_METADATA_BYTES { return (2,0); }
        match self.entries.get(path) {
            Some(Entry::Ready { version, .. }) => (1, *version),
            Some(Entry::Failed { version, .. }) => (2, *version),
            Some(_) => (0, 0),
            None => self.failures.get(&Self::path_key(path)).map_or((3,self.generation), |(_,version)| (2,*version)),
        }
    }
    pub fn failure(&self, path: &ImageKey) -> Option<&'static str> {
        if Self::metadata_cost(path) > MAX_METADATA_BYTES { return Some(Failure::Budget.message()); }
        match self.entries.get(path) { Some(Entry::Failed { reason, .. }) => Some(reason.message()), Some(_) => None,
            None => self.failures.get(&Self::path_key(path)).map(|(reason,_)| reason.message()) }
    }
    /// 已解码图片的原始像素尺寸（缩放前）；未就绪返回 None。
    pub fn size(&self, path: &ImageKey) -> Option<(u32, u32)> {
        match self.entries.get(path) {
            Some(Entry::Ready { width, height, .. }) => Some((*width, *height)),
            _ => None,
        }
    }

    pub fn image(&self, path: &ImageKey) -> Option<Arc<RenderImage>> {
        self.clock.set(self.clock.get() + 1);
        match self.entries.get(path) {
            Some(Entry::Ready { image, used, .. }) => {
                used.set(self.clock.get());
                Some(image.clone())
            }
            _ => None,
        }
    }

    /// 仅实际可见绘制调用；尺寸／复制等查询继续使用 image() 的静态首帧。
    pub fn frame_image(&self, path: &ImageKey, window: &gpui::Window, cx: &App) -> Option<Arc<RenderImage>> {
        let poster = self.image(path)?;
        let Some(Entry::Ready { version, .. }) = self.entries.get(path) else { return None; };
        Some(match self.animations.get(path) {
            Some(playback) => playback.image(self, path, *version, &poster, window, cx),
            None => poster,
        })
    }

    pub fn notice(&self, path: &ImageKey) -> Option<&'static str> {
        match self.entries.get(path) {
            Some(Entry::Ready { notice: Some(Failure::Budget), .. }) => Some("动画超出预算，已显示静态预览"),
            Some(Entry::Ready { notice: Some(_), .. }) => Some("动画后续帧无法解码，已显示静态预览"),
            _ => None,
        }
    }

    pub fn is_pending_or_known(&self, path: &ImageKey) -> bool {
        self.entries.contains_key(path) || Self::metadata_cost(path) > MAX_METADATA_BYTES || self.failures.contains_key(&Self::path_key(path))
    }

    fn remove(&mut self, path: &ImageKey) -> Option<Entry> {
        let entry = self.entries.remove(path)?;
        if let Some(playback) = self.animations.remove(path) {
            let current = playback.current();
            if !matches!(&entry, Entry::Ready { image, .. } if Arc::ptr_eq(image, &current)) { self.retired_animations.push(current); }
        }
        self.animation_queue.get_mut().retain(|(key, _)| key != path);
        self.metadata_bytes = self.metadata_bytes.saturating_sub(Self::metadata_cost(path));
        if let Entry::Ready { bytes, .. } = &entry { self.bytes -= bytes; }
        Some(entry)
    }
    /// 关闭文档时移除无主资源，同时使排队和运行中的旧请求失效。
    pub(crate) fn retain(&mut self, keep: impl Fn(&ImageKey) -> bool) -> Vec<Arc<RenderImage>> {
        let paths: Vec<_> = self.entries.keys().filter(|path| !keep(path)).cloned().collect();
        if paths.is_empty() { return Vec::new(); }
        let mut retired = Vec::new();
        for path in paths {
            if let Some(Entry::Ready { image, .. }) = self.remove(&path) { retired.push(image); }
        }
        let mut released = 0;
        self.queued.retain(|request| {
            let keep = matches!(self.entries.get(&request.key), Some(Entry::Queued));
            if !keep { released += request.encoded_len(); }
            keep
        });
        self.pending_data_bytes -= released;
        if self.entries.is_empty() { self.failures.clear(); self.failure_order.clear(); }
        self.generation = self.generation.wrapping_add(1);
        retired.append(&mut self.retired_animations);
        retired
    }
    fn complete_loaded(&mut self, path: ImageKey, request: u64, loaded: Result<Loaded, Failure>) -> Vec<Arc<RenderImage>> {
        if !matches!(self.entries.get(&path), Some(Entry::Loading(current)) if *current == request) { return Vec::new(); }
        let loaded = match loaded { Ok(loaded) => loaded, Err(error) => return self.complete(path, request, Err(error)) };
        let mut retired = self.complete(path.clone(), request, Ok(loaded.bitmap));
        if let Some(Entry::Ready { image, notice, .. }) = self.entries.get_mut(&path) {
            *notice = loaded.notice;
            if let Some(stream) = loaded.stream {
                self.animations.insert(path, animation::Playback::new(stream, image.clone(), self.animation_bytes.clone()));
            }
        }
        while self.cache_bytes() > MAX_CACHE_BYTES {
            if !self.evict_oldest(&mut retired) { break; }
        }
        retired.append(&mut self.retired_animations);
        retired
    }
    fn complete(&mut self, path: ImageKey, request: u64, decoded: Result<Decoded, Failure>) -> Vec<Arc<RenderImage>> {
        if !matches!(self.entries.get(&path), Some(Entry::Loading(current)) if *current == request) { return Vec::new(); }
        let failure = decoded.as_ref().err().copied();
        let retired = self.insert(path.clone(), decoded.ok());
        if let (Some(reason), Some(Entry::Failed { reason: stored, .. })) = (failure, self.entries.get_mut(&path)) { *stored = reason; }
        if let Some(reason) = failure { self.remember_failure(&path, reason); }
        retired
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
    fn enqueue(&mut self, request: ImageRequest, retired: &mut Vec<Arc<RenderImage>>) {
        let path = &request.key;
        if !self.can_request(path, request.encoded_len()) { return; }
        let cost = Self::metadata_cost(path);
        if cost > MAX_METADATA_BYTES { return; }
        while self.entries.len() >= MAX_ENTRIES || self.metadata_bytes + cost > MAX_METADATA_BYTES {
            if !self.evict_oldest(retired) { return; }
        }
        self.metadata_bytes += cost;
        self.entries.insert(path.clone(), Entry::Queued);
        self.pending_data_bytes += request.encoded_len();
        self.queued.push_back(request);
    }

    /// 返回已淘汰位图，由调用方同时释放各窗口的 GPU atlas 条目。
    fn insert(&mut self, path: ImageKey, decoded: Option<Decoded>) -> Vec<Arc<RenderImage>> {
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
                self.entries.insert(path, Entry::Ready { image, width, height, bytes, used: Cell::new(self.clock.get()), version: self.generation, notice: None });
                while self.cache_bytes() > MAX_CACHE_BYTES {
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
        retired.append(&mut self.retired_animations);
        retired
    }
}

/// 请求解码一批图片；已在缓存或正在解码的路径会被跳过。
pub fn request(paths: Vec<ImageRequest>, cx: &mut App) {
    let mut retired = Vec::new();
    for path in paths {
        cx.global_mut::<ImageStore>().enqueue(path, &mut retired);
    }
    retired.append(&mut cx.global_mut::<ImageStore>().retired_animations);
    for image in retired { cx.drop_image(image, None); }
    start_pending(cx);
}

fn start_pending(cx: &mut App) {
    loop {
        let store = cx.global_mut::<ImageStore>();
        if store.running >= MAX_DECODERS { break; }
        let Some(image) = store.queued.pop_front() else {
            if animation::start_pending(cx) { continue; }
            break;
        };
        let path = image.key.clone();
        let encoded_bytes = image.encoded_len();
        store.next_request = store.next_request.wrapping_add(1);
        let request = store.next_request;
        store.entries.insert(path.clone(), Entry::Loading(request));
        store.running += 1;
        let task = cx.background_executor().spawn(async move { decode(&image) });
        cx.spawn(async move |cx| {
            let decoded = task.await;
            cx.update(|cx| {
                let store = cx.global_mut::<ImageStore>();
                store.running -= 1;
                store.pending_data_bytes -= encoded_bytes;
                let retired = store.complete_loaded(path, request, decoded);
                // 此处在 App 任务中，所有窗口都在 App 内，可完整移除纹理。
                for image in retired { cx.drop_image(image, None); }
                start_pending(cx);
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

pub(crate) fn resolve_source(source: &str, base: Option<&Path>) -> Option<PathBuf> {
    if is_data_source(source) { return None; } // Embedded data has no local or remote file location.
    if let Ok(url) = url::Url::parse(source) {
        if matches!(url.scheme(), "http" | "https") { return Some(PathBuf::from(url.as_str())); }
        if url.scheme() == "file" { return url.to_file_path().ok(); }
    }
    if source.contains("://") { return None; }
    let decoded = crate::markdown::render::percent_decode(source);
    let path = Path::new(&decoded);
    if path.is_absolute() { Some(path.into()) } else { base.map(|base| base.join(path)) }
}

async fn download(source: &str) -> Result<Vec<u8>, Failure> {
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15)).redirect(reqwest::redirect::Policy::limited(3))
        .build().map_err(|_| Failure::Read)?;
    let mut response = client.get(source).send().await.map_err(|_| Failure::Read)?
        .error_for_status().map_err(|_| Failure::Read)?;
    if response.content_length().is_some_and(|length| length > MAX_INPUT_BYTES) { return Err(Failure::Budget); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Read)? {
        if bytes.len() as u64 + chunk.len() as u64 > MAX_INPUT_BYTES { return Err(Failure::Budget); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn decode(request: &ImageRequest) -> Result<Loaded, Failure> {
    let encoded = match &request.key {
        ImageKey::Location(path) => read_encoded(path)?,
        ImageKey::Embedded(_) => {
            use base64::Engine;
            let data = request.encoded.as_deref().ok_or(Failure::Decode)?;
            if data.len() > source::MAX_ENCODED_DATA_BYTES { return Err(Failure::Budget); }
            base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| Failure::Decode)?
        }
    };
    decode_encoded(encoded)
}

fn read_encoded(path: &Path) -> Result<Vec<u8>, Failure> {
    let source = path.to_string_lossy();
    let encoded = if source.starts_with("http://") || source.starts_with("https://") {
        tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| Failure::Read)?
            .block_on(download(&source))?
    } else {
        let file = std::fs::File::open(path).map_err(|_| Failure::Read)?;
        if file.metadata().map_err(|_| Failure::Read)?.len() > MAX_INPUT_BYTES { return Err(Failure::Budget); }
        let mut bytes = Vec::new();
        file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes).map_err(|_| Failure::Read)?;
        bytes
    };
    Ok(encoded)
}

fn decode_encoded(encoded: Vec<u8>) -> Result<Loaded, Failure> {
    if encoded.len() as u64 > MAX_INPUT_BYTES { return Err(Failure::Budget); }
    if encoded.starts_with(b"GIF87a") || encoded.starts_with(b"GIF89a") {
        return decode_gif(encoded);
    }
    if std::str::from_utf8(&encoded).is_ok_and(|text| text.trim_start_matches('\u{feff}').trim_start().starts_with('<')) {
        return decode_svg(&encoded).map(Loaded::from);
    }
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
    Ok((Arc::new(render), width, height, bytes).into())
}

fn decode_gif(encoded: Vec<u8>) -> Result<Loaded, Failure> {
    let gif = gif::decode(encoded)?;
    let bytes = gif.first.buffer().len();
    Ok(Loaded { bitmap: (Arc::new(RenderImage::new(vec![gif.first])), gif.width, gif.height, bytes), stream: gif.stream, notice: gif.notice })
}

fn svg_tree(encoded: &[u8]) -> Result<resvg::usvg::Tree, Failure> {
    if encoded.len() > MAX_SVG_BYTES { return Err(Failure::Budget); }
    let refused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (data_refused, path_refused) = (refused.clone(), refused.clone());
    let options = resvg::usvg::Options { fontdb: crate::embeds::font_database(),
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(move |_, _, _| { data_refused.store(true, std::sync::atomic::Ordering::Relaxed); None }),
            resolve_string: Box::new(move |_, _| { path_refused.store(true, std::sync::atomic::Ordering::Relaxed); None }),
        }, ..Default::default() };
    let tree = resvg::usvg::Tree::from_data(encoded, &options).map_err(|_| Failure::Decode)?;
    let size = tree.size();
    if refused.load(std::sync::atomic::Ordering::Relaxed) { return Err(Failure::Decode); }
    if size.width() > 32_768.0 || size.height() > 32_768.0 || size.width() * size.height() > MAX_INPUT_PIXELS as f32 { return Err(Failure::Budget); }
    Ok(tree)
}

pub(crate) fn validate_svg(encoded: &[u8]) -> Result<(), String> {
    svg_tree(encoded).map(|_| ()).map_err(|reason| reason.message().to_owned())
}

fn decode_svg(encoded: &[u8]) -> Result<Decoded, Failure> {
    let tree = svg_tree(encoded)?;
    let size = tree.size();
    let (width, height) = (size.width().ceil() as u32, size.height().ceil() as u32);
    let scale = (MAX_DECODE_WIDTH as f32 / size.width().max(size.height()))
        .min((MAX_BITMAP_PIXELS as f32 / (size.width() * size.height())).sqrt()).min(2.0);
    let (w, h) = ((size.width() * scale).floor().max(1.0) as u32, (size.height() * scale).floor().max(1.0) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or(Failure::Budget)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for pixel in data.chunks_exact_mut(4) { pixel.swap(0, 2); }
    let bytes = data.len();
    let buffer = image::RgbaImage::from_raw(w, h, data).ok_or(Failure::Decode)?;
    Ok((Arc::new(RenderImage::new(vec![Frame::new(buffer)])), width, height, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(path: &str) -> ImageKey { ImageKey::Location(path.into()) }
    fn local_request(path: &str) -> ImageRequest { ImageRequest { key: key(path), encoded: None } }

    fn bitmap() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![Frame::new(image::RgbaImage::new(1, 1))]))
    }

    #[test]
    fn closed_resources_release_textures_and_ignore_late_decode() {
        let mut store = ImageStore::default();
        store.insert(key("shared"), Some((bitmap(), 1, 1, 4)));
        store.insert(key("closed"), Some((bitmap(), 1, 1, 4)));
        store.enqueue(local_request("pending"), &mut Vec::new());
        store.entries.insert(key("pending"), Entry::Loading(1));
        store.queued.clear();
        store.enqueue(local_request("queued"), &mut Vec::new());
        assert_eq!(store.retain(|path| path == &key("shared")).len(), 1);
        assert!(store.queued.is_empty());
        assert_eq!(store.bytes, 4);
        assert_eq!(store.metadata_bytes, ImageStore::metadata_cost(&key("shared")));
        store.complete(key("pending"), 1, Ok((bitmap(), 1, 1, 4)));
        assert!(!store.is_pending_or_known(&key("pending")));
        store.enqueue(local_request("pending"), &mut Vec::new());
        store.entries.insert(key("pending"), Entry::Loading(2));
        store.complete(key("pending"), 1, Err(Failure::Read));
        assert!(matches!(store.entries.get(&key("pending")), Some(Entry::Loading(2))));
        store.complete(key("pending"), 2, Ok((bitmap(), 1, 1, 4)));
        assert_eq!(store.bytes, 8);
        assert_eq!(store.retain(|_| false).len(), 2);
        assert_eq!((store.bytes, store.metadata_bytes, store.entries.len()), (0, 0, 0));
    }

    #[test]
    fn cache_eviction_returns_only_lru_texture_and_allows_reload() {
        let mut store = ImageStore::default();
        let images = [bitmap(), bitmap(), bitmap()];
        let bytes = MAX_CACHE_BYTES / 2;
        assert!(store.insert(key("a"), Some((images[0].clone(), 1, 1, bytes))).is_empty());
        assert!(store.insert(key("b"), Some((images[1].clone(), 1, 1, bytes))).is_empty());
        store.image(&key("a"));
        let retired = store.insert(key("c"), Some((images[2].clone(), 1, 1, bytes)));
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &images[1]));
        assert!(!store.is_pending_or_known(&key("b")));
        assert!(store.image(&key("a")).is_some());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
        let retired = store.insert(key("b"), Some((images[1].clone(), 1, 1, bytes)));
        assert_eq!(retired.len(), 1);
        assert!(store.image(&key("b")).is_some());
        assert_eq!(store.bytes, MAX_CACHE_BYTES);
    }

    #[test]
    fn cache_eviction_replacement_and_failure_release_old_texture_once() {
        let mut store = ImageStore::default();
        let old = bitmap();
        let new = bitmap();
        store.insert(key("a"), Some((old.clone(), 1, 1, 4)));
        let retired = store.insert(key("a"), Some((new.clone(), 1, 1, 4)));
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &old));
        assert_eq!(store.bytes, 4);
        let retired = store.insert(key("a"), None);
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &new));
        assert_eq!(store.bytes, 0);
        assert!(store.insert(key("a"), None).is_empty());
    }

    #[test]
    fn embedded_images_decode_with_small_keys_and_release_pending_data() {
        use base64::Engine;
        use image::ImageEncoder;
        // A real PNG larger than the old per-key metadata budget.
        let (width, height) = (256, 256);
        let mut state = 123456789u32;
        let pixels: Vec<u8> = (0..width * height * 4).map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        }).collect();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8).unwrap();
        let data = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
        assert!(data.len() * 4 > MAX_METADATA_BYTES);
        let source = resolve_image(&data, None).unwrap();
        assert!(matches!(source.key, ImageKey::Embedded(_)));
        assert!(resolve_source(&data, None).is_none());
        assert_eq!(ImageStore::metadata_cost(&source.key), (32 + 256) * 4);
        let markdown = format!("<img alt=\"diagram\" src=\"{data}\" />\n\nNext paragraph.\n\n![]({data})\n");
        let parsed = crate::nomo_mark::parse::fixture(&markdown);
        assert_eq!(&*parsed.source, markdown);
        let mut found = 0;
        for (event, range) in &parsed.events {
            use pulldown_cmark::{Event, Tag};
            let key = match event {
                Event::Html(_) | Event::InlineHtml(_) => {
                    let Some((_, tag)) = crate::nomo_mark::image_properties::html_image_tag(&markdown, range.clone()) else { continue };
                    resolve_image(&tag.attrs["src"], None).unwrap().key
                }
                Event::Start(Tag::Image { dest_url, .. }) => resolve_image(dest_url, None).unwrap().key,
                Event::Text(text) => { assert!(text.len() < 100); continue; }
                _ => continue,
            };
            assert_eq!(key, source.key);
            found += 1;
        }
        assert_eq!(found, 2);
        let request = source.request();
        let loaded = decode(&request).unwrap();
        assert_eq!((loaded.bitmap.1, loaded.bitmap.2), (width, height));
        assert_eq!(loaded.bitmap.0.as_bytes(0).unwrap()[..4], [pixels[2], pixels[1], pixels[0], pixels[3]]);

        let mut store = ImageStore::default();
        store.enqueue(request.clone(), &mut Vec::new());
        store.enqueue(request, &mut Vec::new()); // same content shares one request
        assert_eq!(store.queued.len(), 1);
        assert_eq!(store.pending_data_bytes, source.encoded_len());
        assert!(store.retain(|_| false).is_empty());
        assert_eq!(store.pending_data_bytes, 0);
        assert!(store.queued.is_empty());

        store.pending_data_bytes = source::MAX_PENDING_DATA_BYTES;
        store.enqueue(source.request(), &mut Vec::new());
        assert!(store.queued.is_empty());
        store.pending_data_bytes = 0;
        assert!(store.insert(source.key.clone(), Some(loaded.bitmap)).is_empty());
        assert_eq!(store.size(&source.key), Some((width, height)));
        assert_eq!(store.retain(|_| false).len(), 1);
    }

    #[test]
    fn invalid_embedded_images_never_become_paths_or_payload_labels() {
        for data in ["data:text/plain;base64,AAAA", "DATA:image/png,AAAA", "data:image/png;base64,"] {
            assert!(resolve_image(data, None).is_err());
            assert!(resolve_source(data, Some(Path::new("."))).is_none());
        }
        let bad = resolve_image("data:image/png;base64,%%%%", None).unwrap();
        assert!(matches!(decode(&bad.request()), Err(Failure::Decode)));
        let huge = format!("data:image/png;base64,{}", "A".repeat(20_000));
        assert_eq!(image_label(&huge, ""), "内嵌图片");
        assert_eq!(image_label(&huge, "diagram"), "diagram");
        assert_eq!(image_label(&"a".repeat(1000), "").chars().count(), 121);
    }
}
