//! 本地图片的异步解码与有界缓存（GPUI 全局状态）。
//!
//! 解码在后台线程进行，宽度超过 `MAX_DECODE_WIDTH` 的图片先缩小再上传，限制位图内存；
//! 缓存按估算字节淘汰最久未用的条目。解码完成后递增 `generation` 并刷新窗口，
//! 语义视图把 generation 纳入含图片块的布局缓存键，从而在尺寸就绪后重排。

use gpui::{App, Global, RenderImage};
use image::{Frame, imageops::FilterType};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 解码宽度上限：2× 视网膜屏下约 1200 逻辑像素，足够正文宽度使用。
const MAX_DECODE_WIDTH: u32 = 2400;
/// 位图缓存总量上限（BGRA 字节）。
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;

enum Entry {
    Loading,
    /// `used` 用 Cell 记录访问时钟，使绘制阶段只需共享借用即可更新 LRU 顺序。
    Ready { image: Arc<RenderImage>, width: u32, height: u32, bytes: usize, used: Cell<u64> },
    Failed,
}

#[derive(Default)]
pub struct ImageStore {
    entries: HashMap<PathBuf, Entry>,
    bytes: usize,
    clock: Cell<u64>,
    pub generation: u64,
}

impl Global for ImageStore {}

impl ImageStore {
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
        self.entries.contains_key(path)
    }

    fn insert(&mut self, path: PathBuf, decoded: Option<(Arc<RenderImage>, u32, u32, usize)>) {
        self.clock.set(self.clock.get() + 1);
        match decoded {
            Some((image, width, height, bytes)) => {
                self.bytes += bytes;
                self.entries.insert(path, Entry::Ready { image, width, height, bytes, used: Cell::new(self.clock.get()) });
                while self.bytes > MAX_CACHE_BYTES {
                    let oldest = self
                        .entries
                        .iter()
                        .filter_map(|(path, entry)| match entry {
                            Entry::Ready { used, .. } => Some((path.clone(), used.get())),
                            _ => None,
                        })
                        .min_by_key(|(_, used)| *used);
                    let Some((path, _)) = oldest else { break };
                    if let Some(Entry::Ready { bytes, .. }) = self.entries.remove(&path) {
                        self.bytes -= bytes;
                    }
                }
            }
            None => {
                self.entries.insert(path, Entry::Failed);
            }
        }
        self.generation += 1;
    }
}

/// 请求解码一批图片；已在缓存或正在解码的路径会被跳过。
pub fn request(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths {
        let store = cx.global_mut::<ImageStore>();
        if store.is_pending_or_known(&path) {
            continue;
        }
        store.entries.insert(path.clone(), Entry::Loading);
        let decode_path = path.clone();
        let task = cx.background_executor().spawn(async move { decode(&decode_path) });
        cx.spawn(async move |cx| {
            let decoded = task.await;
            cx.update(|cx| {
                cx.global_mut::<ImageStore>().insert(path, decoded);
                cx.refresh_windows();
            })
            .ok();
        })
        .detach();
    }
}

fn decode(path: &Path) -> Option<(Arc<RenderImage>, u32, u32, usize)> {
    let image = image::open(path).ok()?;
    let (width, height) = (image.width(), image.height());
    let image = if width > MAX_DECODE_WIDTH {
        let scaled_height = (height as f64 * MAX_DECODE_WIDTH as f64 / width as f64).round().max(1.0) as u32;
        image.resize(MAX_DECODE_WIDTH, scaled_height, FilterType::Triangle)
    } else {
        image
    };
    let mut buffer = image.into_rgba8();
    // GPUI 的图片纹理为 BGRA。
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bytes = buffer.len();
    let render = RenderImage::new(vec![Frame::new(buffer)]);
    Some((Arc::new(render), width, height, bytes))
}
